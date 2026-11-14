//! How the graph names and colours a kind: shared by the canvas and the panels, and aware of the
//! document - a behaviour graph never says "quest".

use std::sync::Arc;

use egui::{Color32, Id, Ui};
use gz_nq::Catalog;
use gz_ui::{Combo, Icon, Palette, PickItem, PickList, palette};

/// The family a kind belongs to: what colours its header, its icon and its block on the minimap.
/// A family says what a node is, never its state - state is the status tones'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Family {
    #[default]
    Muted,
    Trigger,
    Flow,
    Wait,
    Dialog,
    Task,
    Role,
    Item,
    World,
    Logic,
}

impl Family {
    /// Where a family stands in a list of kinds: what starts a graph first, what decides its way,
    /// what it waits for and says, then what it does.
    #[must_use]
    pub fn order(self) -> usize {
        match self {
            Family::Trigger => 0,
            Family::Flow => 1,
            Family::Wait => 2,
            Family::Dialog => 3,
            Family::Task => 4,
            Family::Item => 5,
            Family::World => 6,
            Family::Role => 7,
            Family::Logic => 8,
            Family::Muted => 9,
        }
    }

    /// Its colour among the theme's `graph.family.*` hues; none for a kind nobody knows.
    #[must_use]
    pub fn index(self) -> Option<usize> {
        match self {
            Family::Muted => None,
            Family::Task => Some(1),
            Family::Wait => Some(2),
            Family::Item => Some(3),
            Family::Trigger => Some(4),
            Family::World => Some(5),
            Family::Logic => Some(6),
            Family::Dialog => Some(7),
            Family::Flow => Some(8),
            Family::Role => Some(9),
        }
    }
}

/// The icon of a kind: its catalog group, else its name's prefix, else the family's glyph - a
/// row is never a bare dot.
#[must_use]
pub fn kind_icon(cat: &Catalog, kind: &str) -> Icon {
    if kind.is_empty() {
        return Icon::Flow;
    }
    match kind {
        "flow.end" => return Icon::End,
        "behaviour.run" => return Icon::Behaviour,
        "lua" => return Icon::FileScript,
        "wait.timer" => return Icon::Timer,
        _ => {}
    }
    if gz_nq::text::is_trigger_kind(kind) {
        return Icon::Trigger;
    }
    const GROUPS: [(&str, Icon); 28] = [
        ("triggers", Icon::Trigger),
        ("flow", Icon::Flow),
        ("wait", Icon::Wait),
        ("dialog", Icon::Dialog),
        ("dialog_ctl", Icon::Dialog),
        ("objective", Icon::Task),
        ("tasks", Icon::Task),
        ("quest", Icon::Quest),
        ("vars", Icon::Variable),
        ("items", Icon::Item),
        ("money", Icon::Money),
        ("spawn", Icon::SpawnNpc),
        ("npc", Icon::Role),
        ("behaviour", Icon::Behaviour),
        ("orders", Icon::Order),
        ("relation", Icon::Relation),
        ("squad", Icon::Squad),
        ("world", Icon::World),
        ("actor", Icon::Person),
        ("ui", Icon::PdaMsg),
        ("info", Icon::PdaMsg),
        ("sound", Icon::FileSound),
        ("logic", Icon::FileScript),
        ("custom", Icon::FileScript),
        ("conditions", Icon::Condition),
        ("cond_event", Icon::Condition),
        ("cond_state", Icon::Condition),
        ("cond_world", Icon::Condition),
    ];
    if let Some(k) = cat.find(kind)
        && let Some((_, icon)) = GROUPS.iter().find(|(g, _)| *g == k.group)
    {
        return *icon;
    }
    // a module's own group: the kind's name still says what it is
    const PREFIXES: [(&str, Icon); 18] = [
        ("flow.", Icon::Flow),
        ("wait.", Icon::Wait),
        ("dialog.", Icon::Dialog),
        ("objective.", Icon::Task),
        ("task.", Icon::Task),
        ("var.", Icon::Variable),
        ("global.", Icon::Variable),
        ("item.", Icon::Item),
        ("money.", Icon::Money),
        ("spawn.", Icon::SpawnNpc),
        ("npc.", Icon::Role),
        ("order.", Icon::Order),
        ("relation.", Icon::Relation),
        ("squad.", Icon::Squad),
        ("pda.", Icon::PdaMsg),
        ("news.", Icon::PdaMsg),
        ("actor.", Icon::Person),
        ("world.", Icon::World),
    ];
    PREFIXES.iter().find(|(p, _)| kind.starts_with(p)).map_or(Icon::Flow, |(_, i)| *i)
}

/// The family of an icon.
#[must_use]
pub fn family_of(icon: Icon) -> Family {
    match icon {
        Icon::Trigger => Family::Trigger,
        Icon::End | Icon::Flow => Family::Flow,
        Icon::Wait | Icon::Timer => Family::Wait,
        Icon::Dialog => Family::Dialog,
        Icon::Task | Icon::Quest | Icon::PdaMsg => Family::Task,
        Icon::Role | Icon::Behaviour | Icon::Order | Icon::Relation | Icon::Person => Family::Role,
        Icon::Item | Icon::Money => Family::Item,
        Icon::SpawnNpc | Icon::Squad | Icon::World => Family::World,
        Icon::Variable | Icon::FileScript | Icon::Condition | Icon::FileSound => Family::Logic,
        _ => Family::Muted,
    }
}

/// The colour of a family, at `alpha` of its own.
#[must_use]
pub fn family_color(p: &Palette, f: Family, alpha: f32) -> Color32 {
    let c = f.index().map_or(p.text_muted, |i| p.family(i));
    Palette::alpha(c, alpha)
}

