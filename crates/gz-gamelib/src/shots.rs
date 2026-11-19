//! Pictures without a window (`docs/GAME_LIBRARY.md`, 8): a model, a level or a texture seen
//! from the sides that are asked for, a sheet of small pictures to choose from, and the open
//! scene from any camera - drawn on the processor, so a commandlet and a run without a graphics
//! card answer them, and the same scene gives the same bytes.
//!
//! A face wears the mean colour of its texture (the texture catalog states it) and is lit from
//! one side: what a picture is for here is shape, size and place, read back into metres by what
//! the answer says of its camera.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use gz_base::SmolStr;
use gz_editor::{CmdError, Editor, JsonSchema, Origin, Registry, Reply, Spec};
use gz_fmt_object::Mat4;
use gz_pix::raster::{Camera, Lens, Paint, Raster, View, direction};
use gz_pix::{Canvas, Rgba};
use gz_scene::class::ObjClass;
use gz_scene::library::LibraryObject;
use gz_scene::math::{Aabb, Vec3};
use gz_scene::model::{ObjData, Object};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::index::Index;
use crate::service::GameLibrary;

/// The longest side of a picture, pixels.
pub const MAX_SIDE: u32 = 2048;
/// The most models one sheet of small pictures shows.
pub const MAX_THUMBS: usize = 64;
/// The most names one picture of the scene writes.
pub const MAX_LABELS: usize = 120;

const SKY: Rgba = Rgba(38, 42, 48, 255);
const INK: Rgba = Rgba(235, 238, 240, 255);
const PLATE: Rgba = Rgba(20, 22, 26, 220);

/// The colour every texture is on the whole.
pub struct Palette {
    colours: FxHashMap<String, Rgba>,
    /// The textures that are mostly holes (leaves, nets, glass): a face that wears one is a card
    /// around what it shows, and painted whole it would hide what stands behind it.
    clear: rustc_hash::FxHashSet<String>,
}

