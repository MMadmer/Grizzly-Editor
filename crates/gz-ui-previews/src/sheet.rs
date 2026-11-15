//! `shot.models`: several models in one picture, each in the three-quarter view a thumbnail has,
//! with its reference and its size under it - what an agent looks at to pick the models of a
//! place and to know how large each is.

use std::sync::Arc;

use base64::Engine;
use gz_editor::{CmdError, Editor, Origin, Reply, Value, json};
use gz_pix::{Canvas, Rgba};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::model_load::ModelSource;

/// The models of one sheet at most.
pub const MAX_MODELS: usize = 16;
/// The widest sheet, px.
const MAX_WIDTH: u32 = 2048;
/// A model's picture is this many px square at least and at most.
const CELL: (u32, u32) = (128, 512);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SheetParams {
    /// The models: library references (`props\box_wood_01`), model assets of the project by
    /// their path (`models/wall.meshasset`) or visuals, at most 16.
    refs: Vec<String>,
    /// Where the names are looked up: `library` (the project's and the editor's objects; the
    /// default), `project`, or `game` (the linked game's visuals).
    #[serde(default)]
    source: Option<String>,
    /// The sheet's width in px (1024, at most 2048).
    #[serde(default)]
    px: Option<u32>,
}

/// How a sheet of `count` models `width` px wide is laid out: columns, rows, a cell's side.
#[must_use]
pub fn layout(count: usize, width: u32) -> (u32, u32, u32) {
    let count = count.clamp(1, MAX_MODELS) as u32;
    let columns = (1..=4).find(|c| c * c >= count).unwrap_or(4);
    let rows = count.div_ceil(columns);
    let cell = (width / columns).clamp(CELL.0, CELL.1);
    (columns, rows, cell)
}

/// A name cut in the middle to fit `chars` characters.
#[must_use]
pub fn fitted(name: &str, chars: usize) -> String {
    let all: Vec<char> = name.chars().collect();
    if all.len() <= chars {
        return name.to_owned();
    }
    if chars < 5 {
        return all[..chars].iter().collect();
    }
    let keep = chars - 3;
    let head = keep / 2;
    let tail = keep - head;
    let mut out: String = all[..head].iter().collect();
    out.push_str("...");
    out.extend(&all[all.len() - tail..]);
    out
}

/// A size in metres as an author reads it: `4.00 x 3.00 x 0.30 m`.
#[must_use]
pub fn size_text(size: [f32; 3]) -> String {
    format!("{:.2} x {:.2} x {:.2} m", size[0], size[1], size[2])
}

/// The height of the two caption lines under a cell, px.
const fn caption_height(scale: u32) -> u32 {
    2 * gz_pix::LINE * scale + 6
}

/// One cell of a sheet: the model's picture (or why there is none) and what is written under it.
pub struct Cell {
    pub name: String,
    pub picture: Result<Canvas, String>,
    pub size: Option<[f32; 3]>,
}

/// Draws the sheet. Answers it with each cell's rectangle `[x0, y0, x1, y1]` (the picture and
/// its caption).
#[must_use]
pub fn compose(cells: &[Cell], width: u32, background: Rgba, ink: Rgba) -> (Canvas, Vec<[u32; 4]>) {
    let (columns, rows, cell) = layout(cells.len(), width);
    let scale = if cell >= 384 { 2 } else { 1 };
    let caption = caption_height(scale);
    let mut sheet = Canvas::new(columns * cell, rows * (cell + caption), background);
    let chars = ((cell - 8) / (gz_pix::ADVANCE * scale)) as usize;
    let faint = ink.mix(background, 0.35);
    let mut rects = Vec::with_capacity(cells.len());
    for (i, c) in cells.iter().enumerate().take(MAX_MODELS) {
        let (col, row) = (i as u32 % columns, i as u32 / columns);
        let (x0, y0) = (col * cell, row * (cell + caption));
        match &c.picture {
            Ok(picture) => sheet.blit(picture, i64::from(x0), i64::from(y0)),
            Err(why) => {
                let text = fitted(why, chars);
                let (w, h) = gz_pix::text_size(&text, scale);
                let x = i64::from(x0) + (i64::from(cell) - i64::from(w)) / 2;
                let y = i64::from(y0) + (i64::from(cell) - i64::from(h)) / 2;
                sheet.text(x, y, &text, scale, faint);
            }
        }
        let ty = i64::from(y0 + cell + 3);
        sheet.text(i64::from(x0) + 4, ty, &fitted(&c.name, chars), scale, ink);
        if let Some(size) = c.size {
            let line = i64::from(gz_pix::LINE * scale);
            sheet.text(i64::from(x0) + 4, ty + line, &fitted(&size_text(size), chars), scale, faint);
        }
        // a hairline between cells
        sheet.rect(
            i64::from(x0),
            i64::from(y0),
            i64::from(x0 + cell),
            i64::from(y0 + cell + caption),
            ink.mix(background, 0.8),
        );
        rects.push([x0, y0, x0 + cell, y0 + cell + caption]);
    }
    (sheet, rects)
}

/// A sheet as it leaves the pool: the PNG, its width and height, every cell's rectangle.
type Drawn = (Vec<u8>, u32, u32, Vec<[u32; 4]>);

