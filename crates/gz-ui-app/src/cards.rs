//! What the Start page's cards show, worked out off the UI thread: the list with each folder's
//! state, the project's name by its project file, the text of a card (a twin's differing parent
//! folder), the order the page draws, the preview picture and whether it shows a scene at all.

use std::path::Path;
use std::sync::Arc;

use gz_base::Rev;
use gz_editor::Editor;
use gz_project::list::{entry_name, entry_state};
use gz_project::{EntryState, Projects, Sort};

/// Where a twin's parent folder and the leaf stand in the card's path (bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seg {
    pub begin: usize,
    pub end: usize,
    pub leaf: usize,
}

/// One card of the Start page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// The folder as `_projects.ini` spells it.
    pub path: String,
    /// The project's name: its project file's (`<Name>.grizzly`), else the folder's.
    pub name: String,
    pub state: EntryState,
    /// For two cards of one name, the nearest parent folder that tells them apart; empty for
    /// none.
    pub parent: String,
    /// Where that parent stands in `path`.
    pub seg: Option<Seg>,
    /// `preview.png`'s write time, when the folder holds one.
    pub preview: Option<Rev>,
}

/// The list as the page shows it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub cards: Vec<Card>,
    /// The New Project form's location.
    pub create_parent: String,
    /// The sort `_projects.ini [browser] sort` keeps.
    pub sort: Sort,
}

/// The byte ranges of a path's segments, split at `\` and `/`.
fn parts(path: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    for (i, c) in path.char_indices() {
        if c == '\\' || c == '/' {
            if i > from {
                out.push((from, i));
            }
            from = i + 1;
        }
    }
    if path.len() > from {
        out.push((from, path.len()));
    }
    out
}

/// The folder `up` levels above the leaf of `path` (`D:\a\b\Name`, 1 -> `b`).
#[must_use]
pub fn ancestor_of(path: &str, up: usize) -> Option<&str> {
    let p = parts(path);
    let at = p.len().checked_sub(1 + up)?;
    p.get(at).and_then(|&(a, b)| path.get(a..b))
}

/// Where the folder `up` levels above the leaf, and the leaf itself, stand in `path`.
#[must_use]
pub fn segment_ranges(path: &str, up: usize) -> Option<Seg> {
    let p = parts(path);
    let at = p.len().checked_sub(1 + up)?;
    let &(begin, end) = p.get(at)?;
    let &(leaf, _) = p.last()?;
    Some(Seg { begin, end, leaf })
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// One listed folder as read: its path, its state, its preview's write time, the name of its
/// project file.
pub type Row = (String, EntryState, Option<Rev>, Option<String>);

/// The cards of the listed folders: two of one name name the nearest parent folder that differs
/// between them - two `DARF-Overnight-Delivery` cards read `GrizzlyProjects` and `Temp` instead of
/// one word twice.
#[must_use]
pub fn card_texts(rows: Vec<Row>) -> Vec<Card> {
    let mut cards: Vec<Card> = rows
        .into_iter()
        .map(|(path, state, preview, file_name)| {
            let name = file_name.unwrap_or_else(|| entry_name(&path).to_owned());
            Card { path, name, state, parent: String::new(), seg: None, preview }
        })
        .collect();
    let n = cards.len();
    for i in 0..n {
        let Some(me) = cards.get(i).cloned() else { continue };
        let mut up = 1;
        while let Some(mine) = ancestor_of(&me.path, up) {
            let mut twin = false;
            let mut differs = true;
            for (j, other) in cards.iter().enumerate() {
                if j == i || !same_name(&other.name, &me.name) {
                    continue;
                }
                twin = true;
                if ancestor_of(&other.path, up).is_some_and(|theirs| same_name(theirs, mine)) {
                    differs = false;
                }
            }
            if !twin {
                break;
            }
            if differs {
                let parent = mine.to_owned();
                let seg = segment_ranges(&me.path, up);
                if let Some(card) = cards.get_mut(i) {
                    card.parent = parent;
                    card.seg = seg;
                }
                break;
            }
            up += 1;
        }
    }
    cards
}

/// The cards in the order the page shows them: the search filters ("name path", every word),
/// the sort orders (Name: by the folder's name, ignoring case; Last Opened: the list's order).
#[must_use]
pub fn card_order(cards: &[Card], search: &str, sort: Sort) -> Vec<usize> {
    let mut out: Vec<usize> = cards
        .iter()
        .enumerate()
        .filter(|(_, c)| gz_ui::search::matches(search, &format!("{} {}", c.name, c.path)))
        .map(|(i, _)| i)
        .collect();
    if sort == Sort::Name {
        out.sort_by_cached_key(|&i| cards.get(i).map(|c| c.name.to_lowercase()).unwrap_or_default());
    }
    out
}

/// A card's state as `project.list` and `project.page` name it.
#[must_use]
pub fn state_name(state: EntryState) -> &'static str {
    state.as_str()
}

