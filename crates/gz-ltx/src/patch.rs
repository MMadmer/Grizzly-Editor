//! `.ltxp` directive patches as data: what [`crate::Builder::patch`] applies and what an area
//! that makes directives of its own (an item asset) hands to [`crate::Builder::apply`].
//!
//! A file is read in blocks: the directives up to the first `#file <path under configs>` are
//! the system config's, the ones after it that file's, up to the next `#file` (`#file` alone goes
//! back to the system config). A block's lines are the game's: `[section]`, `[new] : parents`,
//! `[!section]`, `key = value`, `+key = a, b`, `-key = a`, `!key`; `;` starts a comment.

/// What one line of a patch does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// `[name]`, or `[name] : parents` (the parents count only for a section that is new).
    Section { name: String, parents: Option<Vec<String>> },
    /// `[!name]`.
    DropSection(String),
    /// `key = value`, the value as written.
    Set { key: String, value: String },
    /// `+key = a, b`: the members a list lacks, appended.
    Append { key: String, value: String },
    /// `-key = a`: the members taken out of a list.
    Remove { key: String, value: String },
    /// `!key`.
    Delete(String),
}

/// One line of a patch and where it stands (1-based; 0 for a line no file holds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    pub op: Op,
    pub line: u32,
}

impl Directive {
    #[must_use]
    pub fn new(op: Op, line: u32) -> Self {
        Self { op, line }
    }
}

/// The directives of one `#file` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The file as [`config_key`] writes it; `None` for the system config.
    pub file: Option<String>,
    pub directives: Vec<Directive>,
}

/// Directives for a config file outside the system tree, for every ini that reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePatch {
    /// The file, as [`config_key`] writes it.
    pub file: String,
    /// The patch they come from, for the tree's files.
    pub source: String,
    pub layer: u16,
    pub directives: Vec<Directive>,
}

/// The game's `ConfigPath`: a path under `configs` lower-cased, `/` as `\`, `.` and `..`
/// resolved; `None` when it climbs out (`..\x.ltx`).
#[must_use]
pub fn config_path(path: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for part in path.to_ascii_lowercase().split(['/', '\\']) {
        match part {
            ".." => {
                parts.pop()?;
            }
            "" | "." => {}
            p => parts.push(p.to_owned()),
        }
    }
    Some(parts.join("\\"))
}

/// A config's path as patches compare it, from a path of the game's tree or of a project:
/// without a leading `gamedata\` and `configs\`, then [`config_path`] (`Configs/Misc/Trade.LTX` ->
/// `misc\trade.ltx`).
#[must_use]
pub fn config_key(path: &str) -> String {
    let p = path.trim().replace('/', "\\").to_ascii_lowercase();
    let p = p.trim_start_matches('\\');
    let p = p.strip_prefix("gamedata\\").unwrap_or(p);
    let p = p.strip_prefix("configs\\").unwrap_or(p);
    config_path(p).unwrap_or_else(|| p.to_owned())
}

fn tr(s: &str) -> &str {
    s.trim_start_matches([' ', '\t']).trim_end_matches([' ', '\t', '\r'])
}

/// What a `#file` line sends the lines after it to.
enum Target {
    /// `#file` alone: the system config.
    System,
    File(String),
    /// A path that climbs out of `configs` or names a drive: nothing, until the next `#file`.
    Nowhere,
}

/// The target of a `#file` line (the word in any case), `None` for any other line.
fn file_line(t: &str) -> Option<Target> {
    let rest = t.get(..5).filter(|h| h.eq_ignore_ascii_case("#file")).and(t.get(5..))?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let written = tr(rest);
    if written.is_empty() {
        return Some(Target::System);
    }
    Some(match config_path(written) {
        Some(p) if !p.is_empty() && !p.contains(':') => Target::File(p),
        _ => Target::Nowhere,
    })
}

