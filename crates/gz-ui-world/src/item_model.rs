//! What the Items tab's form shows, worked out from what the file STORES (`gz_items::Item`),
//! what the item resolves to (`gz_items::Resolution`) and the linked game's item catalog: one
//! pure model the frames, `items.view` and the tests read.
//!
//! No key, family, group or scope is named here: the rows are the catalog's keys of the item's
//! family, grouped as the catalog groups them, in its order; a key the item sets that the
//! catalog lacks is an [`OtherRow`]. An edit changes the model's [`Changes`] through
//! [`set_own`], [`reset`], [`take_away`] and [`add_member`] / [`drop_member`]; what reaches the
//! file is the difference a save writes.

use gz_items::catalog::{self, Catalog, KeyType};
use gz_items::{Changes, Issue, Item, Kind, Resolution, SectionView, SetValue};
use gz_luadata::Seg;

/// What the item itself says of a key.
#[derive(Clone, Debug, PartialEq)]
pub enum Own {
    /// Nothing: the value is the base's, the game's or the module's.
    Inherit,
    Set(SetValue),
    /// Taken away (`unset`).
    Unset,
    /// Members added to or taken out of the list the key holds (a change).
    Members {
        add: Vec<String>,
        drop: Vec<String>,
    },
}

impl Own {
    /// Whether the item says anything of the key.
    #[must_use]
    pub fn is_own(&self) -> bool {
        !matches!(self, Own::Inherit)
    }

    /// The word `items.view` answers.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Own::Inherit => "inherit",
            Own::Set(_) => "set",
            Own::Unset => "unset",
            Own::Members { .. } => "members",
        }
    }
}

/// A row of a key the catalog knows.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The catalog scope of the section it is a key of.
    pub scope: String,
    pub key: String,
    /// The catalog's title; the key itself when it has none.
    pub title: String,
    /// The key stands after the title: a series' member, or one of two rows of a group that
    /// share a title.
    pub keyed: bool,
    pub ty: KeyType,
    pub unit: String,
    pub help: String,
    /// A token's words.
    pub values: Vec<String>,
    /// On the surface rather than under its group's More.
    pub common: bool,
    /// The game stops without it.
    pub required: bool,
    /// Edited as members of a list.
    pub list: bool,
    pub own: Own,
    /// What the section holds once the item is applied; `None`: the key is not there.
    pub resolved: Option<String>,
    /// Where `resolved` comes from: `this`, `base`, `game`, `module`, `item`; `none` without one.
    pub from: &'static str,
    /// The base the value comes through.
    pub base: Option<String>,
    /// A change: the game's value before it.
    pub before: Option<String>,
}

impl Row {
    /// The row's label in English: the title, with the key after it where rows share one.
    #[must_use]
    pub fn label(&self) -> String {
        if self.keyed { format!("{} ({})", self.title, self.key) } else { self.title.clone() }
    }

    /// The value the field shows: the item's own, else what the section holds.
    #[must_use]
    pub fn shown(&self) -> Option<String> {
        match &self.own {
            Own::Set(v) => Some(v.ltx()),
            Own::Unset => None,
            _ => self.resolved.clone(),
        }
    }
}

/// The rows of one group of the catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub id: String,
    pub title: String,
    pub rows: Vec<Row>,
}

/// A key the item says something of that the catalog lacks.
#[derive(Clone, Debug, PartialEq)]
pub struct OtherRow {
    pub scope: String,
    pub key: String,
    pub own: Own,
}

/// The form of one item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Form {
    pub family: Option<String>,
    /// The catalog's title of the family; empty without one.
    pub family_title: String,
    pub class: Option<String>,
    /// The groups of the item's own section that have a row, in the catalog's order.
    pub groups: Vec<Group>,
    /// The HUD section's name and its groups, when the item resolves one.
    pub hud: Option<(String, Vec<Group>)>,
    pub other: Vec<OtherRow>,
}

