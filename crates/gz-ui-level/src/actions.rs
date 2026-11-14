//! The Level tab's actions: what its menus, its toolbar, its keys and the palette run
//! (docs/UI_DESIGN.md, section 5 "Actions" and "Level workspace").
//!
//! Each is a registry command with its arguments - `scene.*` for the scene and the selection,
//! `level.*` for building and playing, `level.show` for the windows of the tab (a file dialog, the
//! Summary Info window, the viewport over the whole tab) - so a key, a menu row, a toolbar button,
//! a palette row and an agent run the same code. The keys of the ones that act on the scene are
//! live on the Level tab only (`Scope::Tab`); their rows are there wherever they are on.
//!
//! What only a person needs is the reply: a command that refuses because the scene holds unsaved
//! work (`code: unsaved`) is asked about with the kit's Save / Don't Save / Cancel and sent again,
//! and Validate shows its findings in the kit's result dialog.

use std::sync::Arc;

use egui::Key;
use gz_app::{Action, ActionCx, Chord, Enabled, ReplyCx, Scope};
use gz_editor::{Args, Editor, Value, json};
use gz_levelbuild::Step;
use gz_scene::{SceneService, SceneState};
use gz_ui::dialog::{Answer, ResultSpec};
use gz_ui::theme::Tone;
use gz_ui::{Icon, Tx};

/// The kinds of tab the level's keys are live on.
pub const LEVEL_TABS: &[&str] = &["level"];
const TAB: Scope = Scope::Tab(LEVEL_TABS);

/// The groups of the palette and the Keyboard page.
const SCENE: &str = "Scene";
const SELECTION: &str = "Selection";
const BUILD: &str = "Build";

// ---- ids the menus, the toolbar and the status bar name -------------------------------------------

pub const NEW_SCENE: &str = "level.new_scene";
pub const OPEN_SCENE: &str = "level.open_scene";
pub const OPEN_LAST_SCENE: &str = "level.open_last_scene";
pub const SAVE_AS: &str = "level.save_as";
pub const SELECT_ALL: &str = "level.select_all";
pub const DESELECT_ALL: &str = "level.deselect_all";
pub const INVERT_SELECTION: &str = "level.invert_selection";
/// The viewport's own (`gz-ui-viewport`'s actions), listed in the Scene menu too.
pub const HIDE_SELECTED: &str = "viewport.hide_selected";
pub const HIDE_UNSELECTED: &str = "level.hide_unselected";
pub const SHOW_ALL: &str = "viewport.show_all";
pub const GROUP: &str = "viewport.group";
pub const UNGROUP: &str = "level.ungroup";
pub const PLACE_GROUP: &str = "level.place_group";
pub const SAVE_GROUP: &str = "level.save_group";
pub const MARK_MOD: &str = "level.mark_mod_content";
pub const MARK_BASE: &str = "level.mark_base_level";
pub const SUMMARY: &str = "level.summary";
pub const BROWSE_TO_ASSET: &str = "level.browse_to_asset";
pub const SNAPSHOT: &str = "level.recovery_snapshot";
pub const MAXIMIZE: &str = "level.maximize_viewport";
pub const BUILD_LEVEL: &str = "level.build";
pub const REBUILD_ALL: &str = "level.rebuild_all";
pub const BUILD_DRAFT: &str = "level.build_draft";
pub const BUILD_FINAL: &str = "level.build_final";
pub const VALIDATE: &str = "level.validate";
pub const SHOW_PROBLEMS: &str = "level.show_problems";
pub const CLEAR_PROBLEMS: &str = "level.clear_problems";
pub const EXPORT_OBJ: &str = "level.export_obj";
pub const EXPORT_SELECTION_OBJ: &str = "level.export_selection_obj";
pub const COMPILE_FOLDER: &str = "level.compile_folder";
pub const PLAY: &str = "level.play";
pub const PLAY_CLEAN: &str = "level.play_clean";
pub const PLAY_WITHOUT_BUILDING: &str = "level.play_without_building";
pub const STOP: &str = "level.stop";
pub const RELOAD_CONFIGS: &str = "level.reload_configs";
pub const REMOVE_BUILD_FILES: &str = "level.remove_build_files";
pub const GAME_FOLDER: &str = "level.game_folder";