/// `preview.png` of a project folder.
#[must_use]
pub fn preview_file(folder: &str) -> std::path::PathBuf {
    Path::new(folder).join("preview.png")
}

/// The write time of a folder's `preview.png` as a revision; `None` without the file. One
/// attribute read.
#[must_use]
pub fn preview_rev(folder: &str) -> Option<Rev> {
    let meta = std::fs::metadata(preview_file(folder)).ok()?;
    if !meta.is_file() {
        return None;
    }
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    // the size goes in as well: a copy made within the clock's step is still another picture
    Some(Rev((nanos ^ meta.len().rotate_left(48)) | 1))
}

/// The name of a folder's project file (`<Name>.grizzly`), when it holds one. One read of the
/// folder.
#[must_use]
pub fn project_name(folder: &str) -> Option<String> {
    gz_project::project::project_files(Path::new(folder))
        .first()
        .and_then(|f| f.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
}

/// Reads the list as the file has it now, and every folder's state, off the UI thread.
pub async fn load(editor: &Arc<Editor>) -> Snapshot {
    let list = Projects::of(editor).list(editor).await;
    let entries = list.entries();
    let create_parent = Projects::create_parent(&list);
    let sort = list.sort();
    let rows = editor
        .exec
        .io(move || {
            entries
                .into_iter()
                .map(|e| {
                    let state = entry_state(&e);
                    let (preview, name) = if state == EntryState::Project {
                        (preview_rev(&e), project_name(&e))
                    } else {
                        (None, None)
                    };
                    (e, state, preview, name)
                })
                .collect::<Vec<_>>()
        })
        .await;
    Snapshot { cards: card_texts(rows), create_parent, sort }
}

/// A card's picture is some 650 px wide at 3x: a whole viewport's worth of texture per card only
/// costs memory.
pub const PREVIEW_MAX_W: usize = 640;

/// No picture of a scene: the share of the most common colour (4 bits a channel) over a sample
/// grid. A target nothing was rendered into is one flat colour, an empty scene one grey (an older
/// editor wrote it with the stats text over it: 99.5% measured), a scene some 6%. `rgb` holds
/// three bytes a pixel.
#[must_use]
pub fn shows_no_scene(rgb: &[u8], w: usize, h: usize) -> bool {
    if w == 0 || h == 0 || rgb.len() < w * h * 3 {
        return true;
    }
    let step = (((w as f64) * (h as f64) / 65536.0).sqrt() as usize).max(1);
    let mut bins = vec![0u32; 4096];
    let (mut samples, mut top) = (0u64, 0u32);
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = (y * w + x) * 3;
            let (Some(&r), Some(&g), Some(&b)) = (rgb.get(i), rgb.get(i + 1), rgb.get(i + 2)) else {
                continue;
            };
            let key = (usize::from(r >> 4) << 8) | (usize::from(g >> 4) << 4) | usize::from(b >> 4);
            if let Some(bin) = bins.get_mut(key) {
                *bin += 1;
                top = top.max(*bin);
            }
            samples += 1;
        }
    }
    u64::from(top) * 50 >= samples * 49
}

/// A decoded preview: opaque RGB, averaged down to [`PREVIEW_MAX_W`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub w: usize,
    pub h: usize,
    /// Three bytes a pixel.
    pub rgb: Vec<u8>,
}