fn from_word(f: gz_items::From) -> &'static str {
    match f {
        gz_items::From::This => "this",
        gz_items::From::Base => "base",
        gz_items::From::Game => "game",
        gz_items::From::Module => "module",
        gz_items::From::Item => "item",
    }
}

/// What the item says of `key` in `changes`.
#[must_use]
pub fn own_of(changes: &Changes, key: &str) -> Own {
    if let Some(v) = changes.get(key) {
        return Own::Set(v.clone());
    }
    if changes.unset.iter().any(|k| k == key) {
        return Own::Unset;
    }
    let pick = |list: &[(String, Vec<String>)]| -> Vec<String> {
        list.iter().filter(|(k, _)| k == key).flat_map(|(_, m)| m.iter().cloned()).collect()
    };
    let (add, drop) = (pick(&changes.add), pick(&changes.drop));
    if add.is_empty() && drop.is_empty() { Own::Inherit } else { Own::Members { add, drop } }
}

/// The keys `changes` names, in the order it names them.
fn named_keys(changes: &Changes) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let all = changes
        .set
        .iter()
        .map(|(k, _)| k)
        .chain(changes.unset.iter())
        .chain(changes.add.iter().map(|(k, _)| k))
        .chain(changes.drop.iter().map(|(k, _)| k));
    for k in all {
        if !out.contains(k) {
            out.push(k.clone());
        }
    }
    out
}

/// The groups of one scope: a row per catalog key of the family (a series' members as the
/// section or the item has them), each under its catalog group; the keys the catalog lacks go to
/// `other`.
#[allow(clippy::too_many_arguments)]
fn scope_groups(
    catalog: &Catalog,
    scope: &str,
    family: Option<&str>,
    class: Option<&str>,
    changes: &Changes,
    view: Option<&SectionView>,
    before: Option<&SectionView>,
    other: &mut Vec<OtherRow>,
) -> Vec<Group> {
    let mut groups: Vec<Group> = catalog
        .groups
        .iter()
        .map(|g| Group { id: g.id.clone(), title: g.title.clone(), rows: Vec::new() })
        .collect();
    let own_keys = named_keys(changes);
    let present: Vec<&str> =
        view.map(|v| v.rows.iter().map(|r| r.key.as_str()).collect()).unwrap_or_default();
    let mut row_of = |info: &gz_items::KeyInfo, key: &str, series: bool| {
        let resolved = view.and_then(|v| v.get(key));
        let own = own_of(changes, key);
        let title = if info.title.is_empty() { key.to_owned() } else { info.title.clone() };
        let keyed = series && title != key;
        let row = Row {
            scope: scope.to_owned(),
            key: key.to_owned(),
            title,
            keyed,
            ty: info.ty.clone(),
            unit: info.unit.clone(),
            help: info.help.clone(),
            values: info.values.clone(),
            common: info.common,
            required: info.required_for(class),
            list: info.ty == KeyType::List || matches!(own, Own::Members { .. }),
            own,
            resolved: resolved.map(|r| r.value.clone().unwrap_or_default()),
            from: resolved.map_or("none", |r| from_word(r.from)),
            base: resolved.and_then(|r| r.base.clone()),
            before: before.and_then(|b| b.get(key)).map(|r| r.value.clone().unwrap_or_default()),
        };
        let at = match groups.iter().position(|g| g.id.eq_ignore_ascii_case(&info.group)) {
            Some(at) => at,
            None => {
                // a key of no group of the catalog's: a group of its own after the catalog's
                let title = catalog.scope(scope).map_or_else(|| scope.to_owned(), |s| s.title.clone());
                match groups.iter().position(|g| g.id.is_empty()) {
                    Some(at) => at,
                    None => {
                        groups.push(Group { id: String::new(), title, rows: Vec::new() });
                        groups.len() - 1
                    }
                }
            }
        };
        if let Some(g) = groups.get_mut(at) {
            g.rows.push(row);
        }
    };
    let infos: Vec<&gz_items::KeyInfo> = catalog.keys_of(scope, family).collect();
    let plain = |key: &str| infos.iter().any(|k| k.pattern.is_none() && k.key == key);
    let mut claimed: Vec<String> = Vec::new();
    for info in &infos {
        match &info.pattern {
            None => {
                row_of(info, &info.key, false);
                claimed.push(info.key.clone());
            }
            Some(p) => {
                // a series: the members the section or the item has
                let mut members: Vec<&str> = present
                    .iter()
                    .copied()
                    .chain(own_keys.iter().map(String::as_str))
                    .filter(|k| p.matches(k) && !plain(k) && !claimed.iter().any(|c| c == k))
                    .collect();
                members.sort_unstable();
                members.dedup();
                for key in members {
                    row_of(info, key, true);
                    claimed.push(key.to_owned());
                }
            }
        }
    }
    for key in own_keys {
        if catalog.key(scope, family, &key).is_none() {
            other.push(OtherRow { scope: scope.to_owned(), own: own_of(changes, &key), key });
        }
    }
    groups.retain(|g| !g.rows.is_empty());
    // a row goes by its title: two of one title in a group say their keys
    for g in &mut groups {
        let titles: Vec<String> = g.rows.iter().map(|r| r.title.clone()).collect();
        for r in &mut g.rows {
            if !r.keyed && r.title != r.key && titles.iter().filter(|t| **t == r.title).count() > 1 {
                r.keyed = true;
            }
        }
    }
    groups
}

