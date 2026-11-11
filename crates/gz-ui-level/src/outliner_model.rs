//! What the World Outliner lists, as a pure function of a scene snapshot and the panel's
//! filter (`UIWorldOutliner::Rebuild` / `ApplyFilter` / `BuildHierarchy` /
//! `ApplyHierarchyFilter` / `BuildHierarchyRows`).
//!
//! The rows are computed off the UI thread, keyed by the scene's revision and the filter; the
//! panel lays out only the rows that show.

use std::cmp::Ordering;

use gz_base::SmolStr;
use gz_scene::{ObjClass, ObjId, SceneState};
use rustc_hash::{FxHashMap, FxHashSet};

/// The visibility filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visibility {
    #[default]
    All,
    Visible,
    Hidden,
}

impl Visibility {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Visibility::All => "all",
            Visibility::Visible => "visible",
            Visibility::Hidden => "hidden",
        }
    }

    #[must_use]
    pub fn parse(t: &str) -> Option<Self> {
        match t.to_ascii_lowercase().as_str() {
            "all" => Some(Visibility::All),
            "visible" => Some(Visibility::Visible),
            "hidden" => Some(Visibility::Hidden),
            _ => None,
        }
    }
}

/// The order of the objects in a class (the Filters menu's sort; the header does not sort).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sort {
    #[default]
    Scene,
    Az,
    Za,
}

impl Sort {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Sort::Scene => "scene",
            Sort::Az => "az",
            Sort::Za => "za",
        }
    }

    #[must_use]
    pub fn parse(t: &str) -> Option<Self> {
        match t.to_ascii_lowercase().as_str() {
            "scene" => Some(Sort::Scene),
            "az" => Some(Sort::Az),
            "za" => Some(Sort::Za),
            _ => None,
        }
    }
}

/// The search box and the Filters menu.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Filter {
    pub text: String,
    pub selected_only: bool,
    pub visibility: Visibility,
    pub sort: Sort,
    /// The classes switched off (types are hidden rather than shown, so a class the scene
    /// grows later is not silently left out).
    pub hidden: Vec<ObjClass>,
    /// By Hierarchy (objects under the group that owns them) instead of By Type.
    pub hierarchy: bool,
}

impl Filter {
    #[must_use]
    pub fn class_hidden(&self, c: ObjClass) -> bool {
        self.hidden.contains(&c)
    }

    /// `Filtering()`: something narrows what shows.
    #[must_use]
    pub fn filtering(&self) -> bool {
        !parse_terms(&self.text).is_empty() || self.selected_only || self.visibility != Visibility::All
    }

    /// The badge on Filters: how many of its switches are on (`ActiveFilters`).
    #[must_use]
    pub fn active_filters(&self, any_hidden: bool) -> usize {
        usize::from(self.selected_only)
            + usize::from(self.visibility != Visibility::All)
            + usize::from(any_hidden)
    }
}

/// What the panel folds: the classes folded By Type, the groups open By Hierarchy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Folds {
    pub collapsed: Vec<ObjClass>,
    /// `(class, name)` of the open groups: names survive a reload, ids do not.
    pub expanded: Vec<(ObjClass, SmolStr)>,
}

/// One search term (`STerm`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub text: String,
    pub exclude: bool,
}

/// `ParseFilter`: words AND-ed, `-word` excluded, `"two words"` kept together; lower case.
#[must_use]
pub fn parse_terms(filter: &str) -> Vec<Term> {
    let low = filter.to_lowercase();
    let b = low.as_str();
    let mut out = Vec::new();
    let mut rest = b;
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            break;
        }
        let mut exclude = false;
        // a bare '-' is someone mid-typing, not an empty exclusion
        if let Some(after) = rest.strip_prefix('-')
            && !after.is_empty()
            && !after.starts_with(' ')
        {
            exclude = true;
            rest = after;
        }
        let text;
        if let Some(after) = rest.strip_prefix('"') {
            match after.find('"') {
                Some(e) => {
                    text = after.get(..e).unwrap_or("").to_owned();
                    rest = after.get(e + 1..).unwrap_or("");
                }
                None => {
                    text = after.to_owned();
                    rest = "";
                }
            }
        } else {
            let e = rest.find([' ', '\t']).unwrap_or(rest.len());
            text = rest.get(..e).unwrap_or("").to_owned();
            rest = rest.get(e..).unwrap_or("");
        }
        if !text.is_empty() {
            out.push(Term { text, exclude });
        }
    }
    out
}