/// The blocks of a patch file, in order; an empty block is left out.
#[must_use]
pub fn parse(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut cur = Block { file: None, directives: Vec::new() };
    // after a `#file` that names no config nothing is read: it must not fall onto the system's
    let mut nowhere = false;
    for (n, raw) in text.split('\0').next().unwrap_or("").split('\n').enumerate() {
        let line = u32::try_from(n + 1).unwrap_or(u32::MAX);
        let raw = raw.split(';').next().unwrap_or("");
        let t = tr(raw);
        if t.is_empty() {
            continue;
        }
        if let Some(target) = file_line(t) {
            nowhere = matches!(target, Target::Nowhere);
            let file = match target {
                Target::File(p) => Some(p),
                Target::System | Target::Nowhere => None,
            };
            let done = std::mem::replace(&mut cur, Block { file, directives: Vec::new() });
            if !done.directives.is_empty() {
                out.push(done);
            }
            continue;
        }
        if nowhere {
            continue;
        }
        let op = if let Some(body) = t.strip_prefix('[') {
            let Some(close) = body.find(']') else { continue };
            let name = tr(body.get(..close).unwrap_or(""));
            let rest = tr(body.get(close + 1..).unwrap_or(""));
            match name.strip_prefix('!') {
                Some(del) => Op::DropSection(tr(del).to_owned()),
                None => {
                    let parents = rest.strip_prefix(':').map(|p| {
                        p.split(',')
                            .map(|x| tr(x).trim())
                            .filter(|x| !x.is_empty())
                            .map(str::to_owned)
                            .collect()
                    });
                    Op::Section { name: name.to_owned(), parents }
                }
            }
        } else if let Some(eq) = t.find('=') {
            let key = tr(t.get(..eq).unwrap_or(""));
            let value = tr(t.get(eq + 1..).unwrap_or("")).to_owned();
            if let Some(k) = key.strip_prefix('+') {
                Op::Append { key: tr(k).to_owned(), value }
            } else if let Some(k) = key.strip_prefix('-') {
                Op::Remove { key: tr(k).to_owned(), value }
            } else if key.is_empty() {
                continue;
            } else {
                Op::Set { key: key.to_owned(), value }
            }
        } else if let Some(k) = t.strip_prefix('!') {
            Op::Delete(tr(k).to_owned())
        } else {
            continue;
        };
        cur.directives.push(Directive { op, line });
    }
    if !cur.directives.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_follow_the_file_lines() {
        let text = "[wpn_ak74]\n+ammo_class = a ; c\n#FILE Misc/./x/../Death_Generic.ltx\n[base]\nwpn_x = 1\n#file\n!cost\n#file misc\\trade\\trade_military.ltx\n[supplies_1]\nx = 1, 1\n#filed\n#file ..\\outside.ltx\n[base]\nlost = 1\n#file c:\\x.ltx\n[base]\nlost = 2\n";
        let blocks = parse(text);
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].file, None);
        assert_eq!(
            blocks[0].directives,
            [
                Directive::new(Op::Section { name: "wpn_ak74".into(), parents: None }, 1),
                Directive::new(Op::Append { key: "ammo_class".into(), value: "a".into() }, 2),
            ]
        );
        assert_eq!(blocks[1].file.as_deref(), Some("misc\\death_generic.ltx"));
        assert_eq!(blocks[2].file, None);
        assert_eq!(blocks[2].directives, [Directive::new(Op::Delete("cost".into()), 7)]);
        assert_eq!(blocks[3].file.as_deref(), Some("misc\\trade\\trade_military.ltx"));
        // `#filed` is no `#file`: a line of nothing
        assert_eq!(blocks[3].directives.len(), 2);
        // a path that climbs out or names a drive takes everything up to the next `#file` with it
        assert!(
            blocks
                .iter()
                .all(|b| b.directives.iter().all(|d| !matches!(&d.op, Op::Set { key, .. } if key == "lost")))
        );
        assert_eq!(config_key("gamedata/Configs/A.ltx"), "a.ltx");
        assert_eq!(config_path("Weapons\\..\\Items/x.LTX").as_deref(), Some("items\\x.ltx"));
        assert_eq!(config_path("..\\x.ltx"), None);
        let new = parse("[n] : a , B\n[!old]\n-k = x\n");
        assert_eq!(
            new[0].directives[0].op,
            Op::Section { name: "n".into(), parents: Some(vec!["a".into(), "B".into()]) }
        );
        assert_eq!(new[0].directives[1].op, Op::DropSection("old".into()));
    }
}