/// The form of `item` as the file stores it, over what it resolves to (`res`, `None` while it is
/// worked out) and the catalog.
#[must_use]
pub fn build(item: &Item, res: Option<&Resolution>, catalog: &Catalog) -> Form {
    let family = res.and_then(|r| r.family.clone());
    let class = res.and_then(|r| r.class.clone());
    let mut other = Vec::new();
    let groups = scope_groups(
        catalog,
        catalog::ITEM,
        family.as_deref(),
        class.as_deref(),
        &item.changes,
        res.map(|r| &r.main),
        res.and_then(|r| r.before.as_ref()),
        &mut other,
    );
    let empty = Changes::default();
    let hud = res.and_then(|r| r.hud.as_ref()).map(|h| {
        let changes = item.hud.as_ref().map_or(&empty, |x| &x.changes);
        let groups = scope_groups(
            catalog,
            catalog::HUD,
            family.as_deref(),
            class.as_deref(),
            changes,
            Some(h),
            res.and_then(|r| r.hud_before.as_ref()),
            &mut other,
        );
        (h.name.clone(), groups)
    });
    let family_title =
        family.as_deref().and_then(|f| catalog.family(f)).map(|f| f.title.clone()).unwrap_or_default();
    Form { family, family_title, class, groups, hud, other }
}

// ---- edits -------------------------------------------------------------------------------------

/// Takes everything the item says of `key` away: its value comes from the base again.
pub fn reset(changes: &mut Changes, key: &str) {
    changes.set.retain(|(k, _)| k != key);
    changes.unset.retain(|k| k != key);
    changes.add.retain(|(k, _)| k != key);
    changes.drop.retain(|(k, _)| k != key);
}

/// The item sets `key` to `value`.
pub fn set_own(changes: &mut Changes, key: &str, value: SetValue) {
    changes.unset.retain(|k| k != key);
    changes.add.retain(|(k, _)| k != key);
    changes.drop.retain(|(k, _)| k != key);
    match changes.set.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => changes.set.push((key.to_owned(), value)),
    }
}

/// The item takes `key` away (`unset`).
pub fn take_away(changes: &mut Changes, key: &str) {
    reset(changes, key);
    changes.unset.push(key.to_owned());
}

/// The members of a list value as the game reads it.
#[must_use]
pub fn members_of(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|m| !m.is_empty()).map(str::to_owned).collect()
}