/// The mod's level layers (`gz-ui-mod`'s actions), listed under Build > Advanced too.
pub const LAYER_ACTIONS: [&str; 4] =
    ["mod.bake_spawn", "mod.bake_collision", "mod.bake_visuals", "mod.bake_cut"];

/// The action that builds one step of the level: its id.
#[must_use]
pub fn step_action(step: Step) -> &'static str {
    match step {
        Step::Game => "level.build_game",
        Step::SpawnLayer => "level.build_spawn_layer",
        Step::AiMap => "level.build_ai_map",
        Step::GameGraph => "level.build_game_graph",
        Step::Spawn => "level.build_spawn",
        Step::Geometry => "level.build_geometry",
        Step::Details => "level.build_details",
        Step::Light => "level.build_light",
        Step::DetailLight => "level.build_detail_light",
    }
}

// ---- what the actions ask of the editor -----------------------------------------------------------

/// The open scene, when one is up.
#[must_use]
pub fn scene(editor: &Editor) -> Option<Arc<SceneService>> {
    editor.project_opt().and_then(|s| s.services.get::<SceneService>()).filter(|s| s.store.snapshot().valid)
}

fn snapshot(cx: &ActionCx<'_>) -> Option<Arc<gz_store::Snap<SceneState>>> {
    scene(cx.editor).map(|s| s.store.snapshot())
}

/// The Library's kind, the field and the value a scene object's asset is found by in the Content
/// Browser (`content.view action=reveal`): an object, a group, a sound, a particle effect, a spawn
/// section. A model the game draws as it is (visual mode) has no asset of the library.
fn asset_of(o: &gz_scene::Object) -> Option<(&'static str, &'static str, String)> {
    use gz_scene::ObjData;
    let named = |s: &str| (!s.trim().is_empty()).then(|| s.to_owned());
    match &o.data {
        ObjData::SceneObject(d) if !d.visual_mode() => named(&d.reference).map(|r| ("objects", "ref", r)),
        ObjData::Group(g) => named(&g.reference).map(|r| ("groups", "ref", r)),
        ObjData::SoundSrc(d) => named(&d.wav).map(|r| ("sounds", "ref", r)),
        ObjData::Particles(d) => named(&d.reference).map(|r| ("particles", "ref", r)),
        ObjData::Spawn(d) => {
            d.entity.as_ref().and_then(|e| named(e.section())).map(|r| ("spawn_sections", "section", r))
        }
        _ => None,
    }
}

/// The asset of the first selected object that has one.
fn selected_asset(cx: &ActionCx<'_>) -> Option<(&'static str, &'static str, String)> {
    let s = snapshot(cx)?;
    s.selected_in_order().into_iter().filter_map(|id| s.get(id)).find_map(|o| asset_of(o))
}

fn with_project(cx: &ActionCx<'_>) -> Enabled {
    Enabled::when(cx.editor.project_opt().is_some(), "Open a project first.")
}

fn with_scene(cx: &ActionCx<'_>) -> Enabled {
    Enabled::when(scene(cx.editor).is_some(), "Open or create a scene first.")
}

/// What an action of the scene needs to be on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    Scene,
    Objects,
    Selection,
}

/// Whether an action that needs `needs` is on for a scene of `objects` objects and `selected`
/// selected (`None`: no scene is up).
#[must_use]
pub fn gate(scene: Option<(usize, usize)>, needs: Needs) -> Enabled {
    match (scene, needs) {
        (None, _) => Enabled::Off("Open or create a scene first."),
        (Some((objects, _)), Needs::Objects) => Enabled::when(objects > 0, "The scene is empty."),
        (Some((_, selected)), Needs::Selection) => Enabled::when(selected > 0, "Select objects first."),
        (Some(_), Needs::Scene) => Enabled::On,
    }
}

