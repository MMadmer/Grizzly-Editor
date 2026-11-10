//! The Command Palette's ranking (docs/UI_DESIGN.md, section 6, "Command Palette").
//!
//! Only what a row shows is matched - its name, where it lives, its keys - never the COMMAND_*
//! id: "log" once found Submit to Mod Browser by COMMAND_MOD_CATALOG.

use gz_ui::Icon;
use serde_json::{Value, json};

/// One row of the palette.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteRow {
    /// What runs it: `tab:<id>`, `panel:<id>`, `drawer:<id>`, or a command's id.
    pub id: String,
    /// `tab`, `panel`, `drawer`, `command`.
    pub kind: &'static str,
    pub label: String,
    pub category: String,
    /// "Edit > Undo", "Window > Panel", "Tab > quests\a.nqasset".
    pub path: String,
    /// The id without its preset: the registry's name.
    pub name: String,
    pub shortcut: String,
    pub icon: Icon,
    /// The command's index in the shell's table; `None` for a tab, a panel or a drawer.
    pub command: Option<usize>,
    pub score: i32,
}

impl PaletteRow {
    #[must_use]
    pub fn to_json(&self) -> Value {
        let command = self.command.map_or(-1, |c| i64::try_from(c).unwrap_or(-1));
        json!({
            "id": self.id,
            "kind": self.kind,
            "label": self.label,
            "category": self.category,
            "path": self.path,
            "name": self.name,
            "shortcut": self.shortcut,
            "command": command,
            "subcommand": if command < 0 { -1 } else { 0 },
            "score": self.score,
        })
    }
}

pub const NO_MATCH: i32 = 0x3fff_ffff;
/// A match that is only every letter in order: never better than any real one.
const LETTERS: i32 = 160;

fn is_break(c: u8) -> bool {
    matches!(c, b' ' | b'_' | b'-' | b'/' | b'\\' | b'>' | b'.' | b'(' | b':')
}

/// Lower is better. The whole name, then its start, then the start of one of its words ("log"
/// finds Output Log before Catalog), then anywhere in it, then every letter in order.
#[must_use]
pub fn score_in(token: &str, hay: &str, letters: bool) -> i32 {
    if token.is_empty() || hay.is_empty() {
        return NO_MATCH;
    }
    if hay == token {
        return 0;
    }
    let h = hay.as_bytes();
    if hay.starts_with(token) {
        return 4 + ((hay.len() - token.len()) / 8) as i32;
    }
    let mut from = 0;
    while let Some(off) = hay.get(from..).and_then(|rest| rest.find(token)) {
        let at = from + off;
        if at > 0 && h.get(at - 1).copied().is_some_and(is_break) {
            let mut word = 0;
            for i in 0..at {
                if h.get(i).copied().is_some_and(is_break) && h.get(i + 1).copied().is_some_and(|c| !is_break(c)) {
                    word += 1;
                }
            }
            return 16 + word * 2;
        }
        from = at + 1;
        if from >= hay.len() {
            break;
        }
    }
    if let Some(sub) = hay.find(token) {
        return 48 + sub as i32;
    }
    if !letters {
        return NO_MATCH;
    }
    // each letter in order starts a word or follows the one before ("big" finds Build Into
    // Game); letters scattered through the middle of words are no match at all
    let t = token.as_bytes();
    let (mut matched, mut first, mut previous, mut gaps) = (0usize, None::<usize>, 0usize, 0i32);
    for (i, &c) in h.iter().enumerate() {
        if matched >= t.len() {
            break;
        }
        if t.get(matched) != Some(&c) {
            continue;
        }
        let follows = first.is_some() && i == previous + 1;
        if !follows && i > 0 && !h.get(i - 1).copied().is_some_and(is_break) {
            continue;
        }
        if first.is_none() {
            first = Some(i);
        } else {
            gaps += (i - previous - 1) as i32;
        }
        previous = i;
        matched += 1;
    }
    match first {
        Some(f) if matched == t.len() => LETTERS + f as i32 + gaps * 3,
        _ => NO_MATCH,
    }
}

/// Every word of the query matches what the row shows: the name counts most, the path less, the
/// keys least.
#[must_use]
pub fn score_row(label: &str, path: &str, shortcut: &str, tokens: &[String]) -> i32 {
    let (label, path, shortcut) = (label.to_lowercase(), path.to_lowercase(), shortcut.to_lowercase());
    let mut total = 0;
    for token in tokens {
        let best = score_in(token, &label, true).min(score_in(token, &path, false).saturating_add(12)).min(score_in(token, &shortcut, false).saturating_add(30));
        if best >= NO_MATCH {
            return NO_MATCH;
        }
        total += best;
    }
    total
}

/// The query's words, lower case.
#[must_use]
pub fn tokenize(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

fn kind_rank(kind: &str) -> i32 {
    match kind {
        "tab" => 0,
        "panel" => 1,
        "drawer" => 2,
        _ => 3,
    }
}

/// Ranks the rows for a query and keeps the best `limit`.
#[must_use]
pub fn rank(mut rows: Vec<PaletteRow>, query: &str, limit: usize) -> Vec<PaletteRow> {
    let tokens = tokenize(query);
    let ranked = !tokens.is_empty();
    for row in &mut rows {
        row.score = if ranked { score_row(&row.label, &row.path, &row.shortcut, &tokens) } else { kind_rank(row.kind) };
    }
    rows.retain(|r| r.score < NO_MATCH);
    rows.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then_with(|| if ranked { a.label.len().cmp(&b.label.len()) } else { std::cmp::Ordering::Equal })
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
            .then_with(|| a.id.to_lowercase().cmp(&b.id.to_lowercase()))
    });
    rows.truncate(limit);
    rows
}

/// The part of the path before a command's own name: "Scene > Build" of "Scene > Build > Make All".
#[must_use]
pub fn parent(row: &PaletteRow) -> &str {
    if row.kind != "command" {
        return &row.path;
    }
    row.path.rfind(" > ").map_or("", |at| row.path.get(..at).unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::{NO_MATCH, PaletteRow, rank, score_in};
    use gz_ui::Icon;

    fn row(kind: &'static str, label: &str, path: &str) -> PaletteRow {
        PaletteRow {
            id: format!("{kind}:{label}"),
            kind,
            label: label.into(),
            category: String::new(),
            path: path.into(),
            name: String::new(),
            shortcut: String::new(),
            icon: Icon::None,
            command: None,
            score: 0,
        }
    }

    #[test]
    fn words_beat_the_middle() {
        assert_eq!(score_in("log", "log", true), 0);
        assert!(score_in("log", "output log", true) < score_in("log", "catalog", true));
        assert!(score_in("big", "build into game", true) < NO_MATCH);
        assert_eq!(score_in("log", "build into game", true), NO_MATCH);
    }

    #[test]
    fn log_finds_the_output_log_first() {
        let rows = vec![
            row("command", "Submit to Mod Browser", "Mod > Submit to Mod Browser"),
            row("panel", "Output Log", "Window > Panel"),
            row("drawer", "Log Drawer", "Window > Drawer"),
            row("command", "Catalog", "Mod > Catalog"),
        ];
        let out = rank(rows, "log", 10);
        assert!(out.iter().take(2).any(|r| r.label == "Output Log"));
        assert!(!out.iter().any(|r| r.label == "Submit to Mod Browser"));
    }

    #[test]
    fn no_query_lists_tabs_first() {
        let rows = vec![row("command", "Undo", "Edit > Undo"), row("tab", "Level", "Tab")];
        let out = rank(rows, "", 10);
        assert_eq!(out.first().map(|r| r.kind), Some("tab"));
    }
}