/// The members the list holds once the item is applied, worked out at once from what the item
/// says: its own list, or the list it starts from (`start`) with its additions and drops.
#[must_use]
pub fn members_now(own: &Own, start: Option<&str>) -> Vec<String> {
    match own {
        Own::Set(v) => members_of(&v.ltx()),
        Own::Unset => Vec::new(),
        Own::Inherit => start.map(members_of).unwrap_or_default(),
        Own::Members { add, drop } => {
            let mut list: Vec<String> = start.map(members_of).unwrap_or_default();
            list.retain(|m| !drop.iter().any(|d| d.eq_ignore_ascii_case(m)));
            for a in add {
                if !list.iter().any(|m| m.eq_ignore_ascii_case(a)) {
                    list.push(a.clone());
                }
            }
            list
        }
    }
}

fn members_mut<'a>(list: &'a mut Vec<(String, Vec<String>)>, key: &str) -> &'a mut Vec<String> {
    if let Some(at) = list.iter().position(|(k, _)| k == key) {
        return &mut list[at].1;
    }
    list.push((key.to_owned(), Vec::new()));
    let last = list.len() - 1;
    &mut list[last].1
}

/// Adds `member` to the list `key` holds. A change writes it under `add` (or takes it off
/// `drop`); a new item writes the whole list under `set` (`now` is the list as it holds it).
pub fn add_member(changes: &mut Changes, key: &str, member: &str, change: bool, now: &[String]) {
    let member = member.trim();
    if member.is_empty() || now.iter().any(|m| m.eq_ignore_ascii_case(member)) {
        return;
    }
    if change && changes.get(key).is_none() {
        let dropped = members_mut(&mut changes.drop, key);
        let was = dropped.len();
        dropped.retain(|m| !m.eq_ignore_ascii_case(member));
        if dropped.len() == was {
            members_mut(&mut changes.add, key).push(member.to_owned());
        }
        changes.drop.retain(|(_, m)| !m.is_empty());
        changes.unset.retain(|k| k != key);
        return;
    }
    let mut list: Vec<SetValue> = now.iter().map(|m| SetValue::Text(m.clone())).collect();
    list.push(SetValue::Text(member.to_owned()));
    set_own(changes, key, SetValue::List(list));
}

/// Takes `member` out of the list `key` holds, the way [`add_member`] adds one.
pub fn drop_member(changes: &mut Changes, key: &str, member: &str, change: bool, now: &[String]) {
    if change && changes.get(key).is_none() {
        let added = members_mut(&mut changes.add, key);
        let was = added.len();
        added.retain(|m| !m.eq_ignore_ascii_case(member));
        if added.len() == was {
            members_mut(&mut changes.drop, key).push(member.to_owned());
        }
        changes.add.retain(|(_, m)| !m.is_empty());
        return;
    }
    let list: Vec<SetValue> =
        now.iter().filter(|m| !m.eq_ignore_ascii_case(member)).map(|m| SetValue::Text(m.clone())).collect();
    set_own(changes, key, SetValue::List(list));
}

/// What a typed text is as a value of a key of type `ty`: a number for a number, a boolean for
/// a switch, numbers for a vector; anything else - or what does not read as the type - the text
/// as it is (the check says what is wrong with it).
#[must_use]
pub fn typed(ty: &KeyType, text: &str) -> SetValue {
    let t = text.trim();
    let number = |s: &str| s.trim().parse::<f64>().ok().filter(|n| n.is_finite());
    match ty {
        KeyType::Int | KeyType::Float => {
            number(t).map_or_else(|| SetValue::Text(t.to_owned()), SetValue::Number)
        }
        KeyType::Bool => match t.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => SetValue::Bool(true),
            "false" | "off" | "no" | "0" => SetValue::Bool(false),
            _ => SetValue::Text(t.to_owned()),
        },
        KeyType::Vec2 | KeyType::Vec3 | KeyType::Vec4 | KeyType::Color => {
            let parts: Option<Vec<SetValue>> =
                t.split(',').map(|p| number(p).map(SetValue::Number)).collect();
            match parts {
                Some(p) if p.len() > 1 => SetValue::List(p),
                _ => SetValue::Text(t.to_owned()),
            }
        }
        KeyType::List => SetValue::List(members_of(t).into_iter().map(SetValue::Text).collect()),
        _ => SetValue::Text(t.to_owned()),
    }
}

