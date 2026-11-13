//! `characters_view` and `world_view`: the two tabs for whoever looks at them
//! (`EditorCharacters::McpView`, `EditorWorld::McpView`). Anything to show opens the tab on it;
//! without one they only report. The data paths stay `characters_*` and `world_get` /
//! `world_set`.

use std::sync::Arc;

use gz_app::{ShellState, on_ui};
use gz_base::Origin;
use gz_characters::Characters;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Reply, json};
use gz_world::{StartKit, World, WorldService};

use crate::call::{args, error_text, ok, s};
use crate::characters::{self, CharactersDoc, SavePlan, Then, problems_of, run_save};
use crate::world::{self, WorldDoc};

/// The pickers of the form `characters_view show=` puts up.
const SHOWN_PICKERS: [&str; 8] = ["base", "visual", "icon", "community", "rank", "dialog", "sells", "buys"];

fn characters_doc(shell: &mut ShellState) -> Option<&mut CharactersDoc> {
    shell
        .document_mut(characters::TAB)
        .and_then(|d| d.as_any_mut())
        .and_then(|a| a.downcast_mut::<CharactersDoc>())
}

fn world_doc(shell: &mut ShellState) -> Option<&mut WorldDoc> {
    shell.document_mut(world::TAB).and_then(|d| d.as_any_mut()).and_then(|a| a.downcast_mut::<WorldDoc>())
}

/// Brings a tab up through the shell's own opener; a window-less editor has none to bring.
async fn open_tab(ed: &Arc<Editor>, kind: &str, origin: Origin) -> Result<(), CmdError> {
    let reply = ed.call("ui.tab", args([("action", s("open")), ("kind", s(kind))]), origin).await;
    if !ok(&reply)
        && reply.get("error").and_then(serde_json::Value::as_str) != Some("the editor runs without a window")
    {
        return Err(CmdError::new("refused", error_text(&reply)));
    }
    Ok(())
}

/// Whether a tab of this kind is open; `false` without a window.
async fn is_open(ed: &Arc<Editor>, id: &'static str) -> Result<bool, CmdError> {
    match on_ui(ed, move |shell, _| Ok(shell.document_mut(id).is_some())).await {
        Ok(v) => Ok(v),
        Err(e) if e.code == "no_window" => Ok(false),
        Err(e) => Err(e),
    }
}

/// What settling the selection asks for next.
enum Step {
    Done,
    NotFound,
    Unsaved(Vec<String>),
    Save(Box<SavePlan>),
}

fn gone() -> CmdError {
    CmdError::new("refused", "the Characters tab closed meanwhile")
}

