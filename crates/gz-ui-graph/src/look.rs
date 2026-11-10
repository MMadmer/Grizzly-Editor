//! How the graph names and colours a kind (`NqLook`): shared by the canvas and the panels, and
//! aware of the document - a behaviour graph never says "quest".

use egui::Color32;
use gz_nq::Catalog;
use gz_ui::{Icon, Palette};

/// The family a kind belongs to: what colours its strip and its icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Family {
    #[default]
    Muted,
    Trigger,
    End,
    Flow,
    Wait,
    Dialog,
    Task,
    Role,
    Item,
}

/// The icon of a kind (`NqLook::KindIcon`): its catalog group, else its name's prefix.
#[must_use]
pub fn kind_icon(cat: &Catalog, kind: &str) -> Icon {
    if kind.is_empty() {
        return Icon::Dot;
    }
    match kind {
        "flow.end" => return Icon::End,
        "behaviour.run" => return Icon::Behaviour,
        "lua" => return Icon::FileScript,
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
    const PREFIXES: [(&str, Icon); 15] = [
        ("flow.", Icon::Flow),
        ("wait.", Icon::Wait),
        ("dialog.", Icon::Dialog),
        ("objective.", Icon::Task),
        ("task.", Icon::Task),
        ("var.", Icon::Variable),
        ("item.", Icon::Item),
        ("money.", Icon::Money),
        ("spawn.", Icon::SpawnNpc),
        ("npc.", Icon::Role),
        ("order.", Icon::Order),
        ("relation.", Icon::Relation),
        ("squad.", Icon::Squad),
        ("pda.", Icon::PdaMsg),
        ("actor.", Icon::Person),
    ];
    PREFIXES.iter().find(|(p, _)| kind.starts_with(p)).map_or(Icon::Dot, |(_, i)| *i)
}

/// The family of an icon (`NqLook::FamilyColor`'s switch).
#[must_use]
pub fn family_of(icon: Icon) -> Family {
    match icon {
        Icon::Trigger => Family::Trigger,
        Icon::End => Family::End,
        Icon::Flow => Family::Flow,
        Icon::Wait => Family::Wait,
        Icon::Dialog => Family::Dialog,
        Icon::Task | Icon::Quest | Icon::PdaMsg => Family::Task,
        Icon::Role | Icon::Behaviour | Icon::Order | Icon::Squad | Icon::Relation | Icon::Person => Family::Role,
        Icon::Item | Icon::Money | Icon::SpawnNpc | Icon::World => Family::Item,
        _ => Family::Muted,
    }
}

/// The colour of a family, at `alpha` of its own.
#[must_use]
pub fn family_color(p: &Palette, f: Family, alpha: f32) -> Color32 {
    let c = match f {
        Family::Muted => p.text_muted,
        Family::Trigger => p.success,
        Family::End => p.error,
        Family::Flow => p.text_secondary,
        Family::Wait => p.warning,
        Family::Dialog => p.axis_z,
        Family::Task => p.required,
        Family::Role => p.accent_bright,
        Family::Item => p.accent,
    };
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

/// "quest" or "graph" in a sentence (`NqLook::DocNoun`).
#[must_use]
pub fn doc_noun(behaviour: bool) -> &'static str {
    gz_nq::text::doc_noun(behaviour)
}

/// `spot_task` -> `Spot task`: a catalog name read as a label (`Pretty`).
#[must_use]
pub fn pretty(name: &str) -> String {
    const WORDS: [(&str, &str); 6] = [("id", "ID"), ("npc", "NPC"), ("npcs", "NPCs"), ("pda", "PDA"), ("lua", "Lua"), ("ui", "UI")];
    let mut out = String::with_capacity(name.len() + 4);
    for w in name.split('_').filter(|w| !w.is_empty()) {
        let mut word = WORDS.iter().find(|(from, _)| *from == w).map_or_else(|| w.to_owned(), |(_, to)| (*to).to_owned());
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

/// `%g` of the C++: the shortest plain form of a number.
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
        assert_eq!(kind_icon(&cat, "mystery"), Icon::Dot);
        assert_eq!(family_of(Icon::Wait), Family::Wait);
    }
}
