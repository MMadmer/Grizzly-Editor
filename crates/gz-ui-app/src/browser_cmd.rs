//! `project.page`: the Start page as it is drawn - its cards, the search and the sort, the New
//! Project form - and a card's menu or the Delete Project confirmation brought into view, to look
//! at, never to act.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec, Value, json};
use gz_project::{DeleteInfo, EntryState, Sort, paths};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Map;

use crate::cards;
use crate::start::{self, Confirm, Menu, StartPage};

/// `{"ok":false,"error":<message>}`.
fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// What `project.page` puts up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Show {
    /// The card's menu, as its `...` button opens it.
    Menu,
    /// The Delete Project confirmation of the card, as Delete... opens it.
    Confirm,
    /// Takes the menu and the confirmation down; nothing is deleted.
    None,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PageParams {
    /// What to put up; left out: nothing changes.
    #[serde(default)]
    show: Option<Show>,
    /// The project folder whose card the menu or the confirmation is about.
    #[serde(default)]
    path: Option<String>,
    /// The search over the cards; empty clears it.
    #[serde(default)]
    query: Option<String>,
    /// recent (Last Opened) or name.
    #[serde(default)]
    sort: Option<String>,
    /// Opens (true) or closes (false) the New Project form in the first slot.
    #[serde(default)]
    new_project: Option<bool>,
}

/// Registers `project.page`.
pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::view(
            "project.page",
            "Shows the Start page's cards, search, sort and New Project form; puts a card's menu up to look at.",
        )
        .example("show=menu path=D:/Projects/Night"),
        project_page,
    );
}

/// The fields a deletion answers with, flat.
fn delete_fields(out: &mut Map<String, Value>, d: &DeleteInfo) {
    out.insert("path".into(), json!(paths::wire(&d.path)));
    out.insert("name".into(), json!(paths::wire(&d.name)));
    out.insert("refused".into(), json!(d.refused.is_some()));
    if d.missing {
        out.insert("missing".into(), json!(true));
    }
    out.insert("files".into(), json!(d.files));
    out.insert("folders".into(), json!(d.folders));
    out.insert("links".into(), json!(d.links));
    out.insert("bytes".into(), json!(d.bytes));
    out.insert("git".into(), json!(d.git));
    out.insert("releases".into(), json!(d.releases));
    out.insert("release_files".into(), json!(d.release_files));
    out.insert("release_bytes".into(), json!(d.release_bytes));
    out.insert("nested_total".into(), json!(d.nested_total));
    out.insert("failed_total".into(), json!(d.failed_total));
    out.insert("nested".into(), d.nested.iter().map(|n| json!(paths::wire(n))).collect());
    out.insert(
        "failed".into(),
        d.failed
            .iter()
            .map(|f| json!({ "path": paths::wire(&f.path), "reason": paths::wire(&f.reason) }))
            .collect(),
    );
}

/// The menu a card offers in its state, as the page draws it.
fn menu_json(path: &str, shown: bool, state: EntryState) -> Value {
    let items: Vec<Value> = start::menu_items(state)
        .iter()
        .map(|(label, enabled)| json!({ "label": label, "enabled": enabled }))
        .collect();
    json!({ "path": paths::wire(path), "shown": shown, "state": cards::state_name(state), "items": items })
}

