//! The Command Palette's rows and their order (docs/UI_DESIGN.md, section 5).
//!
//! A row is an action, an open tab or a panel of the Level tab. Only what a row shows is matched - its name, its category or path, its keys - with
//! the kit's ranking (`gz_ui::rank`: word prefixes first, then letters in order). Without a query
//! the actions run last from the palette lead, then the open tabs, then everything else in its
//! group's order.

use gz_ui::Icon;
use serde_json::{Value, json};

/// One row of the palette.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteRow {
    /// What runs it: an action's id, `tab:<id>`, `panel:<id>`, or a level command's id.
    pub id: String,
    /// `action`, `tab`, `panel`, `command`.
    pub kind: &'static str,
    pub label: String,
    /// The muted column: an action's group, "Tab", "Panel".
    pub category: String,
    /// What else the row says: a tab's project-relative path.
    pub detail: String,
    pub shortcut: String,
    pub icon: Icon,
    /// Off rows are listed with the reason and never run.
    pub enabled: bool,
    pub why: String,
    /// Run from the palette lately: its place among the recent ones, 0 the newest.
    pub recent: Option<usize>,
}

impl PaletteRow {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "label": self.label,
            "category": self.category,
            "detail": self.detail,
            "shortcut": self.shortcut,
            "enabled": self.enabled,
            "why": self.why,
            "recent": self.recent.is_some(),
        })
    }
}

/// The layout store's list of the actions run from the palette, newest first.
pub const RECENT_KEY: &str = "palette.recent";
/// How many it keeps.
pub const RECENT_KEEP: usize = 8;

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "tab" => 1,
        "panel" => 2,
        _ => 3,
    }
}

/// The rows for a query, best first.
#[must_use]
pub fn rank(rows: Vec<PaletteRow>, query: &str) -> Vec<PaletteRow> {
    if query.trim().is_empty() {
        let mut rows = rows;
        // the recent ones first, newest first; then tabs, panels, the rest as they came
        rows.sort_by_key(|r| match r.recent {
            Some(n) => (0, n),
            None => (kind_rank(r.kind), 0),
        });
        return rows;
    }
    let order = gz_ui::rank::rank(rows.len(), query, |i| {
        let r = &rows[i];
        let second = if r.detail.is_empty() { r.category.as_str() } else { r.detail.as_str() };
        (r.label.as_str(), second, r.shortcut.as_str())
    });
    let mut slots: Vec<Option<PaletteRow>> = rows.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots.get_mut(i).and_then(Option::take)).collect()
}

#[cfg(test)]
mod tests {
    use super::{PaletteRow, rank};
    use gz_ui::Icon;

    fn row(kind: &'static str, label: &str, category: &str) -> PaletteRow {
        PaletteRow {
            id: format!("{kind}:{label}"),
            kind,
            label: label.into(),
            category: category.into(),
            detail: String::new(),
            shortcut: String::new(),
            icon: Icon::None,
            enabled: true,
            why: String::new(),
            recent: None,
        }
    }

    #[test]
    fn log_finds_the_output_log_first() {
        let rows = vec![
            row("action", "Submit to Mod Browser", "Mod"),
            row("panel", "Output Log", "Panel"),
            row("action", "Log Drawer", "Window"),
            row("action", "Catalog", "Mod"),
        ];
        let out = rank(rows, "log");
        assert!(out.iter().take(2).any(|r| r.label == "Output Log"));
        assert!(!out.iter().any(|r| r.label == "Submit to Mod Browser"));
    }

    #[test]
    fn no_query_lists_the_recent_ones_then_the_tabs() {
        let mut undo = row("action", "Undo", "Edit");
        undo.recent = Some(0);
        let rows = vec![row("action", "Save", "File"), row("tab", "Level", "Tab"), undo];
        let out = rank(rows, "");
        assert_eq!(out.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Undo", "Level", "Save"]);
    }
}