fn counts(cx: &ActionCx<'_>) -> Option<(usize, usize)> {
    snapshot(cx).map(|s| (s.objects.len(), s.selection.len()))
}

fn with_objects(cx: &ActionCx<'_>) -> Enabled {
    gate(counts(cx), Needs::Objects)
}

fn with_selection(cx: &ActionCx<'_>) -> Enabled {
    gate(counts(cx), Needs::Selection)
}

/// A build, the compiler or the game runs.
#[must_use]
pub fn running(editor: &Editor) -> bool {
    [gz_levelbuild::JOB_BUILD, "game", gz_levelbuild::JOB_RUN]
        .iter()
        .any(|g| editor.jobs.running_in(g).is_some())
}

const BUSY: &str = "A build, the compiler or the game is running.";

fn can_build(cx: &ActionCx<'_>) -> Enabled {
    if running(cx.editor) {
        return Enabled::Off(BUSY);
    }
    with_objects(cx)
}

fn can_play(cx: &ActionCx<'_>) -> Enabled {
    if running(cx.editor) {
        return Enabled::Off(BUSY);
    }
    with_scene(cx)
}

fn can_stop(cx: &ActionCx<'_>) -> Enabled {
    Enabled::when(running(cx.editor), "Nothing is running.")
}

fn game_root(editor: &Editor) -> Option<String> {
    gz_game::GameLink::of(editor).ok().map(|l| l.root()).filter(|r| !r.is_empty())
}

/// The names of the objects of a list of ids.
fn names_of(s: &SceneState, pick: impl Fn(&gz_scene::Object) -> bool) -> Vec<Value> {
    s.objects.values().filter(|o| pick(o)).map(|o| Value::String(o.name.to_string())).collect()
}

/// The scenes of the project opened last on this machine, project-relative, newest first.
#[must_use]
pub fn recent(editor: &Editor) -> Vec<String> {
    let most =
        gz_app::EditorSettings::of(editor).pref("scene_recent_count").unwrap_or(10.0).max(0.0) as usize;
    let mut list =
        gz_project::Project::of(editor).map(|(_, p)| p.state().recent_scenes()).unwrap_or_default();
    list.truncate(most);
    list
}

fn pairs(list: &[(&str, Value)]) -> Args {
    list.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

fn view(what: &str, value: &str) -> Args {
    pairs(&[(what, Value::String(value.to_owned()))])
}

// ---- replies a person needs -----------------------------------------------------------------------

/// A scene command that refused for unsaved work asks Save / Don't Save / Cancel and is sent again
/// with the answer; any other failure is a notice. What it opened brings the Level tab up.
fn settled(r: &ReplyCx<'_>, command: &'static str, args: Args) {
    let ok = r.reply.get("ok").and_then(Value::as_bool) == Some(true);
    if ok {
        r.editor.post("ui.tab", pairs(&[("action", json!("activate")), ("doc", json!(crate::LEVEL))]));
        return;
    }
    if r.reply.get("code").and_then(Value::as_str) != Some("unsaved") {
        let text = r.reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.");
        r.editor.notices.error(text.to_owned());
        return;
    }
    let name = scene(r.editor).map(|s| s.store.snapshot().meta.name()).unwrap_or_default();
    let what = vec![if name.is_empty() { Tx::from("Untitled scene") } else { Tx::raw(name) }];
    let ed = r.editor.clone();
    let line =
        gz_ui::dialog::unsaved_prompt(r.ctx, crate::tools::asker(r.editor), "scene", &what, move |answer| {
            let flag = match answer {
                Answer::Yes => "save",
                Answer::No => "discard",
                Answer::Cancel => return,
            };
            let mut a = args;
            a.insert(flag.into(), Value::Bool(true));
            let ed2 = ed.clone();
            ed.exec.spawn(async move {
                let reply = ed2.call(command, a, ed2.ui_origin()).await;
                if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                    ed2.post("ui.tab", pairs(&[("action", json!("activate")), ("doc", json!(crate::LEVEL))]));
                } else {
                    let text = reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.");
                    ed2.notices.error(text.to_owned());
                }
            });
        });
    if let Some(l) = line {
        r.editor.log.finding(&l);
    }
}