pub(crate) async fn characters_view(ed: Arc<Editor>, a: Args, origin: Origin) -> Reply {
    if ed.project_opt().is_none() {
        return Err(CmdError::no_project());
    }
    let id = a.text("select").map(str::to_owned);
    let path = a.text("path").map(str::to_owned);
    let (save, discard) = (a.flag("save"), a.flag("discard"));
    // a picker of the form put up to be looked at, never to act (not in the C++ wire)
    let show = a.text("show").map(str::to_owned);
    if let Some(name) = &show
        && !SHOWN_PICKERS.contains(&name.as_str())
    {
        return Err(CmdError::invalid(format!("show must be one of: {}", SHOWN_PICKERS.join(", "))));
    }
    // anything to show opens the tab on it; without one this only looks
    if id.is_some() || path.is_some() || show.is_some() || a.flag("open") {
        open_tab(&ed, characters::TAB, origin).await?;
    }
    if !is_open(&ed, characters::TAB).await? {
        return Ok(characters::closed_json());
    }
    // the files as they are now: a write over MCP may have come after the tab last looked (edits
    // in progress stay)
    let svc = Characters::of(&ed).await?;
    let roster = svc.fresh_roster().await;
    let module = svc.manifest().snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default();
    let (want_id, want_path) = (id.clone(), path.clone());
    let (r, m) = (roster.clone(), module.clone());
    let step = on_ui(&ed, move |shell, _| {
        let doc = characters_doc(shell).ok_or_else(gone)?;
        doc.take_roster(r, m);
        if want_id.is_none() && want_path.is_none() {
            return Ok(Step::Done);
        }
        let Some(index) = doc.find(want_id.as_deref(), want_path.as_deref()) else {
            return Ok(Step::NotFound);
        };
        if Some(index) != doc.sel && doc.is_dirty() {
            // the caller decides what happens to edits in progress: MCP never asks
            if save {
                return doc.save_plan().map(|p| Step::Save(Box::new(p))).map_err(|e| {
                    CmdError::new("refused", format!("the edited character was not saved: {e}"))
                });
            }
            if !discard {
                return Ok(Step::Unsaved(doc.dirty_names()));
            }
            doc.discard_edits();
        }
        let index = doc.find(want_id.as_deref(), want_path.as_deref());
        if index != doc.sel {
            doc.select(index);
        }
        Ok(Step::Done)
    })
    .await?;
    match step {
        Step::Done => {}
        Step::NotFound => {
            let what = match (&id, &path) {
                (Some(id), _) => format!("'{id}'"),
                (None, Some(p)) => format!("at {p}"),
                (None, None) => String::new(),
            };
            return Err(CmdError::not_found(format!(
                "no character {what} (asset.find refs_to=<id> names the files)"
            )));
        }
        Step::Unsaved(names) => {
            return Err(CmdError::unsaved(json!(names)).with_data(
                json!({ "dirty": names, "note": "save=true or discard=true settles the edits first" }),
            ));
        }
        Step::Save(plan) => {
            let file = run_save(&ed, *plan)
                .await
                .map_err(|e| CmdError::new("refused", format!("the edited character was not saved: {e}")))?;
            let roster = svc.fresh_roster().await;
            on_ui(&ed, move |shell, _| {
                let doc = characters_doc(shell).ok_or_else(gone)?;
                doc.saved(Ok(file), Then::Nothing, None);
                doc.take_roster(roster, module);
                let index = doc.find(id.as_deref(), path.as_deref());
                if index != doc.sel {
                    doc.select(index);
                }
                Ok(())
            })
            .await?;
        }
    }
    // the problems of the character the tab shows now, resolved against its base
    let input = on_ui(&ed, |shell, _| {
        Ok(characters_doc(shell).and_then(|d| d.validation_input().map(|i| (i, d.is_dirty()))))
    })
    .await?;
    let problems = match input {
        Some(((roster, sel, edit), dirty)) if sel.is_some() || dirty => {
            problems_of(&ed, roster, sel, edit).await
        }
        _ => Vec::new(),
    };
    on_ui(&ed, move |shell, ctx| {
        let active = shell.active() == characters::TAB;
        let doc = characters_doc(shell).ok_or_else(gone)?;
        doc.set_problems(problems);
        if let Some(name) = show {
            gz_ui::menu::show_popup(ctx, &format!("character.{name}"));
        }
        ctx.request_repaint();
        Ok(doc.view_json(active))
    })
    .await
}

pub(crate) async fn world_view(ed: Arc<Editor>, a: Args, origin: Origin) -> Reply {
    // over MCP, what it asks for is under More
    let more = a.flag("more") || a.has("level") || a.has("filter");
    if ed.project_opt().is_none() {
        return Err(CmdError::no_project());
    }
    let drawer = a.text("drawer").map(str::to_owned);
    if drawer.as_deref().is_some_and(|d| d != "open" && d != "close") {
        return Err(CmdError::invalid("drawer must be open or close"));
    }
    let level = a.text("level").map(str::to_owned);
    let filter = a.str("filter").map(str::to_owned);
    let svc = WorldService::of(&ed).await?;
    if let Some(level) = &level {
        let game = svc.game().await;
        if !game.level_exists(level) {
            return Err(CmdError::new(
                "refused",
                format!("the linked game has no level '{level}' (world_transitions lists them)"),
            ));
        }
    }
    // anything to set opens the tab on it; without one this only looks
    if drawer.is_some() || level.is_some() || filter.is_some() || a.flag("open") {
        open_tab(&ed, world::TAB, origin).await?;
    }
    let _ = svc.manifest().sync().await;
    let snap = svc.manifest().snapshot();
    let problems = match snap.doc() {
        Some(doc) => svc.problems(&ed, &World::parse(doc), &StartKit::parse(doc)).await.1,
        None => Vec::new(),
    };
    let kept = problems.clone();
    let answer = on_ui(&ed, move |shell, ctx| {
        if more {
            world::open_more(ctx);
        }
        let active = shell.active() == world::TAB;
        let Some(doc) = world_doc(shell) else { return Ok(world::closed_json(&problems)) };
        if let Some(d) = drawer {
            doc.set_drawer(d == "open");
        }
        if let Some(l) = level {
            doc.set_level(&l);
        }
        if let Some(f) = filter {
            doc.set_filter(&f);
        }
        ctx.request_repaint();
        Ok(doc.view_json(active, &problems))
    })
    .await;
    match answer {
        Ok(v) => Ok(v),
        // without a window there is no tab: it reports as closed
        Err(e) if e.code == "no_window" => Ok(world::closed_json(&kept)),
        Err(e) => Err(e),
    }
}
