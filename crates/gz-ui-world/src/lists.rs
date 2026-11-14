//! What the pickers of both tabs offer, read on the pools and only looked at by a frame
//! (the pickers' indexes, the trade categories, the visuals' models).
//!
//! [`Lists`] lives as long as the open project. A frame asks [`Lists::want`] - a comparison of
//! two revisions - and reads [`Lists::view`]; when the linked game or the project's files moved,
//! one task reads the lists again and the next frame shows them. Until the first read ends,
//! [`ListsView::loaded`] is false and a picker shows busy.

use std::sync::Arc;

use gz_base::{RelPath, Rev};
use gz_characters::Characters;
use gz_editor::Editor;
use gz_game::GameLink;
use gz_nq::{NqService, PickerEntry, PickerType, Pickers};
use gz_project::{EntryKind, Project};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::call::wake_ui;

/// One list with its ids looked up in a map: a frame never walks it to find a value.
#[derive(Clone, Debug, Default)]
pub struct Indexed {
    pub entries: Vec<PickerEntry>,
    by_id: FxHashMap<String, u32>,
    /// Moves when the list is read again: what a filtered view of it is keyed by.
    pub serial: u64,
}

impl Indexed {
    #[must_use]
    pub fn new(entries: Vec<PickerEntry>, serial: u64) -> Self {
        let mut by_id = FxHashMap::default();
        by_id.reserve(entries.len());
        for (i, e) in entries.iter().enumerate() {
            by_id.entry(e.id.clone()).or_insert(u32::try_from(i).unwrap_or(u32::MAX));
        }
        Self { entries, by_id, serial }
    }

    /// The entry of an id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&PickerEntry> {
        if id.is_empty() {
            return None;
        }
        self.by_id.get(id).and_then(|i| self.entries.get(*i as usize))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// "Caption  (id)" for an entry with a caption, the id alone otherwise.
    #[must_use]
    pub fn label(e: &PickerEntry) -> String {
        if e.name.is_empty() { e.id.clone() } else { format!("{}  ({})", e.name, e.id) }
    }
}

/// A rank of the linked game: its entry and the value it starts at, lowest first.
#[derive(Clone, Debug, Default)]
pub struct Rank {
    pub entry: PickerEntry,
    pub from: i32,
}

/// What the pickers offer now.
#[derive(Clone, Default)]
pub struct ListsView {
    /// The first read ended: until then a picker shows busy.
    pub loaded: bool,
    /// A linked game backs the lists.
    pub linked: bool,
    pub items: Arc<Indexed>,
    pub dialogs: Arc<Indexed>,
    pub icons: Arc<Indexed>,
    /// Every community of the linked game, for the captions.
    pub communities: Arc<Indexed>,
    /// The ones a stalker can be of: not the monsters', not the player's own.
    pub stalker_communities: Arc<Indexed>,
    /// `meshes\actors` of the linked game and the project, below `meshes\`, no extension.
    pub visuals: Arc<Indexed>,
    pub ranks: Arc<Vec<Rank>>,
    pub trade_categories: Arc<Vec<String>>,
    /// Moves with every read.
    pub serial: u64,
}

#[derive(Default)]
struct State {
    view: ListsView,
    /// The revisions the newest read was asked for.
    key: Option<Rev>,
    loading: bool,
    serial: u64,
}

/// The lists of the open project (`session.services`).
#[derive(Default)]
pub struct Lists {
    state: Mutex<State>,
}

/// A community a stalker can be of. The linked game's list also carries the monsters' (their
/// own section, plus "monster") and the player's own, and none of those makes an NPC.
fn stalker_community(e: &PickerEntry) -> bool {
    if e.extra == "monster_communities" || e.id == "monster" {
        return false;
    }
    e.id != "actor" && !e.id.starts_with("actor_")
}