async fn project_page(ed: Arc<Editor>, p: PageParams, _origin: Origin) -> Reply {
    let page = StartPage::of(&ed);
    let path = p.path.as_deref().map(paths::to_backslash).unwrap_or_default();
    let search = p.query.clone();
    let sort = p.sort.clone();
    let active = ed.project_opt().is_some();
    if (p.show.is_some() || search.is_some() || sort.is_some() || p.new_project.is_some()) && active {
        return Err(refuse("the Start page is not on screen: a project is open"));
    }
    if let Some(open) = p.new_project {
        page.state.lock().form_open = open;
    }

    // everything is checked before anything changes
    let sort = match sort.as_deref() {
        None => None,
        Some(key) => Some(Sort::parse(key).ok_or_else(|| CmdError::invalid("sort takes recent or name"))?),
    };
    let (none, menu, confirm) =
        (p.show == Some(Show::None), p.show == Some(Show::Menu), p.show == Some(Show::Confirm));
    let show = p.show.is_some();

    if active {
        // the page is not drawn while a project is open: nothing of it is up
        let mut st = page.state.lock();
        st.menu = None;
        st.confirm = None;
        st.menu_shown = None;
        let mut out = Map::new();
        out.insert("browser".into(), json!(false));
        out.insert("menu".into(), Value::Null);
        out.insert("confirm".into(), Value::Null);
        out.insert("search".into(), json!(st.search));
        out.insert("sort".into(), json!(st.sort.key()));
        out.insert("cards".into(), json!([]));
        return Ok(Value::Object(out));
    }

    // the list as the file has it now
    let snap = cards::load(&ed).await;
    {
        let mut st = page.state.lock();
        start::apply_snapshot(&mut st, snap);
    }

    let target = if show && !none {
        let found = page
            .state
            .lock()
            .cards
            .iter()
            .find(|c| !path.is_empty() && paths::same_path(&c.path, &path))
            .map(|c| (c.path.clone(), c.name.clone()));
        let Some((target, name)) = found else {
            return Err(CmdError::not_found(format!("no card of the Start page shows '{path}'")));
        };
        let probe = target.clone();
        let state = ed.exec.io(move || gz_project::list::entry_state(&probe)).await;
        {
            let mut st = page.state.lock();
            if let Some(card) = st.cards.iter_mut().find(|c| c.path == target) {
                card.state = state;
            }
        }
        if confirm && state != EntryState::Project {
            return Err(refuse(
                "that folder holds no project: its menu offers Remove from List, which asks nothing",
            ));
        }
        Some((target, name))
    } else {
        None
    };
    // the window's own opening: the check and the count, then the question
    let plan = match (&target, confirm) {
        (Some((target, _)), true) => Some(start::delete_plan(&ed, target.clone()).await),
        _ => None,
    };

    // what the page shows changes; nothing is saved and nothing is acted on
    let (cards_now, search_now, sort_now, menu_out, confirm_out) = {
        let mut st = page.state.lock();
        if let Some(sort) = sort {
            st.sort = sort;
        }
        if let Some(search) = &search {
            st.search.clone_from(search);
        }
        if none {
            // both go down, and nothing is deleted
            st.menu = None;
            st.menu_close = true;
            st.confirm = None;
        } else if let Some((target, name)) = &target {
            // a card the search hides has no menu to show: the search goes
            let hidden = st
                .cards
                .iter()
                .find(|c| &c.path == target)
                .is_some_and(|c| !gz_ui::search::matches(&st.search, &format!("{} {}", c.name, c.path)));
            if hidden {
                st.search.clear();
            }
            if menu {
                st.confirm = None;
                st.menu_close = false;
                st.menu = Some(Menu { path: target.clone(), anchor: None, request: true });
            } else {
                st.menu = None;
                st.menu_close = true;
                st.confirm = Some(Confirm {
                    path: target.clone(),
                    name: name.clone(),
                    info: plan,
                    done: false,
                    deleting: false,
                    shown: false,
                });
            }
        }

        // the menu: asked for and not drawn yet, or what the last frame drew
        let menu_out = match &st.menu {
            Some(m) if m.request => Some((m.path.clone(), false)),
            _ => st.menu_shown.clone().filter(|_| !st.menu_close).map(|p| (p, true)),
        }
        .and_then(|(p, shown)| {
            st.cards
                .iter()
                .find(|c| paths::same_path(&c.path, &p))
                .map(|c| menu_json(&c.path, shown, c.state))
        });
        let confirm_out = st.confirm.as_ref().map(|c| {
            let mut out = Map::new();
            out.insert("id".into(), json!(start::delete_dialog_name()));
            out.insert("shown".into(), json!(c.shown));
            let d = c.info.clone().unwrap_or_else(|| DeleteInfo {
                path: c.path.clone(),
                name: c.name.clone(),
                ..DeleteInfo::default()
            });
            let view = if c.done {
                "result"
            } else if d.refused.is_some() || d.missing {
                "refused"
            } else {
                "question"
            };
            out.insert("view".into(), json!(view));
            out.insert("reason".into(), json!(paths::wire(d.refused.as_deref().unwrap_or(""))));
            delete_fields(&mut out, &d);
            Value::Object(out)
        });
        let order = cards::card_order(&st.cards, &st.search, st.sort);
        let cards_now: Vec<cards::Card> = order.iter().filter_map(|&i| st.cards.get(i).cloned()).collect();
        (cards_now, st.search.clone(), st.sort, menu_out, confirm_out)
    };
    ed.exec.wake().ping();

    // what the card's picture area holds: preview.png, or the placeholder for none or one of no scene
    let mut listed = Vec::with_capacity(cards_now.len());
    for card in &cards_now {
        let preview = start::preview_state(&page, card).await;
        listed.push(start::card_json(card, preview));
    }

    let mut out = Map::new();
    out.insert("browser".into(), json!(true));
    out.insert("new_project".into(), json!(page.state.lock().form_shown()));
    out.insert("menu".into(), menu_out.unwrap_or(Value::Null));
    out.insert("confirm".into(), confirm_out.unwrap_or(Value::Null));
    out.insert("search".into(), json!(search_now));
    out.insert("sort".into(), json!(sort_now.key()));
    out.insert("cards".into(), Value::Array(listed));
    Ok(Value::Object(out))
}