/// `NameMatches`.
#[must_use]
pub fn name_matches(terms: &[Term], name: &str) -> bool {
    if terms.is_empty() {
        return true;
    }
    let low = name.to_lowercase();
    terms.iter().all(|t| low.contains(&t.text) != t.exclude)
}

/// `NaturalCompare`: digit runs compare as numbers (leading zeros break ties), the rest
/// without case.
#[must_use]
pub fn natural_compare(lhs: &str, rhs: &str) -> Ordering {
    let (a, b) = (lhs.as_bytes(), rhs.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let (ca, cb) = (a[i], b[j]);
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let (ia, jb) = (i, j);
            while i < a.len() && a[i] == b'0' {
                i += 1;
            }
            while j < b.len() && b[j] == b'0' {
                j += 1;
            }
            let (mut ea, mut eb) = (i, j);
            while ea < a.len() && a[ea].is_ascii_digit() {
                ea += 1;
            }
            while eb < b.len() && b[eb].is_ascii_digit() {
                eb += 1;
            }
            let (da, db) = (ea - i, eb - j);
            if da != db {
                return da.cmp(&db);
            }
            match a[i..ea].cmp(&b[j..eb]) {
                Ordering::Equal => {}
                o => return o,
            }
            let (wa, wb) = (ea - ia, eb - jb);
            if wa != wb {
                return wa.cmp(&wb);
            }
            i = ea;
            j = eb;
            continue;
        }
        let (la, lb) = (ca.to_ascii_lowercase(), cb.to_ascii_lowercase());
        if la != lb {
            return la.cmp(&lb);
        }
        i += 1;
        j += 1;
    }
    if i < a.len() {
        Ordering::Greater
    } else if j < b.len() {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// One row the panel lays out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A class's row (By Type): its eye, its title and `shown of total` while filtering.
    Class { class: ObjClass, shown: usize, total: usize, open: bool },
    /// An object. By Hierarchy, `depth` is its nesting, `direct` says it matched itself (or is
    /// only there for a match under it) and `children` whether it has rows under it.
    Object { id: ObjId, class: ObjClass, depth: u16, direct: bool, children: bool, expanded: bool },
}

/// A class of the scene, as the Filters menu lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub class: ObjClass,
    pub total: usize,
    pub shown: usize,
}

/// What the panel draws, and what `outliner_filter` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub rows: Vec<Row>,
    /// The classes with objects, sorted by class name.
    pub groups: Vec<Group>,
    /// Objects that pass the filter, in classes not switched off.
    pub shown: usize,
    pub total: usize,
    pub selected: usize,
    /// The last selected object in the scene's order, and a signature of the selected set.
    pub last_selected: Option<ObjId>,
    pub selection_sig: u64,
}

impl Listing {
    /// The row of an object, when it shows.
    #[must_use]
    pub fn row_of(&self, id: ObjId) -> Option<usize> {
        self.rows.iter().position(|r| matches!(r, Row::Object { id: i, .. } if *i == id))
    }
}

/// Computes the rows.
#[must_use]
pub fn compute(state: &SceneState, filter: &Filter, folds: &Folds) -> Listing {
    let terms = parse_terms(&filter.text);
    let filtering = filter.filtering();
    let mut classes: Vec<ObjClass> =
        ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool() && state.count(*c) > 0).collect();
    classes.sort_by_key(|c| c.name());

    let mut out = Listing::default();
    // per class: every object, and the ones the filter keeps
    let mut lists: Vec<(ObjClass, Vec<ObjId>, Vec<ObjId>)> = Vec::with_capacity(classes.len());
    let mut sig: u64 = 0xcbf2_9ce4_8422_2325;
    for c in classes {
        let all: Vec<ObjId> = state.by_class[c.index()].iter().copied().collect();
        let mut shown: Vec<ObjId> = Vec::new();
        for id in &all {
            let Some(o) = state.get(*id) else { continue };
            let selected = state.is_selected(*id);
            if selected {
                out.selected += 1;
                out.last_selected = Some(*id);
                sig = (sig ^ u64::from(id.0)).wrapping_mul(0x100_0000_01b3);
            }
            if filter.selected_only && !selected {
                continue;
            }
            match filter.visibility {
                Visibility::Visible if !o.flags.visible => continue,
                Visibility::Hidden if o.flags.visible => continue,
                _ => {}
            }
            if !name_matches(&terms, &o.name) {
                continue;
            }
            shown.push(*id);
        }
        if filter.sort != Sort::Scene {
            let name = |id: &ObjId| state.get(*id).map(|o| o.name.clone()).unwrap_or_default();
            shown.sort_by(|a, b| {
                let o = natural_compare(&name(a), &name(b));
                if filter.sort == Sort::Az { o } else { o.reverse() }
            });
        }
        out.total += all.len();
        if !filter.class_hidden(c) {
            out.shown += shown.len();
        }
        out.groups.push(Group { class: c, total: all.len(), shown: shown.len() });
        lists.push((c, all, shown));
    }
    out.selection_sig = sig;

    if filter.hierarchy {
        out.rows = hierarchy_rows(state, filter, folds, &lists, filtering);
        return out;
    }
    for (c, all, shown) in &lists {
        if filter.class_hidden(*c) || (filtering && shown.is_empty()) {
            continue;
        }
        let open = filtering || !folds.collapsed.contains(c);
        out.rows.push(Row::Class { class: *c, shown: shown.len(), total: all.len(), open });
        if open {
            out.rows.extend(shown.iter().map(|id| Row::Object {
                id: *id,
                class: *c,
                depth: 0,
                direct: true,
                children: false,
                expanded: false,
            }));
        }
    }
    out
}