fn last_scene_args(editor: &Editor) -> Args {
    recent(editor).into_iter().next().map_or_else(Args::new, |p| pairs(&[("path", Value::String(p))]))
}

/// Validate's findings in the kit's result dialog.
pub fn show_validation(r: &ReplyCx<'_>) {
    let ok = r.reply.get("ok").and_then(Value::as_bool) == Some(true);
    let valid = r.reply.get("valid").and_then(Value::as_bool) == Some(true);
    let lines: Vec<String> = r
        .reply
        .get("problems")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).map(str::to_owned).collect())
        .unwrap_or_default();
    let (tone, headline) = if !ok {
        let why = r.reply.get("error").and_then(Value::as_str).map(|e| Tx::ui(e.to_owned()));
        (Tone::Error, why.unwrap_or_else(|| Tx::from("The scene was not checked.")))
    } else if valid {
        (
            if lines.is_empty() { Tone::Success } else { Tone::Warning },
            Tx::from("The scene passes its checks."),
        )
    } else {
        (Tone::Error, Tx::from("The scene does not pass its checks."))
    };
    let mut spec = ResultSpec::new("validate_result", "Validate Scene", tone, headline);
    spec.warnings = lines;
    for l in gz_ui::dialog::result(r.ctx, crate::tools::asker(r.editor), spec) {
        r.editor.log.finding(&l);
    }
}

/// Remove Build Files: on a level the linked game has, a build compiled the level there.
fn can_remove_build_files(cx: &ActionCx<'_>) -> Enabled {
    if running(cx.editor) {
        return Enabled::Off(BUSY);
    }
    let st = gz_levelbuild::LevelBuild::status_last(cx.editor);
    Enabled::when(
        st.is_some_and(|s| s.stock && !s.built_over.is_empty()),
        "No build compiled this level of the linked game here.",
    )
}

/// The files `level.clean dry_run=true` named, asked about by name through the kit's confirm,
/// and removed on Yes.
fn confirm_build_files(r: &ReplyCx<'_>) {
    if r.reply.get("ok").and_then(Value::as_bool) != Some(true) {
        let text = r.reply.get("error").and_then(Value::as_str).unwrap_or("The build files were not found.");
        r.editor.notices.error(text.to_owned());
        return;
    }
    let files: Vec<String> = r
        .reply
        .get("files")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    let level = r.reply.get("level").and_then(Value::as_str).unwrap_or_default().to_owned();
    if files.is_empty() {
        r.editor.notices.note(format!("{level}'s folder holds no file a build wrote."));
        return;
    }
    let spec = gz_ui::ConfirmSpec {
        id: "clean_level".into(),
        title: "Remove Build Files".into(),
        headline: gz_ui::tr!(
            "Remove {n} {n:file|files} a level build wrote into {level}'s folder? The module would ship them over the game's own level.",
            n = files.len(),
            level = level
        ),
        total: files.len(),
        items: files.into_iter().map(gz_ui::Tx::raw).collect(),
        consequence: "Deleted for good - this cannot be undone.".into(),
        action: "Remove".into(),
        destructive: true,
    };
    let ed = r.editor.clone();
    crate::local::confirm(r.ctx, r.editor, spec, move || {
        let ed2 = ed.clone();
        ed.exec.spawn(async move {
            let reply = ed2.call("level.clean", Args::new(), ed2.ui_origin()).await;
            crate::problems::changed(&ed2);
            if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                let text =
                    reply.get("error").and_then(Value::as_str).unwrap_or("The build files were not removed.");
                ed2.notices.error(text.to_owned());
                return;
            }
            // a file in use stays, and stays the build's: the author learns which
            let kept: Vec<&str> = reply
                .get("kept")
                .and_then(Value::as_array)
                .map(|l| l.iter().filter_map(|k| k.get("file").and_then(Value::as_str)).collect())
                .unwrap_or_default();
            if !kept.is_empty() {
                ed2.notices.error(format!("These build files could not be removed: {}.", kept.join(", ")));
            }
        });
    });
}

