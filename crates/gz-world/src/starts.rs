//! The Player Starts of the project: `op = start` entries of the baked spawn layers
//! (`spawn\*.xspawn`), and the open scene's own through the scene area's slot.

use serde_json::{Value, json};

/// A Player Start: baked into a spawn layer, or placed in the open scene and not baked yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Start {
    pub level: String,
    /// The op (a file) or the object (the scene).
    pub name: String,
    /// The project-relative spawn file, or `scene`.
    pub source: String,
    pub position: [f32; 3],
    pub direction: [f32; 3],
}

/// The longest prefix of `s` that reads as a float, and what follows it (`%f`).
fn scan_float(s: &str) -> Option<(f32, &str)> {
    let t = s.trim_start_matches([' ', '\t', '\r', '\n']);
    let n =
        t.bytes().take_while(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E')).count();
    (1..=n).rev().find_map(|i| Some((t.get(..i)?.parse::<f32>().ok()?, t.get(i..)?)))
}

/// `sscanf("%f , %f , %f")`: the numbers that read, in order; the rest stay as they were.
fn parse_vec(text: &str, v: &mut [f32; 3]) {
    let mut rest = text;
    for (i, slot) in v.iter_mut().enumerate() {
        if i > 0 {
            let Some(r) = rest.trim_start_matches([' ', '\t', '\r', '\n']).strip_prefix(',') else { return };
            rest = r;
        }
        let Some((n, r)) = scan_float(rest) else { return };
        *slot = n;
        rest = r;
    }
}

/// The `op = start` entries of one spawn layer: `[obj:<name>]` sections whose
/// `op` is `start`, with their level, position and direction.
#[must_use]
pub fn read_starts(text: &str, source: &str) -> Vec<Start> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut keys: Vec<(String, String)> = Vec::new();
    let flush = |section: &str, keys: &mut Vec<(String, String)>, out: &mut Vec<Start>| {
        let get = |k: &str| keys.iter().rev().find(|(key, _)| key == k).map_or("", |(_, v)| v.as_str());
        if let Some(name) = section.strip_prefix("obj:")
            && get("op") == "start"
        {
            let mut s = Start {
                name: name.to_owned(),
                level: get("level").to_owned(),
                source: source.to_owned(),
                ..Start::default()
            };
            parse_vec(get("position"), &mut s.position);
            parse_vec(get("direction"), &mut s.direction);
            out.push(s);
        }
        keys.clear();
    };
    for raw in text.split('\n') {
        let trim = |s: &str| s.trim_matches([' ', '\t', '\r', '\n']).to_owned();
        let mut line = trim(raw);
        if let Some(sc) = line.find(';') {
            line = trim(line.get(..sc).unwrap_or(""));
        }
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            flush(&section, &mut keys, &mut out);
            section = rest.find(']').map_or(rest, |c| rest.get(..c).unwrap_or("")).to_owned();
            continue;
        }
        if let Some(eq) = line.find('=') {
            keys.push((trim(line.get(..eq).unwrap_or("")), trim(line.get(eq + 1..).unwrap_or(""))));
        }
    }
    flush(&section, &mut keys, &mut out);
    out
}

/// A number as `%.<digits>f` printed it.
fn fixed(v: f32, digits: usize) -> Value {
    format!("{v:.digits$}").parse::<f64>().map_or(Value::Null, |n| json!(n))
}

/// Three numbers as `%.<digits>f` printed them.
#[must_use]
pub fn fixed3(v: [f32; 3], digits: usize) -> Value {
    Value::Array(v.iter().map(|x| fixed(*x, digits)).collect())
}

/// The starts of the answers.
#[must_use]
pub fn starts_json(starts: &[Start]) -> Value {
    Value::Array(
        starts
            .iter()
            .map(|s| json!({ "level": s.level, "name": s.name, "source": s.source, "position": fixed3(s.position, 3), "direction": fixed3(s.direction, 4) }))
            .collect(),
    )
}