/// Whether a value reads as on.
#[must_use]
pub fn is_on(value: &str) -> bool {
    matches!(value.trim().trim_matches('"').to_ascii_lowercase().as_str(), "true" | "on" | "yes" | "1")
}

// ---- where a problem is ------------------------------------------------------------------------

/// What part of the form a problem is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// The item as a whole: under the header.
    Head,
    /// A field of the file's top level: `id`, `section`, `base`, `abstract`, `edit`, `name`,
    /// `short_name`, `description`, `icon`, `hud`.
    Field(String),
    /// A key of a section: the scope and the key.
    Key { scope: String, key: String },
    /// A part of where it is found or what else it holds: `loot`, `sold`, `craft`, `scene`,
    /// `sections`, `lists`, with the row.
    Part { name: String, row: Option<usize> },
}

fn seg_key(s: Option<&Seg>) -> Option<&str> {
    match s {
        Some(Seg::Key(k)) => Some(k.as_str()),
        _ => None,
    }
}

/// The first name a sentence quotes: `the game stops without 'cost' (...)` names `cost`.
fn quoted(message: &str) -> Option<&str> {
    message.split('\'').nth(1).filter(|s| !s.is_empty())
}

/// Where a problem of the check belongs on the form.
#[must_use]
pub fn place_of(issue: &Issue) -> Place {
    let p = &issue.path;
    let first = seg_key(p.first());
    // what the game stops without is put at the base: it is about the key nothing gives
    if issue.code == gz_items::validate::REQUIRED
        && let Some(key) = quoted(&issue.message)
    {
        let scope = if first == Some("hud") { catalog::HUD } else { catalog::ITEM };
        return Place::Key { scope: scope.to_owned(), key: key.to_owned() };
    }
    let change_key = |rest: &[Seg]| -> Option<String> {
        match (seg_key(rest.first()), seg_key(rest.get(1))) {
            (Some("set" | "unset" | "add" | "drop"), Some(k)) => Some(k.to_owned()),
            _ => None,
        }
    };
    match first {
        None => Place::Head,
        Some("set" | "unset" | "add" | "drop") => match change_key(p) {
            Some(key) => Place::Key { scope: catalog::ITEM.to_owned(), key },
            None => Place::Head,
        },
        Some("hud") => match change_key(p.get(1..).unwrap_or(&[])) {
            Some(key) => Place::Key { scope: catalog::HUD.to_owned(), key },
            None => Place::Field("hud".to_owned()),
        },
        Some(part @ ("loot" | "sold" | "craft" | "scene" | "sections" | "lists")) => {
            let row = match p.get(1) {
                Some(Seg::Index(i)) => Some(*i),
                _ => None,
            };
            Place::Part { name: part.to_owned(), row }
        }
        Some(field) => Place::Field(field.to_owned()),
    }
}

/// The sentence a problem shows: the domain's message with a capital and a period; the code
/// stays in MCP and the build.
#[must_use]
pub fn sentence(issue: &Issue) -> String {
    let mut s = issue.message.trim().to_owned();
    // a file that does not read names itself first: the form is about that file already
    if issue.code == gz_items::roster::UNREADABLE
        && let Some(at) = s.find(".itemasset: ")
    {
        s = s.get(at + ".itemasset: ".len()..).unwrap_or("").to_owned();
    }
    if let Some(first) = s.chars().next()
        && first.is_lowercase()
    {
        let up: String = first.to_uppercase().collect();
        s.replace_range(..first.len_utf8(), &up);
    }
    if !s.is_empty() && !s.ends_with(['.', '!', '?']) {
        s.push('.');
    }
    s
}