/// A Clear Problems answered: the Problems panel reads the list again.
fn problems_cleared(r: &ReplyCx<'_>) {
    crate::problems::changed(r.editor);
    if r.reply.get("ok").and_then(Value::as_bool) != Some(true) {
        let text = r.reply.get("error").and_then(Value::as_str).unwrap_or("The problems were not cleared.");
        r.editor.notices.error(text.to_owned());
    }
}

// ---- the table ----------------------------------------------------------------------------------

/// An action of the step `$step`: build it (and what it needs) even when it is fresh.
macro_rules! step {
    ($step:expr) => {
        Action::new(step_action($step), step_title($step), BUILD, "level.build")
            .scope(TAB)
            .args(|_| pairs(&[("only", json!($step.name())), ("force", json!(true))]))
            .enabled(can_build)
    };
}

/// The steps' actions, titled by what they make.
fn step_actions() -> Vec<Action> {
    vec![
        step!(Step::Game),
        step!(Step::SpawnLayer),
        step!(Step::AiMap),
        step!(Step::GameGraph),
        step!(Step::Spawn),
        step!(Step::Geometry),
        step!(Step::Details),
        step!(Step::Light),
        step!(Step::DetailLight),
    ]
}

/// "Build Game Files", "Build AI Map": the title of a step's action.
#[must_use]
pub fn step_title(step: Step) -> &'static str {
    match step {
        Step::Game => "Build Game Files",
        Step::SpawnLayer => "Build Spawn Layer",
        Step::AiMap => "Build AI Map",
        Step::GameGraph => "Build Game Graph",
        Step::Spawn => "Build Spawn",
        Step::Geometry => "Build Geometry",
        Step::Details => "Build Detail Objects",
        Step::Light => "Build Lighting",
        Step::DetailLight => "Build Detail Lighting",
    }
}

