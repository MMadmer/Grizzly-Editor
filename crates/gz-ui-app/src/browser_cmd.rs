//! `project_browser_menu`: brings a card's menu or the Delete Project confirmation into view - to
//! look at, never to act. The wire is `EditorProject::McpBrowserMenu`.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Reply, Value, json};
use gz_project::{DeleteInfo, EntryState, Sort, paths};
use serde_json::Map;

use crate::cards;
use crate::start::{self, Confirm, Menu, StartPage};

/// `{"ok":false,"error":<message>}`.
fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// The fields a deletion answers with, flat (`AppendDeleteJson` with no `counts` object).
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
    out.insert("failed".into(), d.failed.iter().map(|f| json!({ "path": paths::wire(&f.path), "reason": paths::wire(&f.reason) })).collect());
}

/// The menu a card offers in its state, as the page draws it.
fn menu_json(path: &str, shown: bool, state: EntryState) -> Value {
    let items: Vec<Value> = start::menu_items(state).iter().map(|(label, enabled)| json!({ "label": label, "enabled": enabled })).collect();
    json!({ "path": paths::wire(path), "shown": shown, "state": cards::state_name(state), "items": items })
}

pub(crate) async fn project_browser_menu(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let page = StartPage::of(&ed);
    let show = args.str("show").unwrap_or_default().to_owned();
    let path = args.text("path").map(paths::to_backslash).unwrap_or_default();
    let search = args.str("search").map(str::to_owned);
    let sort = args.text("sort").map(str::to_owned);
    if let Some(open) = args.bool("new_project") {
        page.state.lock().form_open = open;
    }
    let active = ed.project_opt().is_some();
    if (!show.is_empty() || search.is_some() || sort.is_some()) && active {
        return Err(refuse("the project browser is not on screen: a project is open"));
    }

    // everything is checked before anything changes
    let sort = match sort.as_deref() {
        None => None,
        Some(key) => Some(Sort::parse(key).ok_or_else(|| refuse("sort: recent or name"))?),
    };
    let (none, menu, confirm) = (show == "none", show == "menu", show == "confirm");
    if !show.is_empty() && !none && !menu && !confirm {
        return Err(refuse("show: menu, confirm or none"));
    }

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

    let target = if !show.is_empty() && !none {
        let found = page.state.lock().cards.iter().find(|c| !path.is_empty() && paths::same_path(&c.path, &path)).map(|c| (c.path.clone(), c.name.clone()));
        let Some((target, name)) = found else { return Err(refuse(format!("no card of the project browser shows '{path}'"))) };
        let probe = target.clone();
        let state = ed.exec.io(move || gz_project::list::entry_state(&probe)).await;
        {
            let mut st = page.state.lock();
            if let Some(card) = st.cards.iter_mut().find(|c| c.path == target) {
                card.state = state;
            }
        }
        if confirm && state != EntryState::Project {
            return Err(refuse("that folder holds no project: its menu offers Remove from List, which asks nothing"));
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
            let hidden = st.cards.iter().find(|c| &c.path == target).is_some_and(|c| !gz_ui::search::matches(&st.search, &format!("{} {}", c.name, c.path)));
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
                st.confirm = Some(Confirm { path: target.clone(), name: name.clone(), info: plan, done: false, deleting: false, shown: false });
            }
        }

        // the menu: asked for and not drawn yet, or what the last frame drew
        let menu_out = match &st.menu {
            Some(m) if m.request => Some((m.path.clone(), false)),
            _ => st.menu_shown.clone().filter(|_| !st.menu_close).map(|p| (p, true)),
        }
        .and_then(|(p, shown)| st.cards.iter().find(|c| paths::same_path(&c.path, &p)).map(|c| menu_json(&c.path, shown, c.state)));
        let confirm_out = st.confirm.as_ref().map(|c| {
            let mut out = Map::new();
            out.insert("id".into(), json!(start::delete_dialog_name()));
            out.insert("shown".into(), json!(c.shown));
            let d = c.info.clone().unwrap_or_else(|| DeleteInfo { path: c.path.clone(), name: c.name.clone(), ..DeleteInfo::default() });
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
    out.insert("menu".into(), menu_out.unwrap_or(Value::Null));
    out.insert("confirm".into(), confirm_out.unwrap_or(Value::Null));
    out.insert("search".into(), json!(search_now));
    out.insert("sort".into(), json!(sort_now.key()));
    out.insert("cards".into(), Value::Array(listed));
    Ok(Value::Object(out))
}