/// By Hierarchy: every object once, under the group that owns it; a match keeps its owners in
/// view; a broken owner chain degrades to roots.
fn hierarchy_rows(
    state: &SceneState,
    filter: &Filter,
    folds: &Folds,
    lists: &[(ObjClass, Vec<ObjId>, Vec<ObjId>)],
    filtering: bool,
) -> Vec<Row> {
    let mut nodes: Vec<ObjId> = Vec::new();
    let mut index: FxHashMap<ObjId, usize> = FxHashMap::default();
    for (_, all, _) in lists {
        for id in all {
            if !index.contains_key(id) {
                index.insert(*id, nodes.len());
                nodes.push(*id);
            }
        }
    }
    let n = nodes.len();
    let mut parent: Vec<Option<usize>> = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| {
            state.get(*id).and_then(|o| o.owner).and_then(|ow| index.get(&ow).copied()).filter(|p| *p != i)
        })
        .collect();
    // cycles: the lowest index of each loop becomes a root
    let mut mark = vec![0u8; n];
    for start in 0..n {
        if mark[start] != 0 {
            continue;
        }
        let mut path = Vec::new();
        let mut cur = Some(start);
        while let Some(c) = cur {
            if mark[c] != 0 {
                break;
            }
            mark[c] = 1;
            path.push(c);
            cur = parent[c];
        }
        if let Some(c) = cur
            && mark[c] == 1
            && let Some(begin) = path.iter().position(|x| *x == c)
            && let Some(lowest) = path.get(begin..).and_then(|cycle| cycle.iter().min().copied())
        {
            parent[lowest] = None;
        }
        for x in path {
            mark[x] = 2;
        }
    }
    let mut direct = vec![false; n];
    for (c, _, shown) in lists {
        if filter.class_hidden(*c) {
            continue;
        }
        for id in shown {
            if let Some(&i) = index.get(id) {
                direct[i] = true;
            }
        }
    }
    let mut included = direct.clone();
    for i in 0..n {
        if !direct[i] {
            continue;
        }
        let mut p = parent[i];
        while let Some(x) = p {
            if included[x] {
                break;
            }
            included[x] = true;
            p = parent[x];
        }
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, up) in parent.iter().enumerate().take(n) {
        match *up {
            Some(p) => children[p].push(i),
            None => roots.push(i),
        }
    }
    if filter.sort != Sort::Scene {
        let name = |i: usize| state.get(nodes[i]).map(|o| o.name.clone()).unwrap_or_default();
        let less = |a: &usize, b: &usize| {
            let o = natural_compare(&name(*a), &name(*b));
            let o = if filter.sort == Sort::Az { o } else { o.reverse() };
            o.then(a.cmp(b))
        };
        roots.sort_by(less);
        for c in &mut children {
            c.sort_by(less);
        }
    }
    let open_set: FxHashSet<(ObjClass, &str)> =
        folds.expanded.iter().map(|(c, n)| (*c, n.as_str())).collect();
    let filtering = filtering || !filter.hidden.is_empty();
    let mut rows = Vec::new();
    let mut pending: Vec<(usize, u16)> = roots.iter().rev().map(|r| (*r, 0u16)).collect();
    while let Some((i, depth)) = pending.pop() {
        if !included[i] {
            continue;
        }
        let Some(o) = state.get(nodes[i]) else { continue };
        let has_children = children[i].iter().any(|c| included[*c]);
        let expanded = open_set.contains(&(o.class, o.name.as_str()));
        rows.push(Row::Object {
            id: nodes[i],
            class: o.class,
            depth,
            direct: direct[i],
            children: has_children,
            expanded: filtering || expanded,
        });
        if !has_children || (!filtering && !expanded) {
            continue;
        }
        for c in children[i].iter().rev() {
            if included[*c] {
                pending.push((*c, depth + 1));
            }
        }
    }
    rows
}