/// Every action of the Level tab.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn actions() -> Vec<Action> {
    let mut v = vec![
        // the scene's file
        Action::new(NEW_SCENE, "New Scene", SCENE, "scene.new")
            .icon(Icon::New)
            .keys(const { &[Chord::ctrl(Key::N)] })
            .scope(TAB)
            .enabled(with_project)
            .reply(|r| settled(r, "scene.new", Args::new())),
        Action::new(OPEN_SCENE, "Open Scene...", SCENE, "level.show")
            .icon(Icon::Open)
            .keys(const { &[Chord::ctrl(Key::O)] })
            .scope(TAB)
            .args(|_| view("dialog", "open_scene"))
            .enabled(with_project),
        Action::new(OPEN_LAST_SCENE, "Open Last Scene", SCENE, "scene.open")
            .scope(TAB)
            .args(|cx| last_scene_args(cx.editor))
            .enabled(|cx| {
                Enabled::when(!recent(cx.editor).is_empty(), "No scene of the project was opened yet.")
            })
            .reply(|r| settled(r, "scene.open", last_scene_args(r.editor))),
        Action::new(SAVE_AS, "Save As...", SCENE, "level.show")
            .keys(const { &[Chord::ctrl_alt(Key::S)] })
            .scope(TAB)
            .args(|_| view("dialog", "save_as"))
            .enabled(with_scene),
        // the selection, of whatever kinds it holds
        Action::new(SELECT_ALL, "Select All", SELECTION, "scene.select")
            .icon(Icon::Select)
            .keys(const { &[Chord::ctrl(Key::A)] })
            .scope(TAB)
            .args(|_| pairs(&[("all", json!(true))]))
            .enabled(with_objects),
        Action::new(DESELECT_ALL, "Deselect All", SELECTION, "scene.select")
            .scope(TAB)
            .args(|_| pairs(&[("none", json!(true))]))
            .enabled(with_selection),
        Action::new(INVERT_SELECTION, "Invert Selection", SELECTION, "scene.select")
            .keys(const { &[Chord::ctrl(Key::I)] })
            .scope(TAB)
            .args(|_| pairs(&[("invert", json!(true))]))
            .enabled(with_objects),
        Action::new(HIDE_UNSELECTED, "Hide Unselected", SELECTION, "scene.set")
            .keys(const { &[Chord::shift(Key::H)] })
            .scope(TAB)
            .args(|cx| {
                let names = snapshot(cx).map(|s| names_of(&s, |o| o.flags.visible && !s.is_selected(o.id)));
                pairs(&[("hidden", json!(true)), ("objects", Value::Array(names.unwrap_or_default()))])
            })
            .enabled(|cx| match snapshot(cx) {
                None => with_scene(cx),
                Some(s) => Enabled::when(
                    !s.selection.is_empty() && s.objects.len() > s.selection.len(),
                    "Select what stays shown first.",
                ),
            }),
        // groups
        Action::new(UNGROUP, "Ungroup", SCENE, "scene.ungroup")
            .keys(const { &[Chord::shift(Key::G)] })
            .scope(TAB)
            .enabled(|cx| match snapshot(cx) {
                None => with_scene(cx),
                Some(s) => Enabled::when(
                    s.selected_in_order()
                        .iter()
                        .filter_map(|id| s.get(*id))
                        .any(|o| o.class == gz_scene::ObjClass::Group),
                    "Select a group first.",
                ),
            }),
        Action::new(PLACE_GROUP, "Place Group...", SCENE, "level.show")
            .scope(TAB)
            .args(|_| view("dialog", "place_group"))
            .enabled(with_scene),
        Action::new(SAVE_GROUP, "Save Selection as Group...", SCENE, "level.show")
            .scope(TAB)
            .args(|_| view("dialog", "save_group"))
            .enabled(with_selection),
        // what the module ships
        Action::new(MARK_MOD, "Mark as Mod Content", SCENE, "scene.set")
            .scope(TAB)
            .args(|_| pairs(&[("set", json!({ "authored": true }))]))
            .enabled(with_selection),
        Action::new(MARK_BASE, "Mark as Base Level", SCENE, "scene.set")
            .scope(TAB)
            .args(|_| pairs(&[("set", json!({ "authored": false }))]))
            .enabled(with_selection),
        // the selection's asset; with none, the key goes on to the tab's own file
        Action::new(BROWSE_TO_ASSET, "Browse to Asset", SCENE, "content.view")
            .icon(Icon::Folder)
            .keys(const { &[Chord::ctrl(Key::B)] })
            .scope(TAB)
            .args(|cx| {
                let (kind, field, value) = selected_asset(cx).unwrap_or_default();
                pairs(&[("action", json!("reveal")), ("kind", json!(kind)), (field, json!(value))])
            })
            .enabled(|cx| {
                Enabled::when(selected_asset(cx).is_some(), "Select an object made from an asset first.")
            }),
        Action::new(SUMMARY, "Summary Info", SCENE, "level.show")
            .icon(Icon::Info)
            .scope(TAB)
            .args(|_| view("dialog", "summary"))
            .enabled(with_scene),
        Action::new(SNAPSHOT, "Create Recovery Snapshot", SCENE, "scene.recover")
            .scope(TAB)
            .args(|_| view("action", "snapshot"))
            .enabled(|cx| {
                Enabled::when(
                    scene(cx.editor).is_some_and(|s| s.recovery.can_snapshot(true).is_ok()),
                    "The scene has nothing to keep yet.",
                )
            }),
        Action::new(MAXIMIZE, "Maximize Viewport", SCENE, "level.show")
            .icon(Icon::Perspective)
            .keys(const { &[Chord::new(Key::F11)] })
            .scope(TAB)
            .args(|_| view("maximize", "toggle"))
            .enabled(|cx| Enabled::when(cx.kind == crate::LEVEL, "Only the Level tab has a viewport."))
            .checked(|cx| crate::doc::maximized(cx.editor)),
        // building
        Action::new(BUILD_LEVEL, "Build Level", BUILD, "level.build")
            .icon(Icon::BuildAll)
            .scope(TAB)
            .enabled(can_build),
        Action::new(REBUILD_ALL, "Rebuild All", BUILD, "level.build")
            .scope(TAB)
            .args(|_| pairs(&[("force", json!(true))]))
            .enabled(can_build),
        Action::new(BUILD_DRAFT, "Build at Draft Quality", BUILD, "level.build")
            .scope(TAB)
            .args(|_| pairs(&[("quality", json!("draft"))]))
            .enabled(can_build),
        Action::new(BUILD_FINAL, "Build at Final Quality", BUILD, "level.build")
            .scope(TAB)
            .args(|_| pairs(&[("quality", json!("final"))]))
            .enabled(can_build),
        Action::new(VALIDATE, "Validate Scene", BUILD, "level.validate")
            .icon(Icon::Validate)
            .scope(TAB)
            .enabled(with_scene)
            .reply(show_validation),
        Action::new(SHOW_PROBLEMS, "Show Problems", BUILD, "ui.panel")
            .icon(Icon::Problems)
            .scope(TAB)
            .args(|_| pairs(&[("action", json!("open")), ("id", json!(crate::problems::PANEL))]))
            .enabled(with_project),
        Action::new(CLEAR_PROBLEMS, "Clear Problems", BUILD, "level.problems")
            .icon(Icon::Clear)
            .scope(TAB)
            .args(|_| pairs(&[("clear", json!(true))]))
            .enabled(with_scene)
            .reply(problems_cleared),
        Action::new(EXPORT_OBJ, "Export Scene as OBJ...", BUILD, "level.show")
            .scope(TAB)
            .args(|_| view("dialog", "export_obj"))
            .enabled(with_objects),
        Action::new(EXPORT_SELECTION_OBJ, "Export Selection as OBJ...", BUILD, "level.show")
            .scope(TAB)
            .args(|_| view("dialog", "export_selection_obj"))
            .enabled(with_selection),
        Action::new(COMPILE_FOLDER, "Compile a Level Folder...", BUILD, "level.show")
            .scope(TAB)
            .args(|_| view("dialog", "compile_folder"))
            .enabled(|cx| if running(cx.editor) { Enabled::Off(BUSY) } else { Enabled::On }),
        // playing
        Action::new(PLAY, "Play", BUILD, "level.play")
            .icon(Icon::Play)
            .keys(const { &[Chord::alt(Key::P)] })
            .scope(TAB)
            .args(|_| view("action", "start"))
            .enabled(can_play),
        Action::new(PLAY_CLEAN, "Play Clean", BUILD, "level.play")
            .icon(Icon::PlayClean)
            .scope(TAB)
            .args(|_| pairs(&[("action", json!("start")), ("clean", json!(true))]))
            .enabled(|cx| if running(cx.editor) { Enabled::Off(BUSY) } else { with_project(cx) }),
        Action::new(PLAY_WITHOUT_BUILDING, "Play Without Building", BUILD, "level.play")
            .icon(Icon::PlayGame)
            .scope(TAB)
            .args(|_| pairs(&[("action", json!("start")), ("build", json!(false))]))
            .enabled(can_play),
        Action::new(STOP, "Stop", BUILD, "level.play")
            .icon(Icon::Stop)
            .scope(TAB)
            .args(|_| view("action", "stop"))
            .enabled(can_stop),
        Action::new(RELOAD_CONFIGS, "Reload Game Configs", BUILD, "level.play")
            .icon(Icon::ReloadConfigs)
            .scope(TAB)
            .args(|_| view("action", "reload_configs"))
            .enabled(|cx| {
                if running(cx.editor) {
                    Enabled::Off(BUSY)
                } else {
                    Enabled::when(game_root(cx.editor).is_some(), "No game is linked.")
                }
            }),
        Action::new(REMOVE_BUILD_FILES, "Remove Build Files...", BUILD, "level.clean")
            .scope(TAB)
            .args(|_| pairs(&[("dry_run", json!(true))]))
            .enabled(can_remove_build_files)
            .reply(confirm_build_files),
        Action::new(GAME_FOLDER, "Open Game Folder", BUILD, "ui.reveal")
            .icon(Icon::GameFolder)
            .scope(TAB)
            .args(|cx| game_root(cx.editor).map_or_else(Args::new, |r| pairs(&[("path", Value::String(r))])))
            .enabled(|cx| Enabled::when(game_root(cx.editor).is_some(), "No game is linked.")),
    ];
    v.extend(step_actions());
    v
}