/// Whether the item is a change of one that exists.
#[must_use]
pub fn is_change(item: &Item) -> bool {
    item.kind() == Kind::Change
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = "\
[catalog]\nversion = 1\n[scopes]\norder = item, hud\n[families]\norder = weapon\n\
[family.weapon]\ntitle = Firearm\nclasses = WP_AK74\n\
[groups]\norder = general, economy\n[group.general]\ntitle = General\n[group.economy]\ntitle = Economy\n\
[item.cost]\nfamilies = *\ntype = int\ngroup = economy\ntitle = Cost\nunit = RU\ncommon = true\nrequired = true\n\
[item.ammo_class]\nfamilies = weapon\ntype = list\ngroup = general\ntitle = Ammunition\n\
[item.snd_shoot]\nfamilies = weapon\ntype = sound\ngroup = general\ntitle = Shot\npattern = snd_shoot<N>\n\
[item.medkit_only]\nfamilies = nothing\ntype = int\ngroup = general\n\
[hud.hands_position]\nfamilies = weapon\ntype = vec3\ngroup = general\ntitle = Hands\n";

    fn view(name: &str, rows: &[(&str, &str, gz_items::From)]) -> SectionView {
        SectionView {
            name: name.into(),
            exists: true,
            parents: Vec::new(),
            rows: rows
                .iter()
                .map(|(k, v, f)| gz_items::Row {
                    key: (*k).into(),
                    value: Some((*v).into()),
                    from: *f,
                    base: None,
                    file: String::new(),
                    line: 0,
                })
                .collect(),
        }
    }

    fn res() -> Resolution {
        Resolution {
            family: Some("weapon".into()),
            class: Some("WP_AK74".into()),
            main: view(
                "wpn_ak74",
                &[
                    ("cost", "1200", gz_items::From::This),
                    ("ammo_class", "ammo_a, ammo_b", gz_items::From::Game),
                    ("snd_shoot1", "a\\b", gz_items::From::Game),
                ],
            ),
            before: Some(view("wpn_ak74", &[("cost", "900", gz_items::From::Game)])),
            hud: Some(view("wpn_ak74_hud", &[("hands_position", "0, 0, 0", gz_items::From::Game)])),
            ..Resolution::default()
        }
    }

    #[test]
    fn the_rows_are_the_catalogs_keys_of_the_family_under_its_groups() {
        let catalog = Catalog::parse(CATALOG);
        let mut item = Item { edit: Some("wpn_ak74".into()), ..Item::default() };
        item.changes.set.push(("cost".into(), SetValue::Number(1200.0)));
        item.changes.set.push(("my_key".into(), SetValue::Text("x".into())));
        let f = build(&item, Some(&res()), &catalog);
        assert_eq!(f.family_title, "Firearm");
        let ids: Vec<&str> = f.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["general", "economy"]);
        let general: Vec<&str> = f.groups[0].rows.iter().map(|r| r.key.as_str()).collect();
        // the series shows the member the section has; a key of another family is no row
        assert_eq!(general, ["ammo_class", "snd_shoot1"]);
        assert_eq!(f.groups[0].rows[1].label(), "Shot (snd_shoot1)");
        let cost = &f.groups[1].rows[0];
        assert!(cost.common && cost.required && cost.own.is_own());
        assert_eq!(
            (cost.shown().as_deref(), cost.before.as_deref(), cost.from),
            (Some("1200"), Some("900"), "this")
        );
        assert!(f.groups[0].rows[0].list);
        assert_eq!(
            f.other,
            [OtherRow {
                scope: "item".into(),
                key: "my_key".into(),
                own: Own::Set(SetValue::Text("x".into()))
            }]
        );
        let (hud, groups) = f.hud.as_ref().unwrap();
        assert_eq!(hud, "wpn_ak74_hud");
        assert_eq!(groups[0].rows[0].resolved.as_deref(), Some("0, 0, 0"));
    }

    #[test]
    fn a_change_adds_and_drops_members_and_a_new_item_writes_the_list() {
        let mut c = Changes::default();
        let now = members_now(&Own::Inherit, Some("ammo_a, ammo_b"));
        add_member(&mut c, "ammo_class", "ammo_c", true, &now);
        drop_member(&mut c, "ammo_class", "ammo_a", true, &now);
        assert_eq!(c.add, [("ammo_class".to_owned(), vec!["ammo_c".to_owned()])]);
        assert_eq!(c.drop, [("ammo_class".to_owned(), vec!["ammo_a".to_owned()])]);
        assert_eq!(members_now(&own_of(&c, "ammo_class"), Some("ammo_a, ammo_b")), ["ammo_b", "ammo_c"]);
        // taking back what it added leaves nothing to add
        drop_member(&mut c, "ammo_class", "ammo_c", true, &now);
        assert!(c.add.is_empty());
        let mut n = Changes::default();
        add_member(&mut n, "ammo_class", "ammo_c", false, &now);
        assert_eq!(n.get("ammo_class").map(SetValue::ltx).as_deref(), Some("ammo_a, ammo_b, ammo_c"));
        take_away(&mut n, "ammo_class");
        assert_eq!(own_of(&n, "ammo_class"), Own::Unset);
        reset(&mut n, "ammo_class");
        assert_eq!(own_of(&n, "ammo_class"), Own::Inherit);
    }

    #[test]
    fn a_typed_value_takes_its_keys_type() {
        assert_eq!(typed(&KeyType::Int, " 12 "), SetValue::Number(12.0));
        assert_eq!(typed(&KeyType::Float, "heavy"), SetValue::Text("heavy".into()));
        assert_eq!(typed(&KeyType::Bool, "On"), SetValue::Bool(true));
        assert_eq!(typed(&KeyType::Vec3, "0.1, 0, -2").ltx(), "0.1, 0, -2");
        assert_eq!(typed(&KeyType::Sound, "a\\b, 1"), SetValue::Text("a\\b, 1".into()));
        assert!(is_on("TRUE") && !is_on("0"));
    }

    #[test]
    fn a_problem_finds_its_row() {
        let at = |code: &'static str, path: Vec<Seg>, msg: &str| place_of(&Issue::warning(code, path, msg));
        assert_eq!(
            at("I412", vec![Seg::key("set"), Seg::key("cost")], "x"),
            Place::Key { scope: "item".into(), key: "cost".into() }
        );
        assert_eq!(
            at("I419", vec![Seg::key("hud"), Seg::key("set"), Seg::key("anm_idle")], "x"),
            Place::Key { scope: "hud".into(), key: "anm_idle".into() }
        );
        assert_eq!(
            at(gz_items::validate::REQUIRED, vec![Seg::key("base")], "the game stops without 'cost' (Cost)"),
            Place::Key { scope: "item".into(), key: "cost".into() }
        );
        assert_eq!(at("I421", vec![Seg::key("icon")], "x"), Place::Field("icon".into()));
        assert_eq!(
            at("I423", vec![Seg::key("craft"), Seg::Index(1), Seg::key("at")], "x"),
            Place::Part { name: "craft".into(), row: Some(1) }
        );
        assert_eq!(at("I401", Vec::new(), "x"), Place::Head);
        assert_eq!(
            sentence(&Issue::warning("I412", Vec::new(), "'cost' is \"x\": it takes a number")),
            "'cost' is \"x\": it takes a number."
        );
        assert_eq!(sentence(&Issue::warning("I425", Vec::new(), "a community")), "A community.");
    }
}