/// Decodes a `preview.png` and averages it down by a whole factor to at most
/// [`PREVIEW_MAX_W`] wide. A picture of no scene (a black or empty grey frame) is `None`: the card draws its placeholder; the file itself is the project's and stays.
#[must_use]
pub fn decode_preview(bytes: &[u8]) -> Option<Picture> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let size = reader.output_buffer_size()?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).ok()?;
    let (iw, ih) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let stride = info.line_size;
    if iw == 0 || ih == 0 || stride < iw * channels || buf.len() < stride * ih {
        return None;
    }
    let pixel = |x: usize, y: usize| -> [u32; 3] {
        let i = y * stride + x * channels;
        let at = |k: usize| u32::from(buf.get(i + k).copied().unwrap_or(0));
        match channels {
            1 | 2 => [at(0), at(0), at(0)],
            _ => [at(0), at(1), at(2)],
        }
    };
    let f = (iw / PREVIEW_MAX_W).max(1);
    let (ow, oh) = (iw / f, ih / f);
    if ow == 0 || oh == 0 {
        return None;
    }
    let n = (f * f) as u32;
    let mut rgb = Vec::with_capacity(ow * oh * 3);
    for y in 0..oh {
        for x in 0..ow {
            let mut sum = [0u32; 3];
            for sy in 0..f {
                for sx in 0..f {
                    for (s, v) in sum.iter_mut().zip(pixel(x * f + sx, y * f + sy)) {
                        *s += v;
                    }
                }
            }
            rgb.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    if shows_no_scene(&rgb, ow, oh) {
        return None;
    }
    Some(Picture { w: ow, h: oh, rgb })
}

/// Reads and decodes a project folder's `preview.png`; `None` without one or for a picture of no
/// scene. Blocking: the pools run it.
#[must_use]
pub fn read_preview(folder: &str) -> Option<Picture> {
    let bytes = std::fs::read(preview_file(folder)).ok()?;
    decode_preview(&bytes)
}

/// The UV rectangle that cuts a `have_w` x `have_h` picture to a `want_w` x `want_h` frame
/// without stretching it: the middle of the picture.
#[must_use]
pub fn crop_uv(have_w: f32, have_h: f32, want_w: f32, want_h: f32) -> egui::Rect {
    let (mut u0, mut v0, mut u1, mut v1) = (0.0, 0.0, 1.0, 1.0);
    if have_w <= 0.0 || have_h <= 0.0 || want_w <= 0.0 || want_h <= 0.0 {
        return egui::Rect::from_min_max(egui::pos2(u0, v0), egui::pos2(u1, v1));
    }
    let want = want_w / want_h;
    let have = have_w / have_h;
    if have > want {
        u0 = (1.0 - want / have) * 0.5;
        u1 = 1.0 - u0;
    } else {
        v0 = (1.0 - have / want) * 0.5;
        v1 = 1.0 - v0;
    }
    egui::Rect::from_min_max(egui::pos2(u0, v0), egui::pos2(u1, v1))
}

/// What a deletion or its check answered over the wire, back as the confirmation shows it (the
/// `project.delete` reply: the counts under `deleted`, paths with forward slashes).
#[must_use]
pub fn info_from_reply(reply: &serde_json::Value) -> gz_project::DeleteInfo {
    use serde_json::Value;
    let text = |k: &str| {
        reply.get(k).and_then(Value::as_str).map(gz_project::paths::to_backslash).unwrap_or_default()
    };
    let flag = |k: &str| reply.get(k).and_then(Value::as_bool).unwrap_or(false);
    let num = |v: Option<&Value>| v.and_then(Value::as_u64).unwrap_or(0);
    let counts = reply.get("deleted").unwrap_or(reply);
    let u32_of = |v: Option<&Value>| u32::try_from(num(v)).unwrap_or(u32::MAX);
    let refused = flag("refused").then(|| {
        reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("The project cannot be deleted from here.")
            .to_owned()
    });
    gz_project::DeleteInfo {
        path: text("path"),
        name: text("name"),
        refused,
        missing: flag("missing"),
        files: u32_of(counts.get("files")),
        folders: u32_of(counts.get("folders")),
        links: u32_of(counts.get("links")),
        bytes: num(counts.get("bytes")),
        git: flag("git"),
        releases: flag("releases"),
        release_files: u32_of(reply.get("release_files")),
        release_bytes: num(reply.get("release_bytes")),
        nested: reply
            .get("nested")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(gz_project::paths::to_backslash).collect())
            .unwrap_or_default(),
        nested_total: u32_of(reply.get("nested_total")),
        failed: reply
            .get("failed")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|f| gz_project::DeleteFailure {
                        path: f
                            .get("path")
                            .and_then(Value::as_str)
                            .map(gz_project::paths::to_backslash)
                            .unwrap_or_default(),
                        reason: f.get("reason").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        failed_total: u32_of(reply.get("failed_total")),
        removed_from_list: flag("removed_from_list"),
    }
}

/// `n file` / `n files`.
#[must_use]
pub fn count_text(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A size as the confirmation names it: bytes, KB, MB, GB.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1 << 20;
    const GB: u64 = 1 << 30;
    if bytes < KB {
        format!("{bytes} bytes")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str) -> Row {
        (path.to_owned(), EntryState::Project, None, None)
    }

    #[test]
    fn a_card_is_named_by_its_project_file() {
        let cards =
            card_texts(vec![("D:/p/folder".to_owned(), EntryState::Project, None, Some("Night".to_owned()))]);
        assert_eq!(cards.first().map(|c| c.name.as_str()), Some("Night"));
    }

    #[test]
    fn ancestors_and_ranges() {
        assert_eq!(ancestor_of("D:\\a\\b\\Name", 1), Some("b"));
        assert_eq!(ancestor_of("D:\\a\\b\\Name", 3), Some("D:"));
        assert_eq!(ancestor_of("D:\\a\\b\\Name", 4), None);
        assert_eq!(ancestor_of("D:/a//b/Name/", 1), Some("b"));
        assert_eq!(segment_ranges("D:\\a\\b\\Name", 1), Some(Seg { begin: 5, end: 6, leaf: 7 }));
    }

    #[test]
    fn twins_name_the_parent_that_differs() {
        let cards = card_texts(vec![
            row("C:\\Users\\x\\GrizzlyProjects\\Mod"),
            row("C:\\Temp\\Mod"),
            row("C:\\Other\\Solo"),
        ]);
        assert_eq!(cards.first().map(|c| c.parent.as_str()), Some("GrizzlyProjects"));
        assert_eq!(cards.get(1).map(|c| c.parent.as_str()), Some("Temp"));
        assert_eq!(cards.get(2).map(|c| c.parent.as_str()), Some(""));
        // the same parent one level up is no difference: the next one up is named
        let cards = card_texts(vec![row("D:\\A\\Twin\\Mod"), row("D:\\B\\Twin\\Mod")]);
        assert_eq!(cards.first().map(|c| c.parent.as_str()), Some("A"));
        assert_eq!(cards.get(1).map(|c| c.parent.as_str()), Some("B"));
        assert_eq!(cards.first().and_then(|c| c.seg), Some(Seg { begin: 3, end: 4, leaf: 10 }));
    }

    #[test]
    fn order_filters_and_sorts() {
        let cards = card_texts(vec![row("D:\\p\\beta"), row("D:\\p\\Alpha"), row("D:\\q\\gamma")]);
        assert_eq!(card_order(&cards, "", Sort::Recent), vec![0, 1, 2]);
        assert_eq!(card_order(&cards, "", Sort::Name), vec![1, 0, 2]);
        assert_eq!(card_order(&cards, "ALPHA", Sort::Recent), vec![1]);
        assert_eq!(card_order(&cards, "p a", Sort::Recent), vec![0, 1]);
    }

    #[test]
    fn a_flat_picture_shows_no_scene() {
        let flat = vec![85u8; 64 * 32 * 3];
        assert!(shows_no_scene(&flat, 64, 32));
        let mut varied = Vec::with_capacity(64 * 32 * 3);
        for i in 0..64 * 32 {
            varied.extend([(i % 256) as u8, ((i * 7) % 256) as u8, ((i * 13) % 256) as u8]);
        }
        assert!(!shows_no_scene(&varied, 64, 32));
        assert!(shows_no_scene(&[], 0, 0));
    }

    #[test]
    fn crops_keep_the_middle() {
        let uv = crop_uv(200.0, 100.0, 100.0, 100.0);
        assert!((uv.min.x - 0.25).abs() < 1e-6 && (uv.max.x - 0.75).abs() < 1e-6 && uv.min.y == 0.0);
        let uv = crop_uv(100.0, 200.0, 100.0, 100.0);
        assert!((uv.min.y - 0.25).abs() < 1e-6 && uv.min.x == 0.0);
    }

    #[test]
    fn texts() {
        assert_eq!(count_text(1, "file", "files"), "1 file");
        assert_eq!(count_text(3, "file", "files"), "3 files");
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(3 * 1_048_576), "3.0 MB");
    }

    #[test]
    fn a_reply_reads_back() {
        let r = serde_json::json!({ "ok": false, "error": "it is the open project", "path": "D:/p/x", "name": "x", "refused": true,
            "deleted": { "files": 3, "folders": 1, "links": 0, "bytes": 10 }, "git": true, "releases": false, "release_files": 0,
            "release_bytes": 0, "nested_total": 0, "failed_total": 0, "nested": [], "failed": [{ "path": "D:/p/x/a", "reason": "in use" }] });
        let info = info_from_reply(&r);
        assert_eq!(info.path, "D:\\p\\x");
        assert_eq!(info.refused.as_deref(), Some("it is the open project"));
        assert_eq!((info.files, info.folders, info.bytes), (3, 1, 10));
        assert!(info.git);
        assert_eq!(info.failed.first().map(|f| f.path.as_str()), Some("D:\\p\\x\\a"));
    }
}