/// Where the level's actions stand in the caption's File menu: group, order, id, label.
pub const FILE_MENU: &[(i32, i32, &str, Option<&str>)] =
    &[(10, 0, NEW_SCENE, None), (10, 1, OPEN_SCENE, None), (20, 1, SAVE_AS, None)];

/// Plugs the actions and the File menu's rows into the shell.
pub(crate) fn install(b: &mut gz_app::ShellBuilder) {
    for a in actions() {
        b.action(a);
    }
    for (group, order, id, label) in FILE_MENU {
        b.menu_action("File", *group, *order, id, *label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selection_turns_the_rows_on() {
        assert_eq!(gate(None, Needs::Selection), Enabled::Off("Open or create a scene first."));
        assert_eq!(gate(Some((5, 0)), Needs::Selection), Enabled::Off("Select objects first."));
        assert!(gate(Some((5, 2)), Needs::Selection).on());
        assert_eq!(gate(Some((0, 0)), Needs::Objects), Enabled::Off("The scene is empty."));
        assert!(gate(Some((1, 0)), Needs::Objects).on() && gate(Some((0, 0)), Needs::Scene).on());
    }

    #[test]
    fn ids_are_unique_and_named_for_the_level() {
        let all = actions();
        let mut ids: Vec<&str> = all.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
        assert!(all.iter().all(|a| a.id.starts_with("level.")));
        assert!(!all.iter().any(|a| [HIDE_SELECTED, SHOW_ALL, GROUP].contains(&a.id)));
        // every step of the build has its action, titled by what it makes
        for s in Step::ORDER {
            let a = all.iter().find(|a| a.id == step_action(s)).expect("a step's action");
            assert!(a.title.starts_with("Build "), "{}", a.title);
        }
    }

    #[test]
    fn no_two_level_actions_share_a_default_chord() {
        // the level's and the viewport's: both live on the Level tab
        let all: Vec<gz_app::Action> =
            actions().into_iter().chain(gz_ui_viewport::actions::actions()).collect();
        let mut chords: Vec<(String, &str)> =
            all.iter().flat_map(|a| a.keys.iter().map(move |c| (c.label(), a.id))).collect();
        chords.sort();
        for w in chords.windows(2) {
            assert!(w[0].0 != w[1].0, "{} and {} both default to {}", w[0].1, w[1].1, w[0].0);
        }
    }
}