pub(crate) async fn shot_models(editor: Arc<Editor>, p: SheetParams, _origin: Origin) -> Reply {
    if p.refs.is_empty() || p.refs.len() > MAX_MODELS {
        return Err(CmdError::invalid(format!("refs takes 1 to {MAX_MODELS} models")));
    }
    let source = match p.source.as_deref() {
        None => ModelSource::Library,
        Some(word) => ModelSource::parse(word)
            .ok_or_else(|| CmdError::invalid("source takes library, project or game"))?,
    };
    let width = p.px.unwrap_or(1024);
    if !(128..=MAX_WIDTH).contains(&width) {
        return Err(CmdError::invalid(format!("px takes 128 to {MAX_WIDTH}")));
    }
    let (_, _, side) = layout(p.refs.len(), width);
    let mut cells = Vec::with_capacity(p.refs.len());
    for name in &p.refs {
        let engine = crate::paths::engine_name(name);
        let cell = match crate::cmd::render_model_image(&editor, &engine, source, side).await {
            Ok((img, bounds)) => {
                let size = bounds.max - bounds.min;
                Cell {
                    name: engine,
                    picture: Canvas::from_rgba(img.width, img.height, img.rgba)
                        .ok_or_else(|| "the picture's pixels do not match its size".to_owned()),
                    size: Some([size.x, size.y, size.z]),
                }
            }
            Err(why) => Cell { name: engine, picture: Err(why), size: None },
        };
        cells.push(cell);
    }
    let palette = gz_app::EditorSettings::of(&editor).theme().palette();
    let background = Rgba::rgb(palette.canvas.r(), palette.canvas.g(), palette.canvas.b());
    let ink = if background.luma() < 0.5 { Rgba::rgb(235, 235, 235) } else { Rgba::rgb(20, 20, 20) };
    let facts: Vec<(String, Option<[f32; 3]>, Option<String>)> =
        cells.iter().map(|c| (c.name.clone(), c.size, c.picture.as_ref().err().cloned())).collect();
    let (png, w, h, rects) = editor
        .exec
        .cpu(move || -> Result<Drawn, CmdError> {
            let (sheet, rects) = compose(&cells, width, background, ink);
            let png = sheet
                .png()
                .map_err(|e| CmdError::internal(format!("the sheet could not be encoded: {e}")))?;
            Ok((png, sheet.width(), sheet.height(), rects))
        })
        .await?;
    let encoded = editor.exec.cpu(move || base64::engine::general_purpose::STANDARD.encode(png)).await;
    let models: Vec<Value> = facts
        .iter()
        .zip(&rects)
        .map(|((name, size, error), rect)| {
            let mut m = json!({ "ref": crate::paths::json_path_str(name), "cell": rect });
            if let Value::Object(o) = &mut m {
                if let Some(s) = size {
                    let r = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
                    o.insert("size".into(), json!([r(s[0]), r(s[1]), r(s[2])]));
                }
                if let Some(e) = error {
                    o.insert("error".into(), json!(e));
                }
            }
            m
        })
        .collect();
    Ok(json!({ "png_base64": encoded, "width": w, "height": h, "models": models }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_is_as_square_as_its_count_allows() {
        assert_eq!(layout(1, 1024), (1, 1, 512));
        assert_eq!(layout(2, 1024), (2, 1, 512));
        assert_eq!(layout(4, 1024), (2, 2, 512));
        assert_eq!(layout(5, 1024), (3, 2, 341));
        assert_eq!(layout(9, 1024), (3, 3, 341));
        assert_eq!(layout(10, 1024), (4, 3, 256));
        assert_eq!(layout(16, 1024), (4, 4, 256));
        // more than a sheet holds is a full sheet; a narrow one keeps its cells readable
        assert_eq!(layout(40, 1024), (4, 4, 256));
        assert_eq!(layout(16, 128), (4, 4, 128));
        assert_eq!(layout(0, 1024), (1, 1, 512));
    }

    #[test]
    fn a_long_name_is_cut_in_the_middle() {
        assert_eq!(fitted("props\\box", 20), "props\\box");
        assert_eq!(fitted("abcdefghijklmnop", 10), "abc...mnop");
        assert_eq!(fitted("abcdefghijklmnop", 10).chars().count(), 10);
        assert_eq!(fitted("abcdef", 4), "abcd");
        assert_eq!(size_text([4.0, 3.0, 0.3]), "4.00 x 3.00 x 0.30 m");
    }

    #[test]
    fn every_cell_has_its_picture_and_its_words() {
        let red = Rgba::rgb(200, 0, 0);
        let cells = vec![
            Cell {
                name: "props\\a".into(),
                picture: Ok(Canvas::new(256, 256, red)),
                size: Some([1.0, 2.0, 3.0]),
            },
            Cell { name: "props\\missing".into(), picture: Err("not found".into()), size: None },
            Cell {
                name: "props\\c".into(),
                picture: Ok(Canvas::new(256, 256, red)),
                size: Some([0.5, 0.5, 0.5]),
            },
        ];
        let (sheet, rects) = compose(&cells, 512, Rgba::BLACK, Rgba::WHITE);
        let caption = caption_height(1);
        assert_eq!((sheet.width(), sheet.height()), (512, 2 * (256 + caption)));
        assert_eq!(
            rects,
            vec![
                [0, 0, 256, 256 + caption],
                [256, 0, 512, 256 + caption],
                [0, 256 + caption, 256, 2 * (256 + caption)]
            ]
        );
        // the first picture is where its cell is, the missing one shows its reason, not a picture
        assert_eq!(sheet.pixel(100, 100), Some(red));
        assert_ne!(sheet.pixel(300, 20), Some(red));
        assert_eq!(sheet.pixel(100, 256 + i64::from(caption) + 100), Some(red));
        // words under the first cell
        let under = (0..256)
            .any(|x| (258..258 + i64::from(caption) - 2).any(|y| sheet.pixel(x, y) == Some(Rgba::WHITE)));
        assert!(under);
    }
}