/// The family colour of a kind.
#[must_use]
pub fn kind_color(p: &Palette, cat: &Catalog, kind: &str, alpha: f32) -> Color32 {
    family_color(p, family_of(kind_icon(cat, kind)), alpha)
}

/// The title of a kind as the document speaks (`NqLook::KindTitle`).
#[must_use]
pub fn kind_title(cat: &Catalog, kind: &str, behaviour: bool) -> String {
    gz_nq::text::kind_title(cat, kind, behaviour)
}

/// The description of a kind as the document speaks (`NqLook::KindDesc`).
#[must_use]
pub fn kind_desc(cat: &Catalog, kind: &str, behaviour: bool) -> String {
    match cat.find(kind) {
        None => String::new(),
        Some(k) if behaviour => gz_nq::text::regraph(&k.desc),
        Some(k) => k.desc.clone(),
    }
}

/// The kinds of a use mask as the kit's pick list rows, grouped by family: worked out once per
/// catalog and kept.
fn kind_items(ui: &Ui, cat: &Catalog, mask: u8, behaviour: bool) -> Arc<Vec<PickItem>> {
    let id = Id::unique(("gz-ui-graph.kind_items", cat.generation, mask, behaviour));
    if let Some(v) = ui.ctx().data(|d| d.get_temp::<Arc<Vec<PickItem>>>(id)) {
        return v;
    }
    let p = palette(ui.ctx());
    let mut rows: Vec<(usize, PickItem)> = cat
        .kinds_for(mask, behaviour)
        .into_iter()
        .map(|k| {
            let icon = kind_icon(cat, &k.id);
            let family = family_of(icon);
            let first = first_sentence(&kind_desc(cat, &k.id, behaviour));
            let item = PickItem::new(k.id.clone(), kind_title(cat, &k.id, behaviour))
                .icon(icon)
                .tint(family_color(&p, family, 1.0))
                .detail(if first.is_empty() { k.id.clone() } else { first });
            (family.order(), item)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
    let v: Arc<Vec<PickItem>> = Arc::new(rows.into_iter().map(|(_, it)| it).collect());
    ui.ctx().data_mut(|d| d.insert_temp(id, v.clone()));
    v
}

/// The first sentence of a description: the second line of a pick list's row.
#[must_use]
pub fn first_sentence(desc: &str) -> String {
    let line = desc.lines().next().unwrap_or("").trim();
    match line.find(". ") {
        Some(i) => line[..=i].to_owned(),
        None => line.to_owned(),
    }
}

/// A kind picked from the kinds of a use mask: the kit's combo over its pick list, under the
/// names the document speaks. Returns whether the kind changed.
pub fn kind_combo(
    ui: &mut Ui,
    id_salt: &str,
    cat: &Catalog,
    mask: u8,
    behaviour: bool,
    kind: &mut String,
) -> bool {
    let items = kind_items(ui, cat, mask, behaviour);
    let shown = kind_title(cat, kind, behaviour);
    let w = ui.available_width();
    let current = kind.clone();
    let list = PickList::new(("gz-ui-graph.kind", id_salt), &items).current(&current).hint("Search kinds");
    let picked = Combo::new(("gz-ui-graph.kind_combo", id_salt), &shown)
        .icon(kind_icon(cat, kind))
        .width(w)
        .show(ui, list);
    match picked.and_then(|i| items.get(i)) {
        Some(it) if it.id != *kind => {
            kind.clone_from(&it.id);
            true
        }
        _ => false,
    }
}

/// "quest" or "graph" in a sentence (`NqLook::DocNoun`).
#[must_use]
pub fn doc_noun(behaviour: bool) -> &'static str {
    gz_nq::text::doc_noun(behaviour)
}

/// `spot_task` -> `Spot task`: a catalog name read as a label (`Pretty`).
#[must_use]
pub fn pretty(name: &str) -> String {
    const WORDS: [(&str, &str); 6] =
        [("id", "ID"), ("npc", "NPC"), ("npcs", "NPCs"), ("pda", "PDA"), ("lua", "Lua"), ("ui", "UI")];
    let mut out = String::with_capacity(name.len() + 4);
    for w in name.split('_').filter(|w| !w.is_empty()) {
        let mut word = WORDS
            .iter()
            .find(|(from, _)| *from == w)
            .map_or_else(|| w.to_owned(), |(_, to)| (*to).to_owned());
        if out.is_empty()
            && let Some(first) = word.get(..1)
            && first.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        {
            let up = first.to_ascii_uppercase();
            word.replace_range(..1, &up);
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
    }
    if out.is_empty() { name.to_owned() } else { out }
}

/// `3 nodes`, `1 error` (`Plural`).
#[must_use]
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The shortest plain form of a number.
#[must_use]
pub fn num(v: f64) -> String {
    gz_luadata::Value::Number(v).to_text("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(pretty("spot_task"), "Spot task");
        assert_eq!(pretty("npc_id"), "NPC ID");
        assert_eq!(pretty("ui"), "UI");
        assert_eq!(plural(1, "node", "nodes"), "1 node");
        assert_eq!(plural(3, "node", "nodes"), "3 nodes");
        assert_eq!(num(5.0), "5");
        assert_eq!(num(0.5), "0.5");
    }

    #[test]
    fn icons_by_prefix() {
        let cat = Catalog::empty();
        assert_eq!(kind_icon(&cat, "trigger.start"), Icon::Trigger);
        assert_eq!(kind_icon(&cat, "flow.end"), Icon::End);
        assert_eq!(kind_icon(&cat, "var.set"), Icon::Variable);
        assert_eq!(kind_icon(&cat, "mystery"), Icon::Flow);
        assert_eq!(family_of(Icon::Wait), Family::Wait);
    }
}
