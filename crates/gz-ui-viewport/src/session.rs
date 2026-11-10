//! The camera of each scene, kept beside the project's session (`<project>\temp\viewport.ltx`):
//! a scene opens where the author last looked at it. The scene file's own camera (`SceneMeta`)
//! is the fallback for a scene this project never showed.

use std::path::Path;

use glam::Vec3;
use gz_ltx::LtxDoc;
use rustc_hash::FxHashMap;

/// The file under the project's `temp`.
pub const FILE: &str = "viewport.ltx";

/// A camera as it is kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Saved {
    pub hpb: Vec3,
    pub position: Vec3,
}

/// The key a scene is kept under: its path in the project (lower case, back slashes), or the
/// absolute one for a scene outside it.
#[must_use]
pub fn scene_key(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    rel.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn vec3(text: &str) -> Option<Vec3> {
    let v: Vec<f32> = text.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match v[..] {
        [x, y, z] if x.is_finite() && y.is_finite() && z.is_finite() => Some(Vec3::new(x, y, z)),
        _ => None,
    }
}

/// Every scene's camera in a file's text; what does not read is left out.
#[must_use]
pub fn parse(text: &str) -> FxHashMap<String, Saved> {
    let doc = LtxDoc::parse(text);
    let mut out = FxHashMap::default();
    for s in doc.sections() {
        if let (Some(hpb), Some(position)) = (doc.get(s, "hpb").and_then(vec3), doc.get(s, "position").and_then(vec3)) {
            out.insert(s.to_lowercase(), Saved { hpb, position });
        }
    }
    out
}

/// The file's text, scenes in name order.
#[must_use]
pub fn write(map: &FxHashMap<String, Saved>) -> String {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let mut doc = LtxDoc::new();
    for k in keys {
        let Some(s) = map.get(k) else { continue };
        doc.set(k, "position", &format!("{:.4}, {:.4}, {:.4}", s.position.x, s.position.y, s.position.z));
        doc.set(k, "hpb", &format!("{:.5}, {:.5}, {:.5}", s.hpb.x, s.hpb.y, s.hpb.z));
    }
    doc.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cameras_round_trip() {
        let mut m = FxHashMap::default();
        m.insert("rawdata\\levels\\a.level".to_owned(), Saved { hpb: Vec3::new(0.5, -0.25, 0.0), position: Vec3::new(1.0, 2.0, 3.5) });
        let text = write(&m);
        let back = parse(&text);
        let s = back.get("rawdata\\levels\\a.level").copied();
        assert!(s.is_some_and(|s| (s.position - Vec3::new(1.0, 2.0, 3.5)).length() < 1e-3 && (s.hpb.x - 0.5).abs() < 1e-4));
        assert_eq!(scene_key(Path::new("C:\\p"), Path::new("C:\\p\\RawData\\Levels\\A.level")), "rawdata\\levels\\a.level");
        assert!(parse("[x]\nhpb = 1, 2\n").is_empty());
    }
}
