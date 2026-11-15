//! The Level tab's commands of its own windows:
//!
//! - `scene.choices` over the scene's: the items of the lists only this crate reads (textures,
//!   shaders, models, motions);
//! - `scene.choose` - the chooser on a property put up for a person (the UI half of
//!   `scene.choices`): opened, searched, a row highlighted, closed, its state read;
//! - `level.show` - a window of the tab opened for a person (a file dialog, Summary Info), a
//!   popover of the toolbar or the status bar put up to look at, the viewport over the whole tab,
//!   and what of it is up now.

use std::sync::Arc;

use gz_app::on_ui;
use gz_editor::{CmdError, Editor, JsonSchema, Origin, Registry, Reply, Spec, Value, json};
use gz_scene::props;
use serde::Deserialize;

use crate::doc::{Act, post_to_level, with_level};
use crate::level_cmds::LevelDialog;

pub(crate) fn register(reg: &mut Registry) {
    // over the scene's own: with the lists the Level tab reads (textures, shaders, models, motions)
    reg.cmd(
        Spec::read("scene.choices", "What a property picks from: its words, or the items of its list.")
            .example("object=stalker_01 key=props.visual query=bandit"),
        scene_choices,
    );
    reg.cmd(
        Spec::view(
            "scene.choose",
            "Puts the chooser of a property up for a person: searched, a row highlighted with its preview; or closes it.",
        )
        .example("key=props.visual query=stalker highlight=actors\\stalker_bandit\\stalker_bandit_1"),
        scene_choose,
    );
    reg.cmd(
        Spec::view(
            "level.show",
            "Opens a window of the Level tab for a person, puts a popover up to look at, or the viewport over the whole tab; says what is up.",
        )
        .example("popover=build"),
        level_show,
    );
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChoicesParams {
    /// The object (left out: the first selected one).
    #[serde(default)]
    object: Option<String>,
    /// The property's key.
    key: String,
    /// A property of the level instead of an object's.
    #[serde(default)]
    world: bool,
    /// Only items that hold this text (any case).
    #[serde(default)]
    query: Option<String>,
    /// At most this many (default 500).
    #[serde(default)]
    limit: Option<usize>,
}

/// `scene.choices`: the scene's answer, with the items of the lists only this crate reads.
async fn scene_choices(ed: Arc<Editor>, p: ChoicesParams, _o: Origin) -> Reply {
    let (row, items) = gz_scene::choices(&ed, p.object.as_deref(), &p.key, p.world).await?;
    let list = match &row.kind {
        props::Kind::Choose(choice) => crate::lists::ListKey::of_choose(choice),
        _ => None,
    };
    let items = match (items, &list) {
        (Some(items), _) => items,
        (None, Some(list)) => crate::lists::Lists::wait(&ed, list).await.as_ref().clone(),
        (None, None) => Vec::new(),
    };
    let mut reply = gz_scene::choices_reply(&row, &items, p.query.as_deref(), p.limit);
    // an empty list says what is missing, of the thing it is missing from
    if items.is_empty()
        && let (Some(list), Value::Object(m)) = (&list, &mut reply)
    {
        m.insert("empty".into(), json!(list.empty_text()));
    }
    Ok(reply)
}

/// What `scene.choose` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ChooseAction {
    /// Opens the chooser of `key` (the default).
    #[default]
    Open,
    /// Moves the open chooser's search or highlight.
    Move,
    /// Takes the chooser down; nothing is picked.
    Close,
    /// Says what the chooser shows.
    State,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChooseParams {
    /// Open, move, close or state; left out: open.
    #[serde(default)]
    action: ChooseAction,
    /// open: the object whose property it is; it is selected first. Left out: the selection.
    #[serde(default)]
    object: Option<String>,
    /// open: the property's key (`scene.get` lists them).
    #[serde(default)]
    key: Option<String>,
    /// open: a property of the level (Level Settings) instead of the selection's.
    #[serde(default)]
    world: bool,
    /// open, move: the chooser's search.
    #[serde(default)]
    query: Option<String>,
    /// open, move: the item to highlight - its row is marked and the preview shows it.
    #[serde(default)]
    highlight: Option<String>,
}

/// The chooser as the Level document holds it, or null.
async fn chooser_state(ed: &Arc<Editor>) -> Result<Value, CmdError> {
    // the frame after the one that opened it has drawn its rows
    ed.frames.after(2).await?;
    on_ui(ed, |shell, _ctx| {
        with_level(shell, |doc| doc.chooser().map_or(Value::Null, crate::choose::Chooser::state))
            .ok_or_else(|| CmdError::new("no_window", "The Level tab is not open."))
    })
    .await
}

async fn scene_choose(ed: Arc<Editor>, p: ChooseParams, _o: Origin) -> Reply {
    gz_app::UiQueue::of(&ed)?;
    match p.action {
        ChooseAction::State => return Ok(json!({ "chooser": chooser_state(&ed).await? })),
        ChooseAction::Close => {
            post_to_level(&ed, Act::CloseChooser);
            ed.frames.after(1).await?;
            return Ok(json!({ "chooser": Value::Null }));
        }
        ChooseAction::Move => {
            let (query, highlight) = (p.query.clone(), p.highlight.clone());
            let moved = on_ui(&ed, move |shell, _ctx| {
                with_level(shell, |doc| match doc.chooser_mut() {
                    Some(c) => {
                        if let Some(q) = query {
                            c.search = q;
                        }
                        if let Some(h) = highlight {
                            c.highlight(&h);
                        }
                        true
                    }
                    None => false,
                })
                .ok_or_else(|| CmdError::new("no_window", "The Level tab is not open."))
            })
            .await?;
            if !moved {
                return Err(CmdError::invalid("No chooser is open: scene.choose key=<property> opens one."));
            }
            return Ok(json!({ "chooser": chooser_state(&ed).await? }));
        }
        ChooseAction::Open => {}
    }
    let key =
        p.key.clone().ok_or_else(|| CmdError::invalid("key names the property whose chooser opens."))?;
    if let Some(name) = &p.object {
        let mut a = gz_editor::Args::new();
        a.insert("objects".into(), json!([name]));
        let reply = ed.call("scene.select", a, Origin::Mcp).await;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) || reply.get("missing").is_some() {
            return Err(CmdError::not_found(format!("There is no object {name}.")));
        }
    }
    let svc = gz_scene::need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    let lib = svc.library.clone();
    let ctx = gz_scene::prop_context(&ed).await;
    let radio = gz_scene::game::radio_cx(&ed).await;
    let target = if p.world { crate::details::WORLD } else { crate::details::SELECTION };
    let k = key.clone();
    let chooser = ed
        .exec
        .cpu(move || {
            crate::details::chooser_for(&crate::details::rows(&snap, &lib, target, &ctx, &radio), &k, target)
        })
        .await;
    let Some(mut chooser) = chooser else {
        return Err(CmdError::invalid(format!("{key} picks nothing from a list: scene.set sets it.")));
    };
    if let Some(q) = p.query {
        chooser.search = q;
    }
    if let Some(h) = p.highlight {
        chooser.highlight(&h);
    }
    post_to_level(&ed, Act::Choose(Box::new(chooser)));
    Ok(json!({ "chooser": chooser_state(&ed).await? }))
}