/// The models of `meshes\actors`: the linked game's, then the project's own, sorted by id
/// (`Visuals`).
fn visuals(game: Option<&gz_vfs::GameFiles>, project: Option<&Project>) -> Vec<PickerEntry> {
    let mut out: Vec<PickerEntry> = Vec::new();
    let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
    if let Some(files) = game {
        for f in files.find("meshes/actors", "ogf") {
            let Some(stem) = f.path.get("meshes/".len()..f.path.len().saturating_sub(4)) else { continue };
            if stem.len() < "actors/x".len() {
                continue;
            }
            let id = stem.replace('/', "\\");
            if seen.insert(id.clone()) {
                out.push(PickerEntry { id, name: String::new(), extra: String::new() });
            }
        }
    }
    if let Some(project) = project
        && let Ok(dir) = RelPath::new("gamedata/meshes/actors")
    {
        let tree = project.tree.snapshot();
        for (rel, entry) in tree.files_under(&dir) {
            if !matches!(entry.kind, EntryKind::File { .. }) || !rel.has_extension("ogf") {
                continue;
            }
            let path = rel.as_str();
            let Some(stem) = path.get("gamedata/meshes/".len()..path.len().saturating_sub(4)) else {
                continue;
            };
            let id = stem.replace('/', "\\").to_lowercase();
            if seen.insert(id.clone()) {
                out.push(PickerEntry { id, name: "the project's".to_owned(), extra: "project".to_owned() });
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The view of what was read.
fn build_view(
    pickers: Option<&Pickers>,
    categories: Arc<Vec<String>>,
    visual_list: Vec<PickerEntry>,
    serial: u64,
) -> ListsView {
    let list = |t: PickerType| pickers.map(|p| p.entries(t).to_vec()).unwrap_or_default();
    let communities = list(PickerType::Community);
    let stalker: Vec<PickerEntry> = communities.iter().filter(|e| stalker_community(e)).cloned().collect();
    let mut ranks: Vec<Rank> = list(PickerType::Rank)
        .into_iter()
        .map(|e| Rank { from: gz_world::model::atoi(&e.extra), entry: e })
        .collect();
    ranks.sort_by_key(|r| r.from);
    ListsView {
        loaded: true,
        linked: pickers.is_some_and(Pickers::available),
        items: Arc::new(Indexed::new(list(PickerType::Item), serial)),
        dialogs: Arc::new(Indexed::new(list(PickerType::Dialog), serial)),
        icons: Arc::new(Indexed::new(list(PickerType::Icon), serial)),
        communities: Arc::new(Indexed::new(communities, serial)),
        stalker_communities: Arc::new(Indexed::new(stalker, serial)),
        visuals: Arc::new(Indexed::new(visual_list, serial)),
        ranks: Arc::new(ranks),
        trade_categories: categories,
        serial,
    }
}

impl Lists {
    /// The lists of the open project; `None` without one.
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Self>> {
        let session = editor.project_opt()?;
        Some(session.services.get_or_init(Self::default))
    }

    /// What the pickers offer now.
    #[must_use]
    pub fn view(&self) -> ListsView {
        self.state.lock().view.clone()
    }

    /// Reads the lists again when the linked game or the project's files moved since the last
    /// read: a frame calls it, and it only compares revisions.
    pub fn want(self: &Arc<Self>, editor: &Arc<Editor>) {
        let Ok((_, project)) = Project::of(editor) else { return };
        let key = project.game_rev().mix(project.tree.rev());
        {
            let mut st = self.state.lock();
            if st.loading || st.key == Some(key) {
                return;
            }
            st.loading = true;
            st.key = Some(key);
        }
        self.load(editor, false);
    }

    /// Reload: the linked game mounted again, then every list read again.
    pub fn force(self: &Arc<Self>, editor: &Arc<Editor>) {
        {
            let mut st = self.state.lock();
            if st.loading {
                st.key = None;
                return;
            }
            st.loading = true;
            st.key = None;
        }
        self.load(editor, true);
    }

    fn load(self: &Arc<Self>, editor: &Arc<Editor>, refresh: bool) {
        let (ed, me) = (editor.clone(), self.clone());
        let _task = editor.exec.spawn(async move {
            let link = GameLink::of(&ed).ok();
            if refresh && let Some(link) = &link {
                let _ = link.refresh().await;
            }
            let pickers = match NqService::of(&ed) {
                Ok(nq) => Some(nq.pickers(&ed).await),
                Err(_) => None,
            };
            let categories = match Characters::of(&ed).await {
                Ok(svc) => svc.checker(&ed).await.trade_categories,
                Err(_) => Arc::new(Vec::new()),
            };
            let game = match &link {
                Some(link) if Project::of(&ed).is_ok_and(|(_, p)| !p.game_root().is_empty()) => {
                    link.files().await.ok()
                }
                _ => None,
            };
            let project = Project::of(&ed).ok().map(|(_, p)| p);
            let serial = {
                let mut st = me.state.lock();
                st.serial += 1;
                st.serial
            };
            let view = ed
                .exec
                .io(move || {
                    let list = visuals(game.as_deref(), project.as_deref());
                    build_view(pickers.as_deref(), categories, list, serial)
                })
                .await;
            {
                let mut st = me.state.lock();
                st.view = view;
                st.loading = false;
            }
            wake_ui(&ed);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, name: &str, extra: &str) -> PickerEntry {
        PickerEntry { id: id.into(), name: name.into(), extra: extra.into() }
    }

    #[test]
    fn a_stalker_is_not_a_monster_or_the_player() {
        assert!(stalker_community(&e("stalker", "Loners", "")));
        assert!(!stalker_community(&e("monster", "", "")));
        assert!(!stalker_community(&e("zombie", "", "monster_communities")));
        assert!(!stalker_community(&e("actor_dolg", "", "")));
        assert!(!stalker_community(&e("actor", "", "")));
    }

    #[test]
    fn an_id_is_found_without_a_walk() {
        let list = Indexed::new(vec![e("b", "", ""), e("a", "Alpha", "")], 1);
        assert_eq!(list.get("a").map(|x| x.name.as_str()), Some("Alpha"));
        assert!(list.get("").is_none());
        assert!(list.get("c").is_none());
        assert_eq!(Indexed::label(&e("a", "Alpha", "")), "Alpha  (a)");
        assert_eq!(Indexed::label(&e("a", "", "")), "a");
    }

    #[test]
    fn a_view_without_a_game_is_loaded_and_empty() {
        let view =
            build_view(None, Arc::new(Vec::new()), vec![e("actors\\b", "", ""), e("actors\\a", "", "")], 3);
        assert!(view.loaded);
        assert!(!view.linked);
        assert_eq!(view.visuals.entries.len(), 2);
        assert!(view.ranks.is_empty());
    }
}