impl Palette {
    /// From the texture catalog of the library.
    #[must_use]
    pub fn of(index: &Index) -> Self {
        let mut colours = FxHashMap::default();
        let mut clear = rustc_hash::FxHashSet::default();
        for row in index.rows("texture") {
            let (Some(name), Some(hex)) =
                (row.get("name").and_then(Value::as_str), row.get("colour").and_then(Value::as_str))
            else {
                continue;
            };
            if row.get("opaque").and_then(Value::as_f64).is_some_and(|o| o < 0.6) {
                clear.insert(name.to_lowercase().replace('/', "\\"));
            }
            let h = hex.trim_start_matches('#');
            let part = |at: usize| h.get(at..at + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
            if let (Some(r), Some(g), Some(b)) = (part(0), part(2), part(4)) {
                // the game's textures are dark and are lit by the level: lifted, so that shapes read
                let lift = |v: u8| ((f32::from(v) / 255.0).powf(0.6) * 255.0).round() as u8;
                colours.insert(name.to_lowercase().replace('/', "\\"), Rgba(lift(r), lift(g), lift(b), 255));
            }
        }
        Self { colours, clear }
    }

    /// Whether a texture is mostly holes.
    #[must_use]
    pub fn is_clear(&self, texture: &str) -> bool {
        self.clear.contains(&texture.to_lowercase().replace('/', "\\"))
    }

    /// The colour of a texture; one nobody describes is a grey its name picks.
    #[must_use]
    pub fn colour(&self, texture: &str) -> Rgba {
        let key = texture.to_lowercase().replace('/', "\\");
        self.colours.get(&key).copied().unwrap_or_else(|| {
            let h = key.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
            let v = 110 + (h % 60) as u8;
            Rgba(v, v, v, 255)
        })
    }
}

/// Draws a model standing at `world`: every face in its texture's colour. With `cut`, nothing
/// that lies wholly above that height: a storey seen from above without what is over it.
pub fn draw(
    r: &mut Raster,
    cam: &Camera,
    model: &LibraryObject,
    world: &Mat4,
    palette: &Palette,
    cut: Option<f32>,
) -> usize {
    let surfaces = &model.object.surfaces;
    let colours: Vec<Rgba> = surfaces.iter().map(|s| palette.colour(&s.texture)).collect();
    let holes: Vec<bool> = surfaces.iter().map(|s| palette.is_clear(&s.texture)).collect();
    let plain = Rgba(150, 150, 150, 255);
    let mut drawn = 0;
    for (i, t) in model.tris.iter().enumerate() {
        let at = |k: usize| model.verts.get(t[k] as usize).map(|v| world.transform_tiny(*v));
        let (Some(a), Some(b), Some(c)) = (at(0), at(1), at(2)) else { continue };
        if cut.is_some_and(|h| a.y > h && b.y > h && c.y > h) {
            continue;
        }
        let surface = model.tri_surface.get(i).map(|s| *s as usize);
        if surface.and_then(|s| holes.get(s)).copied().unwrap_or(false) {
            continue;
        }
        let colour = surface.and_then(|s| colours.get(s)).copied().unwrap_or(plain);
        let corners = [(a, glam::Vec2::ZERO), (b, glam::Vec2::ZERO), (c, glam::Vec2::ZERO)];
        r.triangles(cam, &corners, &Paint::flat(colour));
        drawn += 1;
    }
    drawn
}

fn png_reply(canvas: &Canvas, mut extra: serde_json::Map<String, Value>) -> Reply {
    let png =
        canvas.png().map_err(|e| CmdError::internal(format!("the picture could not be encoded: {e}")))?;
    extra.insert("width".into(), json!(canvas.width()));
    extra.insert("height".into(), json!(canvas.height()));
    extra.insert("png_base64".into(), json!(base64::engine::general_purpose::STANDARD.encode(png)));
    Ok(Value::Object(extra))
}

fn v3(v: Vec3) -> Value {
    let r = |x: f32| (f64::from(x) * 1000.0).round() / 1000.0;
    json!([r(v.x), r(v.y), r(v.z)])
}

fn side(asked: Option<u32>, default: u32) -> Result<u32, CmdError> {
    let s = asked.unwrap_or(default);
    if !(32..=MAX_SIDE).contains(&s) {
        return Err(CmdError::invalid(format!("a picture is 32 to {MAX_SIDE} pixels a side, not {s}")));
    }
    Ok(s)
}

/// The colours of the library and the game's files, for whoever draws.
async fn palette(ed: &Arc<Editor>) -> Result<Arc<Palette>, CmdError> {
    let index = Index::of(ed).await?;
    Ok(ed.exec.cpu(move || Arc::new(Palette::of(&index))).await)
}

/// A model by the name a scene would use: the open project's library knows every reference;
/// without a project, the game's compiled models (`...\.ogf`).
async fn model(ed: &Arc<Editor>, reference: &str) -> Result<Arc<LibraryObject>, CmdError> {
    let name = reference.trim().replace('/', "\\");
    if let Ok(svc) = gz_scene::SceneService::of(ed) {
        let (lib, key) = (svc.library.clone(), SmolStr::new(&name));
        let found = ed
            .exec
            .io(move || {
                // what the scene holds already is taken from it; anything else is read into a
                // library of the look's own - looking at models leaves the scene's as it was
                lib.cached(&key).or_else(|| {
                    let own = lib.fork();
                    own.prefetch(std::slice::from_ref(&key));
                    own.cached(&key)
                })
            })
            .await;
        if let Some(lo) = found {
            return Ok(lo);
        }
    }
    let low = name.to_lowercase();
    if let Some(stem) = low.strip_suffix(".ogf") {
        let (e, stem, key) = (ed.clone(), stem.to_owned(), name.clone());
        let made = ed
            .exec
            .io(move || {
                let bytes = crate::provider::model_bytes(&e, None, &stem)?;
                let src = gz_fmt_mesh::read_ogf_bytes(&bytes, &key).ok()?;
                Some(Arc::new(LibraryObject::provided(&key, src.object)))
            })
            .await;
        if let Some(lo) = made {
            return Ok(lo);
        }
    }
    Err(CmdError::not_found(format!(
        "there is no model '{reference}': a compiled model of the game is `<name>.ogf` (`asset.find kind=model`), a library object or a model asset its reference"
    )))
}

/// One model seen along a view, on a tile of `size` pixels; answers the tile and how many
/// pixels a metre is.
fn tile(model: &LibraryObject, palette: &Palette, forward: Vec3, up: Vec3, size: u32) -> (Canvas, f32) {
    let b = &model.bounds;
    let cam = Camera::frame(b.min, b.max, forward, up, size, size, size / 12 + 4);
    let mut r = Raster::new(size, size, SKY);
    draw(&mut r, &cam, model, &Mat4::IDENTITY, palette, None);
    r.outline(Rgba(12, 14, 16, 255));
    (r.into_canvas(), cam.px_per_m().unwrap_or(1.0))
}

/// A bar of a round length under a tile: what a metre is there.
fn scale_bar(c: &mut Canvas, x: i64, y: i64, px_per_m: f32) {
    let mut metres = 0.01_f32;
    for step in [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0] {
        metres = step;
        if step * px_per_m >= 48.0 {
            break;
        }
    }
    let len = (metres * px_per_m).round() as i64;
    c.fill_rect(x, y, x + len, y + 3, INK);
    let text = if metres < 1.0 { format!("{} cm", (metres * 100.0).round()) } else { format!("{metres} m") };
    c.text(x + len + 6, y - 3, &text, 1, INK);
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PreviewParams {
    /// What to look at, by the name the game or a scene uses: a model (`dynamics\box\box_wood_01.ogf`,
    /// a library object, a model asset), a level of the game, a texture, a spawn section (the
    /// model it wears).
    #[serde(rename = "ref")]
    reference: String,
    /// The kind when a name is one of several: model, level, texture or section. Left out: a
    /// model.
    #[serde(default)]
    kind: Option<String>,
    /// The sides to show, each a tile of the sheet: iso, front, back, left, right, top, bottom
    /// (the side that is seen). Left out: iso, front, right, top.
    #[serde(default)]
    views: Option<Vec<String>>,
    /// One more side by its angles instead of a word: degrees round the vertical from +Z towards
    /// +X, and degrees down.
    #[serde(default)]
    heading: Option<f32>,
    /// With heading: degrees the view looks down (30).
    #[serde(default)]
    pitch: Option<f32>,
    /// The side of a tile, pixels (384; 32 to 2048).
    #[serde(default)]
    size: Option<u32>,
}

async fn asset_preview(ed: Arc<Editor>, p: PreviewParams, _o: Origin) -> Reply {
    let size = side(p.size, 384)?;
    let kind = p.kind.as_deref().unwrap_or("model").trim().to_ascii_lowercase();
    let mut views: Vec<(String, Vec3, Vec3)> = Vec::new();
    for word in
        p.views.clone().unwrap_or_else(|| ["iso", "front", "right", "top"].map(str::to_owned).to_vec())
    {
        let v = View::parse(&word).ok_or_else(|| {
            CmdError::invalid(format!("'{word}' is no view: iso, front, back, left, right, top or bottom"))
        })?;
        let (forward, up) = v.axes();
        views.push((v.name().to_owned(), forward, up));
    }
    if let Some(h) = p.heading {
        let pitch = p.pitch.unwrap_or(30.0);
        if !h.is_finite() || !pitch.is_finite() {
            return Err(CmdError::invalid("heading and pitch take degrees"));
        }
        if p.views.is_none() {
            views.clear();
        }
        views.push((format!("heading {h} pitch {pitch}"), direction(h, pitch), Vec3::Y));
    }
    if views.is_empty() || views.len() > 8 {
        return Err(CmdError::invalid("a preview shows 1 to 8 views"));
    }
    if kind == "texture" {
        return texture_preview(&ed, &p.reference, size).await;
    }
    let lo = match kind.as_str() {
        "model" => model(&ed, &p.reference).await?,
        "section" => {
            let index = Index::of(&ed).await?;
            let visual = index
                .row("section", &p.reference)
                .and_then(|r| r.get("visual").and_then(Value::as_str).map(str::to_owned))
                .ok_or_else(|| {
                    CmdError::not_found(format!(
                        "the section '{}' is none of the game's or wears no model (`asset.find kind=section`)",
                        p.reference
                    ))
                })?;
            model(&ed, &visual).await?
        }
        "level" => {
            let (e, level) = (ed.clone(), p.reference.trim().to_lowercase());
            ed.exec.io(move || crate::levels::geometry(&e, &level).map(Arc::new)).await.ok_or_else(|| {
                CmdError::not_found(format!(
                    "the game has no level '{}' (`asset.find kind=level`)",
                    p.reference
                ))
            })?
        }
        other => {
            return Err(CmdError::invalid(format!(
                "a preview shows a model, level, texture or section, not '{other}'"
            )));
        }
    };
    let palette = palette(&ed).await?;
    let name = p.reference.clone();
    let (canvas, shown) = ed
        .exec
        .cpu(move || {
            let columns = if views.len() <= 3 { views.len() as u32 } else { views.len().div_ceil(2) as u32 };
            let rows = (views.len() as u32).div_ceil(columns);
            let foot = 22;
            let mut sheet = Canvas::new(columns * size, rows * size + foot, SKY);
            let mut shown = Vec::new();
            for (i, (word, forward, up)) in views.iter().enumerate() {
                let (t, px_per_m) = tile(&lo, &palette, *forward, *up, size);
                let (x, y) = ((i as u32 % columns) * size, (i as u32 / columns) * size);
                sheet.blit(&t, i64::from(x), i64::from(y));
                sheet.label(i64::from(x) + 4, i64::from(y) + 4, word, 1, 3, INK, PLATE);
                scale_bar(&mut sheet, i64::from(x) + 8, i64::from(y + size) - 12, px_per_m);
                shown.push(json!({
                    "view": word,
                    "rect": [x, y, x + size, y + size],
                    "px_per_m": (f64::from(px_per_m) * 100.0).round() / 100.0,
                    "looks": v3(*forward),
                }));
            }
            let b = &lo.bounds;
            let s = b.max - b.min;
            let line = format!("{name}   {:.2} x {:.2} x {:.2} m (x, y, z)   {} triangles", s.x, s.y, s.z, lo.tris.len());
            sheet.text(6, i64::from(rows * size) + 7, &line, 1, INK);
            let facts = json!({ "size": v3(s), "box": { "min": v3(b.min), "max": v3(b.max) }, "triangles": lo.tris.len() });
            (sheet, json!({ "views": shown, "model": facts }))
        })
        .await;
    let mut extra = serde_json::Map::new();
    extra.insert("ref".into(), json!(p.reference));
    extra.insert("kind".into(), json!(kind));
    if let Value::Object(m) = shown {
        extra.extend(m);
    }
    png_reply(&canvas, extra)
}

/// A texture as it is, over a chequered ground that shows where it is clear.
async fn texture_preview(ed: &Arc<Editor>, name: &str, size: u32) -> Reply {
    let (_, files) = GameLibrary::files(ed).await?;
    let rel = format!("textures/{}.dds", name.trim().trim_end_matches(".dds").replace('\\', "/"));
    let shown = name.to_owned();
    let made = ed
        .exec
        .io(move || -> Result<(Canvas, Value), String> {
            let bytes = files
                .read(&rel)
                .map_err(|_| format!("the game has no texture '{shown}' (`asset.find kind=texture`)"))?;
            let info = gz_fmt_tex::read_dds_info(&bytes).map_err(|e| e.to_string())?;
            // the first level that fits the picture asked for
            let mut mip = 0;
            while mip + 1 < info.mips && (info.width.max(info.height) >> mip) > size {
                mip += 1;
            }
            let mut image = gz_fmt_tex::read_dds_level(&bytes, 0, mip).map_err(|e| e.to_string())?;
            // a texture without a level that small is made smaller here: the answer is never
            // larger than what was asked for
            let longest = image.width.max(image.height);
            if longest > size {
                let scaled =
                    |side: u32| (u64::from(side) * u64::from(size) / u64::from(longest)).max(1) as u32;
                image = gz_fmt_tex::resize(
                    &image,
                    scaled(image.width),
                    scaled(image.height),
                    gz_fmt_tex::MipFilter::Box,
                )
                .map_err(|e| e.to_string())?;
            }
            let picture = Canvas::from_rgba(image.width, image.height, image.rgba)
                .ok_or_else(|| "the texture's level has no pixels".to_owned())?;
            let mut sheet = Canvas::new(image.width, image.height, Rgba(96, 96, 96, 255));
            for y in (0..image.height).step_by(8) {
                for x in (0..image.width).step_by(8) {
                    if (x / 8 + y / 8) % 2 == 0 {
                        sheet.fill_rect(
                            i64::from(x),
                            i64::from(y),
                            i64::from(x + 8),
                            i64::from(y + 8),
                            Rgba(128, 128, 128, 255),
                        );
                    }
                }
            }
            sheet.blit(&picture, 0, 0);
            Ok((sheet, json!({ "size": [info.width, info.height], "level": mip, "mips": info.mips })))
        })
        .await
        .map_err(CmdError::not_found)?;
    let mut extra = serde_json::Map::new();
    extra.insert("ref".into(), json!(name));
    extra.insert("kind".into(), json!("texture"));
    extra.insert("texture".into(), made.1);
    png_reply(&made.0, extra)
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ThumbParams {
    /// The models to show, by the name a scene uses (`<name>.ogf`, a library object, a model
    /// asset): at most 64. One sheet holds them all, each with its name and its size.
    refs: Vec<String>,
    /// The side of one small picture, pixels (128; 32 to 512).
    #[serde(default)]
    size: Option<u32>,
}

/// The file a model's small picture is kept in, by the model's name and the picture's side.
fn thumb_file(ed: &Editor, reference: &str, size: u32) -> Option<PathBuf> {
    let layout = GameLibrary::layout(ed)?;
    Some(layout.thumb("model", &format!("{}@{size}", reference.to_lowercase())))
}

/// How a small picture is drawn: raised with the drawing, so one kept from before is made again.
const THUMB_DRAWN: u32 = 2;

/// The most small pictures a kind keeps. A picture is kept by its model's name and its side, and
/// nothing ever asks for one again once its model is gone or drawn another way: past the cap the
/// ones made longest ago go.
const THUMBS_KEPT: usize = 4096;

/// Cuts a folder of small pictures to `keep`: the pictures made longest ago go with their stamps,
/// down to three quarters of the cap so that the next ones written do not prune again at once.
/// Blocking; answers how many pictures went.
fn prune_thumbs(dir: &std::path::Path, keep: usize) -> usize {
    let Ok(read) = std::fs::read_dir(dir) else { return 0 };
    let mut pictures: Vec<(std::time::SystemTime, PathBuf)> = read
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if pictures.len() <= keep {
        return 0;
    }
    pictures.sort();
    let go = pictures.len() - keep * 3 / 4;
    for (_, picture) in pictures.iter().take(go) {
        let _ = std::fs::remove_file(picture);
        let _ = std::fs::remove_file(picture.with_extension("stamp"));
    }
    go
}

async fn asset_thumbnail(ed: Arc<Editor>, p: ThumbParams, _o: Origin) -> Reply {
    let size = side(p.size, 128)?.min(512);
    if p.refs.is_empty() || p.refs.len() > MAX_THUMBS {
        return Err(CmdError::invalid(format!(
            "a sheet shows 1 to {MAX_THUMBS} models, not {}",
            p.refs.len()
        )));
    }
    let palette = palette(&ed).await?;
    let mut tiles: Vec<(String, Option<Canvas>, Value)> = Vec::new();
    let mut wrote: Option<PathBuf> = None;
    for reference in &p.refs {
        let Ok(lo) = model(&ed, reference).await else {
            tiles.push((
                reference.clone(),
                None,
                json!({ "ref": reference, "error": "there is no such model" }),
            ));
            continue;
        };
        let file = thumb_file(&ed, reference, size);
        // a picture made before is the same picture while the model is the same
        let stamp = lo.stamp;
        let (pal, lo2, kept) = (palette.clone(), lo.clone(), file.clone());
        let (canvas, made) = ed
            .exec
            .cpu(move || {
                let mark = kept.as_ref().map(|f| f.with_extension("stamp"));
                // the model it was made of and the way it was drawn
                let made_of = format!("{THUMB_DRAWN}:{stamp}");
                let fresh = stamp != 0
                    && mark.as_ref().and_then(|m| std::fs::read_to_string(m).ok()).as_deref()
                        == Some(made_of.as_str());
                if fresh
                    && let Some(c) =
                        kept.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| Canvas::from_png(&b))
                {
                    return (c, false);
                }
                let (forward, up) = View::Iso.axes();
                let (c, _) = tile(&lo2, &pal, forward, up, size);
                let mut written = false;
                if let (Some(f), Some(m), Ok(png)) = (&kept, &mark, c.png()) {
                    written = crate::layout::write_atomic(f, &png).is_ok();
                    let _ = crate::layout::write_atomic(m, made_of.as_bytes());
                }
                (c, written)
            })
            .await;
        if made {
            wrote = file.as_ref().and_then(|f| f.parent().map(PathBuf::from));
        }
        let s = lo.bounds.max - lo.bounds.min;
        tiles.push((
            reference.clone(),
            Some(canvas),
            json!({ "ref": reference, "size": v3(s), "file": file.map(|f| f.display().to_string()) }),
        ));
    }
    // the folder was added to: it is kept within its cap
    if let Some(dir) = wrote {
        let gone = ed.exec.io(move || prune_thumbs(&dir, THUMBS_KEPT)).await;
        if gone > 0 {
            ed.log.info(format!("[game library] {gone} small pictures made longest ago were removed"));
        }
    }
    let columns = (tiles.len() as u32).min((1280 / size).max(1));
    let rows = (tiles.len() as u32).div_ceil(columns);
    let cell_h = size + 24;
    let mut sheet = Canvas::new(columns * size, rows * cell_h, SKY);
    let mut items = Vec::new();
    for (i, (reference, canvas, mut info)) in tiles.into_iter().enumerate() {
        let (x, y) = ((i as u32 % columns) * size, (i as u32 / columns) * cell_h);
        if let Some(c) = &canvas {
            sheet.blit(c, i64::from(x), i64::from(y));
        }
        // the leaf of the name, and the size under it
        let leaf =
            reference.rsplit(['\\', '/']).next().unwrap_or(reference.as_str()).trim_end_matches(".ogf");
        let fits = (size / 6).max(4) as usize;
        let short: String = leaf.chars().take(fits).collect();
        sheet.text(i64::from(x) + 2, i64::from(y + size) + 2, &short, 1, INK);
        if let Some(s) = info.get("size").and_then(Value::as_array) {
            let n = |k: usize| s.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            sheet.text(
                i64::from(x) + 2,
                i64::from(y + size) + 12,
                &format!("{:.1}x{:.1}x{:.1}", n(0), n(1), n(2)),
                1,
                Rgba(170, 176, 182, 255),
            );
        }
        if let Some(m) = info.as_object_mut() {
            m.insert("rect".into(), json!([x, y, x + size, y + size]));
        }
        items.push(info);
    }
    let mut extra = serde_json::Map::new();
    extra.insert("items".into(), json!(items));
    png_reply(&sheet, extra)
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SceneShotParams {
    /// Where the camera stands, metres: with target, a picture as an eye sees it.
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// What the camera looks at, metres.
    #[serde(default)]
    target: Option<[f32; 3]>,
    /// With position and target: the vertical field of view, degrees (70).
    #[serde(default)]
    fov: Option<f32>,
    /// Instead of position and target, a flat view to scale: top, bottom, front, back, left,
    /// right or iso, framing `objects`, `area`, or what the scene holds of its own.
    #[serde(default)]
    view: Option<String>,
    /// With view: the objects the picture frames, by name.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// With view: the area the picture frames, `[x0, z0, x1, z1]` in metres.
    #[serde(default)]
    area: Option<[f32; 4]>,
    /// Nothing wholly above this height is drawn, metres: a storey of a building seen from above
    /// without its ceiling and what is over it.
    #[serde(default)]
    cut: Option<f32>,
    /// The picture's width, pixels (1280; 32 to 2048).
    #[serde(default)]
    width: Option<u32>,
    /// The picture's height, pixels (720; 32 to 2048).
    #[serde(default)]
    height: Option<u32>,
    /// Write the names of what the scene holds of its own beside it (true). The answer's
    /// `legend` says where each stands in the picture either way.
    #[serde(default)]
    labels: Option<bool>,
}

/// What stands in a picture of the scene and is no model: a mark of its kind's colour.
fn mark_of(o: &Object) -> Option<(Rgba, f32)> {
    Some(match &o.data {
        ObjData::Light(_) => (Rgba(255, 220, 60, 255), 5.0),
        ObjData::SoundSrc(_) => (Rgba(80, 160, 255, 255), 4.0),
        ObjData::Particles(_) => (Rgba(230, 90, 220, 255), 4.0),
        ObjData::Glow(_) => (Rgba(255, 255, 255, 255), 3.0),
        ObjData::Spawn(_) if gz_scene::objects::visual_ref(o).is_none() => (Rgba(90, 220, 110, 255), 4.0),
        _ => return None,
    })
}

async fn shot_scene(ed: Arc<Editor>, p: SceneShotParams, _o: Origin) -> Reply {
    let (width, height) = (side(p.width, 1280)?, side(p.height, 720)?);
    let svc = gz_scene::need_scene(&ed).await?;
    let state = svc.store.snapshot().state.clone();
    svc.prefetch_library(&ed, &state).await;
    let lib = svc.library.clone();
    let palette = palette(&ed).await?;
    let labels = p.labels.unwrap_or(true);
    let made = ed
        .exec
        .cpu(move || -> Result<(Canvas, serde_json::Map<String, Value>), CmdError> {
            let own = |o: &Object| !o.marks.base;
            // the camera
            let cam = match (p.position, p.target, p.view.as_deref()) {
                (Some(eye), Some(at), None) => {
                    let (eye, at) = (Vec3::from_array(eye), Vec3::from_array(at));
                    if !eye.is_finite() || !at.is_finite() || eye.distance(at) < 1e-3 {
                        return Err(CmdError::invalid("position and target take two different points"));
                    }
                    Camera::look(eye, at, p.fov.unwrap_or(70.0))
                }
                (None, None, Some(word)) => {
                    let view = View::parse(word).ok_or_else(|| {
                        CmdError::invalid(format!("'{word}' is no view: top, bottom, front, back, left, right or iso"))
                    })?;
                    let mut b = Aabb::INVALID;
                    if let Some([x0, z0, x1, z1]) = p.area {
                        if ![x0, z0, x1, z1].iter().all(|v| v.is_finite()) || x1 <= x0 || z1 <= z0 {
                            return Err(CmdError::invalid("area takes [x0, z0, x1, z1] with x1 > x0 and z1 > z0"));
                        }
                        // as high as what stands there, up to the cut
                        let mut inside = Aabb::INVALID;
                        for o in state.objects.values() {
                            let at = o.position();
                            if own(o) && at.x >= x0 && at.x <= x1 && at.z >= z0 && at.z <= z1 {
                                inside.modify(at);
                            }
                        }
                        let top = p.cut.unwrap_or(if inside.is_valid() { inside.max.y + 3.0 } else { 10.0 });
                        let low = if inside.is_valid() { inside.min.y - 0.5 } else { top - 4.0 };
                        b.modify(Vec3::new(x0, low.min(top - 1.0), z0));
                        b.modify(Vec3::new(x1, top, z1));
                    } else {
                        let names: Option<Vec<String>> = p.objects.clone();
                        for o in state.objects.values() {
                            let wanted = match &names {
                                Some(list) => list.iter().any(|n| n.trim().eq_ignore_ascii_case(&o.name)),
                                None => own(o),
                            };
                            if wanted && let Some(ob) = gz_scene::sense::world_bounds(&state, &lib, o) {
                                b.modify(ob.min);
                                b.modify(ob.max);
                            }
                        }
                        if !b.is_valid() {
                            return Err(CmdError::not_found(
                                "nothing to frame: name objects, give an area, or position and target",
                            ));
                        }
                        b = Aabb::new(b.min - Vec3::splat(1.0), b.max + Vec3::splat(1.0));
                        if let Some(h) = p.cut {
                            b.max.y = b.max.y.min(h);
                        }
                    }
                    let (forward, up) = view.axes();
                    Camera::frame(b.min, b.max, forward, up, width, height, 12)
                }
                _ => {
                    return Err(CmdError::invalid(
                        "a picture of the scene takes position and target (as an eye sees), or view with objects or area (flat, to scale)",
                    ));
                }
            };
            let mut r = Raster::new(width, height, SKY);
            let mut triangles = 0;
            for o in state.objects.values().filter(|o| o.flags.visible) {
                let model = match o.class {
                    ObjClass::SpawnPoint => gz_scene::objects::visual_ref(o).and_then(|v| lib.cached(&v)),
                    ObjClass::SceneObject | ObjClass::Terrain => gz_scene::objects::terrain::mesh_of(&state, &lib, o),
                    _ => None,
                };
                let Some(model) = model else { continue };
                let world = if o.class == ObjClass::Terrain { Mat4::IDENTITY } else { o.world() };
                triangles += draw(&mut r, &cam, &model, &world, &palette, p.cut);
            }
            r.outline(Rgba(12, 14, 16, 255));
            // way paths: the links between their points
            for o in state.of_class(ObjClass::Way).filter(|o| own(o) && o.flags.visible) {
                if let ObjData::Way(w) = &o.data {
                    for pair in w.points.windows(2) {
                        r.line(&cam, pair[0].position + Vec3::Y * 0.1, pair[1].position + Vec3::Y * 0.1, Rgba(255, 150, 40, 255));
                    }
                }
            }
            // what the scene holds of its own, where it stands in the picture
            let mut legend = Vec::new();
            let mut marks: Vec<(f32, f32, Rgba, f32, String, bool)> = Vec::new();
            for o in state.objects.values().filter(|o| own(o) && o.flags.visible) {
                let at = match gz_scene::sense::world_bounds(&state, &lib, o) {
                    Some(b) if mark_of(o).is_none() => Vec3::new((b.min.x + b.max.x) * 0.5, b.max.y, (b.min.z + b.max.z) * 0.5),
                    _ => o.position(),
                };
                let Some((x, y, depth)) = cam.project(at, width, height) else { continue };
                if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
                    continue;
                }
                // behind something nearer by more than a hand: there, and not seen
                let seen = r.depth_at(x as u32, y as u32).is_none_or(|d| depth <= d + 0.3);
                legend.push(json!({
                    "name": o.name.as_str(),
                    "kind": o.class.key(),
                    "px": [x.round(), y.round()],
                    "depth": (f64::from(depth) * 100.0).round() / 100.0,
                    "seen": seen,
                }));
                let (colour, radius) = mark_of(o).unwrap_or((Rgba(255, 255, 255, 255), 2.0));
                marks.push((x, y, colour, radius, o.name.to_string(), seen));
            }
            let mut canvas = r.into_canvas();
            for (n, (x, y, colour, radius, name, seen)) in marks.iter().enumerate() {
                let colour = if *seen { *colour } else { colour.shade(0.55) };
                canvas.disc((*x, *y), *radius, Rgba(12, 14, 16, 255));
                canvas.disc((*x, *y), radius - 1.0, colour);
                // a name is written beside what the picture shows; what stands behind a wall keeps
                // its mark, dimmed, and its row of the legend
                if labels && *seen && n < MAX_LABELS {
                    canvas.label(*x as i64 + 6, *y as i64 - 12, name, 1, 2, INK, PLATE);
                }
            }
            let mut extra = serde_json::Map::new();
            let lens = match cam.lens {
                Lens::Ortho { px_per_m } => json!({ "flat": true, "px_per_m": (f64::from(px_per_m) * 100.0).round() / 100.0 }),
                Lens::Perspective { fov } => json!({ "flat": false, "fov": fov }),
            };
            extra.insert(
                "camera".into(),
                json!({ "eye": v3(cam.eye), "looks": v3(cam.forward), "right": v3(cam.right), "up": v3(cam.up), "lens": lens }),
            );
            extra.insert("triangles".into(), json!(triangles));
            extra.insert("legend".into(), json!(legend));
            extra.insert(
                "marks".into(),
                json!({ "light": "yellow", "sound": "blue", "particles": "magenta", "spawn point without a model": "green", "way path": "orange line", "a model": "white dot at its top" }),
            );
            Ok((canvas, extra))
        })
        .await?;
    png_reply(&made.0, made.1)
}

/// Registers the pictures.
pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::read(
            "asset.preview",
            "A model, a level of the game, a texture or a section's model drawn from the sides asked for, without a window.",
        )
        .example("ref=dynamics\\\\box\\\\box_wood_01.ogf views=[\"iso\",\"front\",\"top\"]"),
        asset_preview,
    );
    reg.cmd(
        Spec::read(
            "asset.thumbnail",
            "A sheet of small pictures of models, each with its name and size: what to choose from, kept in the game library.",
        )
        .example("refs=[\"dynamics\\\\box\\\\box_wood_01.ogf\",\"dynamics\\\\box\\\\box_wood_02.ogf\"]"),
        asset_thumbnail,
    );
    reg.cmd(
        Spec::read(
            "shot.scene",
            "The open scene drawn on the processor from a camera of the picture's own, with what the scene holds of its own named: no window, no graphics card.",
        )
        .example("view=top area=[28,-20,52,-5] cut=4.4"),
        shot_scene,
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_of_small_pictures_is_cut_to_its_cap_oldest_first() {
        let dir = std::env::temp_dir().join(format!("gz-gamelib-thumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let start = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        for i in 0..10u64 {
            for ext in ["png", "stamp"] {
                let f = dir.join(format!("{i:02}.{ext}"));
                std::fs::write(&f, b"x").unwrap();
                let at = start + std::time::Duration::from_secs(i * 60);
                std::fs::File::options().write(true).open(&f).unwrap().set_modified(at).unwrap();
            }
        }
        // within the cap nothing goes
        assert_eq!(prune_thumbs(&dir, 10), 0);
        // past it the oldest go with their stamps, down to three quarters of the cap
        assert_eq!(prune_thumbs(&dir, 8), 4);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), 12, "{left:?}");
        assert_eq!(left.first().map(String::as_str), Some("04.png"));
        assert_eq!(prune_thumbs(&dir.join("nowhere"), 1), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