/// The viewport over the whole tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Maximize {
    /// Over the whole tab.
    On,
    /// Back in its dock.
    Off,
    /// The other way round.
    Toggle,
}

/// The popovers `level.show` puts up to look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Popover {
    /// Build Level's menu: the steps with their state, Rebuild All, Advanced.
    Build,
    /// Play's menu: Play Clean, Play Without Building.
    Play,
    /// The toolbar's More.
    More,
    /// The status bar's Build chip (while a step is stale).
    Rebuild,
    /// None: whatever is up closes.
    None,
}

impl Popover {
    /// The name the kit's popups are asked for by.
    fn request(self) -> &'static str {
        match self {
            Popover::Build => "toolbar_split:level.build",
            Popover::Play => "toolbar_split:level.play",
            Popover::More => "level_more",
            Popover::Rebuild => "rebuild",
            Popover::None => "",
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShowParams {
    /// A window to open for a person: open_scene, save_as, place_group, save_group, export_obj,
    /// export_selection_obj, compile_folder (the system's file dialog), summary (Summary Info). A
    /// caller who cannot answer a file dialog runs the command it names instead.
    #[serde(default)]
    dialog: Option<LevelDialog>,
    /// A popover to put up, to look at: build, play, more, rebuild; none takes it down.
    #[serde(default)]
    popover: Option<Popover>,
    /// The viewport over the whole tab: on, off, toggle.
    #[serde(default)]
    maximize: Option<Maximize>,
    /// Goes to the problem of this row of the Problems panel (from 0), as a click on it does: its
    /// object selected and framed, or its place framed; the panel comes up.
    #[serde(default)]
    problem: Option<usize>,
}

async fn level_show(ed: Arc<Editor>, p: ShowParams, origin: Origin) -> Reply {
    gz_app::UiQueue::of(&ed)?;
    if let Some(d) = p.dialog {
        if d != LevelDialog::Summary && !origin.may_ask() {
            return Err(CmdError::new("refused", format!("{}: {}", crate::level_cmds::NAME_IT, d.instead())));
        }
        let e2 = ed.clone();
        on_ui(&ed, move |shell, ctx| {
            shell.activate(ctx, crate::LEVEL);
            crate::level_cmds::open(&e2, ctx, d).map_err(|e| CmdError::new("refused", e))
        })
        .await?;
    }
    if let Some(m) = p.maximize {
        let on = match m {
            Maximize::On => Some(true),
            Maximize::Off => Some(false),
            Maximize::Toggle => None,
        };
        post_to_level(&ed, Act::Maximize(on));
    }
    if let Some(n) = p.problem {
        let mut a = gz_editor::Args::new();
        a.insert("limit".into(), json!(10_000));
        let listed = ed.call("level.problems", a, Origin::Mcp).await;
        let rows = listed.get("problems").and_then(Value::as_array).cloned().unwrap_or_default();
        let row = rows.get(n).ok_or_else(|| {
            CmdError::not_found(format!("There is no problem {n}: the list holds {}.", rows.len()))
        })?;
        let problem = crate::problems::Problem::of(row);
        crate::problems::select(&ed, n);
        crate::problems::go(&ed, &problem).await;
        let mut a = gz_editor::Args::new();
        a.insert("action".into(), json!("open"));
        a.insert("id".into(), json!(crate::problems::PANEL));
        ed.call("ui.panel", a, Origin::Mcp).await;
    }
    if let Some(pop) = p.popover {
        on_ui(&ed, move |shell, ctx| {
            shell.activate(ctx, crate::LEVEL);
            egui::Popup::close_all(ctx);
            if pop != Popover::None {
                gz_ui::menu::show_popup(ctx, pop.request());
            }
            Ok(())
        })
        .await?;
    }
    let chooser = chooser_state(&ed).await.unwrap_or(Value::Null);
    let open = on_ui(&ed, |_shell, ctx| Ok(egui::Popup::is_any_open(ctx))).await?;
    Ok(json!({
        "maximized": crate::doc::maximized(&ed),
        "popover_open": open,
        "chooser": chooser,
        "summary_open": crate::level_cmds::summary_open(&ed),
        "problems": crate::problems::state_json(&ed),
        "build": gz_levelbuild::LevelBuild::status_now(&ed).map(|s| s.text()),
    }))
}