/// `McpSetFilter`'s type list: `;`-separated class names, every one a class of the scene.
/// Answers the classes to hide, or the error text naming the unknown ones and what the scene
/// has.
pub fn types_to_hide(groups: &[Group], types: &str) -> Result<Vec<ObjClass>, String> {
    if types.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: Vec<&str> = types.split(';').map(|t| t.trim_matches(' ')).filter(|t| !t.is_empty()).collect();
    let unknown: Vec<&str> = wanted
        .iter()
        .copied()
        .filter(|w| !groups.iter().any(|g| g.class.name().eq_ignore_ascii_case(w)))
        .collect();
    if !unknown.is_empty() {
        let has: Vec<&str> = groups.iter().map(|g| g.class.name()).collect();
        return Err(format!("unknown object type(s): {}; scene has: {}", unknown.join(", "), has.join(", ")));
    }
    Ok(groups
        .iter()
        .filter(|g| !wanted.iter().any(|w| g.class.name().eq_ignore_ascii_case(w)))
        .map(|g| g.class)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_scene::{ObjData, Object};

    fn scene(names: &[(&str, ObjClass)]) -> SceneState {
        let mut s = SceneState::default();
        for (n, c) in names {
            let data = match c {
                ObjClass::Light => ObjData::Light(gz_scene::objects::LightData::default()),
                _ => ObjData::Way(gz_scene::objects::WayData::default()),
            };
            s.insert(Object::new(*c, n, data));
        }
        s
    }

    #[test]
    fn the_search_grammar() {
        let t = parse_terms(r#"box -red "big one" -"#);
        assert_eq!(t.len(), 4);
        assert!(t[1].exclude && t[1].text == "red");
        assert_eq!(t[2].text, "big one");
        assert_eq!(t[3].text, "-");
        assert!(name_matches(&parse_terms("box -red"), "Box_Blue"));
        assert!(!name_matches(&parse_terms("box -red"), "box_red"));
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural_compare("a2", "a10"), Ordering::Less);
        assert_eq!(natural_compare("a02", "a2"), Ordering::Greater);
        assert_eq!(natural_compare("B", "a"), Ordering::Greater);
        assert_eq!(natural_compare("abc", "ab"), Ordering::Greater);
        assert_eq!(natural_compare("x", "x"), Ordering::Equal);
    }

    #[test]
    fn by_type_rows_and_counts() {
        let s = scene(&[("l1", ObjClass::Light), ("w1", ObjClass::Way), ("l2", ObjClass::Light)]);
        let l = compute(&s, &Filter::default(), &Folds::default());
        assert_eq!(l.total, 3);
        assert_eq!(
            l.groups.iter().map(|g| g.class).collect::<Vec<_>>(),
            vec![ObjClass::Light, ObjClass::Way]
        );
        assert_eq!(l.rows.len(), 5);
        let f = Filter { text: "l2".into(), ..Filter::default() };
        let l = compute(&s, &f, &Folds::default());
        assert_eq!(l.shown, 1);
        assert_eq!(l.rows.len(), 2, "the empty class vanishes while filtering");
        let folds = Folds { collapsed: vec![ObjClass::Light], expanded: Vec::new() };
        let l = compute(&s, &Filter::default(), &folds);
        assert_eq!(l.rows.len(), 3);
    }

    #[test]
    fn unknown_types_are_named() {
        let groups = vec![
            Group { class: ObjClass::Way, total: 1, shown: 1 },
            Group { class: ObjClass::Light, total: 1, shown: 1 },
        ];
        assert_eq!(types_to_hide(&groups, "way"), Ok(vec![ObjClass::Light]));
        let e = types_to_hide(&groups, "way;boat").unwrap_err();
        assert!(e.starts_with("unknown object type(s): boat; scene has: way, light"));
    }
}
