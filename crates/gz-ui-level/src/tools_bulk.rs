//! The Tools panel's forms of the painted layers: the AI map, the detail objects, the wallmarks.
//! A form reads the scene's snapshot and posts `scene.ai_map`, `scene.details` or
//! `scene.wallmarks`; it never changes the scene itself. What a command answers comes back as a
//! note or an error. The layers lie on the ground - every static object - by themselves.
//!
//! Each form keeps the common case on its surface (generate, add, clear, the brush or the mark's
//! size) and the rest under one folded More. Widget the kit lacks, local here:
//! `local_number_row` (a number of a grid row sent once the drag or the typing ends).

use std::sync::Arc;

use egui::{Context, Ui, Vec2};
use gz_editor::{Args, Editor, Value};
use gz_fmt_level::{node_flags, tool_flags};
use gz_scene::SceneState;
use gz_scene::bulk::LinkMode;
use gz_scene::bulk::wallmark::flags as wm_flags;
use gz_scene::model::SceneMode;
use gz_ui::text::{self, Clip};
use gz_ui::{Answer, ConfirmSpec, Font, Icon, Kind, PickAction, PropFlags, PropertyGrid, Tone, palette};
use rustc_hash::FxHashMap;

use crate::choose::{Chooser, Purpose};
use crate::lists::ListKey;
use crate::local::{LocalButton, local_button_flow};
use crate::tools::asker;

/// What the forms keep between frames: numbers being dragged or typed.
#[derive(Default)]
pub struct BulkPanels {
    pending: FxHashMap<&'static str, f32>,
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// Runs a tool command off the UI thread; a refusal becomes an error, `message` / `note` a note.
fn run(editor: &Arc<Editor>, cmd: &'static str, action: &str, extra: &[(&str, Value)]) {
    let mut args = Args::new();
    args.insert("action".into(), Value::String(action.to_owned()));
    for (k, v) in extra {
        args.insert((*k).into(), v.clone());
    }
    let ed = editor.clone();
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let reply = ed.call(cmd, args, origin).await;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let text = reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned();
            ed.notices.error(text);
            return;
        }
        if let Some(n) = reply.get("note").and_then(Value::as_str) {
            ed.notices.note(n.to_owned());
        } else if let Some(f) =
            reply.get("failed").and_then(Value::as_array).and_then(|f| f.first()).and_then(Value::as_str)
        {
            ed.notices.note(f.to_owned());
        } else if reply.get("stopped").and_then(Value::as_bool) == Some(true) {
            let n = reply.get("added").and_then(Value::as_u64).unwrap_or(0);
            ed.notices.note(format!("Generation stopped: the {n} nodes made so far are kept."));
        }
    });
}

fn switch_row(ui: &mut Ui, on: bool, label: &str, tip: &str, enabled: bool, why: &str) -> Option<bool> {
    let p = palette(ui.ctx());
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let mut v = on;
            let r = gz_ui::grid::switch(ui, &mut v);
            let r = if enabled { r.on_hover_text(tip) } else { r.on_disabled_hover_text(why) };
            if r.changed() {
                out = Some(v);
            }
            text::label(ui, label, Font::Body, if enabled { p.text } else { p.text_disabled }, Clip::End);
        });
    });
    out
}

fn danger(ui: &mut Ui, label: &str, tip: &str, enabled: bool, why: &str) -> bool {
    let mut b = gz_ui::Button::new(label).kind(Kind::GhostDanger).enabled(enabled, why);
    if enabled {
        b = b.tip(tip);
    }
    b.show(ui).clicked()
}

fn confirm(ctx: &Context, editor: &Arc<Editor>, spec: ConfirmSpec, then: impl FnOnce() + Send + 'static) {
    let line = gz_ui::dialog::confirm(ctx, asker(editor), spec, move |answer| {
        if answer == Answer::Yes {
            then();
        }
    });
    if let Some(line) = line {
        editor.log.plain(line);
    }
}

fn spec(id: &str, title: &str, headline: &str, items: Vec<String>, action: &str) -> ConfirmSpec {
    ConfirmSpec {
        id: id.into(),
        title: title.into(),
        headline: headline.into(),
        total: items.len(),
        items,
        consequence: "Edit > Undo brings them back.".into(),
        action: action.into(),
        destructive: true,
    }
}

impl BulkPanels {
    /// `local_number_row`: a number of a grid row, shown as it is dragged or typed and sent once
    /// that ends.
    #[allow(clippy::too_many_arguments)]
    fn local_number_row(
        &mut self,
        g: &mut gz_ui::grid::Grid<'_>,
        key: &'static str,
        label: &str,
        help: &str,
        value: f32,
        speed: f32,
        unit: &str,
        send: &mut Option<(&'static str, f32)>,
    ) {
        let pending = &mut self.pending;
        g.row(label, help, PropFlags::NONE, 0, |ui| {
            let mut v = pending.get(key).copied().unwrap_or(value);
            let r = gz_ui::grid::number(ui, &mut v, speed, unit);
            if r.changed() {
                pending.insert(key, v);
            }
            let done = r.drag_stopped() || r.lost_focus() || (r.changed() && !r.dragged() && !r.has_focus());
            if done && let Some(v) = pending.remove(key) {
                *send = Some((key, v));
            }
        });
    }
}

/// What a layer lies on: every static object, or the list it keeps.
fn ground_line(ui: &mut Ui, names: &[gz_base::SmolStr], what: &str) {
    if names.is_empty() {
        text::status(ui, Tone::Muted, &format!("{what} lies on every static object."));
    } else {
        let list = names.iter().take(3).map(|n| n.as_str()).collect::<Vec<_>>().join(", ");
        let more = if names.len() > 3 { format!(" and {} more", names.len() - 3) } else { String::new() };
        text::status(ui, Tone::Muted, &format!("{what} lies on {list}{more}"));
    }
}

// ---- AI Map ---------------------------------------------------------------------------------------

/// The AI Map tool's form.
pub(crate) fn aimap_tool(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    snap: &SceneState,
    panels: &mut BulkPanels,
    chooser: &mut Option<Chooser>,
) {
    let part = snap.bulk.ai_map.as_ref();
    let t = &snap.tools.bulk;
    let names = part.map(|a| a.header.snap_objects.clone()).unwrap_or_default();
    let (nodes, selected) = part.map_or((0, 0), |a| {
        let sel = a
            .nodes
            .iter()
            .flat_map(|tile| tile.iter())
            .filter(|n| n.flags & node_flags::SELECTED != 0)
            .count();
        (a.node_count(), sel)
    });
    let busy = editor.jobs.running_in("scene").is_some();
    let placing = gz_ui_viewport::placing(editor);
    if gz_ui_app::local::local_section(ui, "tool_ai_commands", "Commands", true) {
        ground_line(ui, &names, "The AI map");
        text::status(ui, Tone::Muted, &format!("{} · {} selected", plural(nodes, "node"), selected));
        let gen_why = "Something is being generated already";
        let can_gen = !busy;
        match local_button_flow(
            ui,
            &[
                LocalButton::new(
                    "Generate",
                    "Make the AI map over the ground, inside the AI Map Bounds volume when there is one",
                )
                .when(can_gen, gen_why),
                LocalButton::new("Grow From Selected", "Grow the AI map out from the selected nodes only")
                    .when(can_gen && selected > 0, if can_gen { "Select nodes first" } else { gen_why }),
            ],
        ) {
            Some(0) => run(editor, "scene.ai_map", "generate", &[]),
            Some(1) => run(editor, "scene.ai_map", "generate", &[("selected", Value::Bool(true))]),
            _ => {}
        }
        if let Some(v) = switch_row(
            ui,
            placing,
            "Add Nodes",
            "A click or a drag on the ground adds nodes there (the brush's size round it); Alt adds one and goes back to Select",
            true,
            "",
        ) {
            gz_ui_viewport::set_placing(editor, v);
        }
        if danger(ui, "Clear...", "Remove every node of the AI map", nodes > 0, "The AI map is empty") {
            let ed = editor.clone();
            confirm(
                ui.ctx(),
                editor,
                spec(
                    "clear_ai_map",
                    "Clear AI Map",
                    "Remove every node of the AI map?",
                    vec![plural(nodes, "node")],
                    "Clear",
                ),
                move || run(&ed, "scene.ai_map", "clear", &[]),
            );
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_ai_nodes", "Nodes", true) {
        if let Some(v) = switch_row(
            ui,
            t.ignore_constraints,
            "Ignore Constraints",
            "Link and add nodes even where the height or slope limits would refuse them",
            true,
            "",
        ) {
            run(editor, "scene.ai_map", "set", &[("ignore_constraints", Value::Bool(v))]);
        }
        if let Some(v) =
            switch_row(ui, t.auto_link, "Auto Link", "A node added links to its neighbours at once", true, "")
        {
            run(editor, "scene.ai_map", "set", &[("auto_link", Value::Bool(v))]);
        }
        let brush = part.map_or(1, |a| a.header.brush_size);
        let mut send = None;
        PropertyGrid::new("tools").show(ui, |g| {
            panels.local_number_row(
                g,
                "brush_size",
                "Brush Size",
                "How far round the click nodes are added, in patches (1 adds one node)",
                brush as f32,
                0.1,
                "",
                &mut send,
            );
        });
        if let Some((_, v)) = send {
            run(
                editor,
                "scene.ai_map",
                "set",
                &[("brush_size", Value::from(v.round().clamp(1.0, 100.0) as u32))],
            );
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_ai_links", "Links", true) {
        PropertyGrid::new("tools").show(ui, |g| {
            g.row(
                "Mode",
                "What a side button does to the links of the selected nodes",
                PropFlags::NONE,
                0,
                |ui| {
                    let mut cur = match t.link_mode {
                        LinkMode::Add => 0,
                        LinkMode::Remove => 1,
                        LinkMode::Invert => 2,
                    };
                    if gz_ui::segmented(ui, &mut cur, &["Add", "Remove", "Invert"], None) {
                        let m = ["add", "remove", "invert"][cur.min(2)];
                        run(editor, "scene.ai_map", "set", &[("mode", Value::String(m.into()))]);
                    }
                },
            );
        });
        // the sides follow the camera: up is the side the view looks towards
        let why = "Select nodes first";
        let cell = 26.0;
        let origin = ui.cursor().min;
        let sides: [(f32, f32, &str, Icon, &str); 5] = [
            (1.0, 0.0, "away", Icon::ChevronUp, "The side away from the camera"),
            (0.0, 1.0, "left", Icon::ChevronLeft, "The left side"),
            (1.0, 1.0, "all", Icon::Move, "All four sides"),
            (2.0, 1.0, "right", Icon::ChevronRight, "The right side"),
            (1.0, 2.0, "towards", Icon::ChevronDown, "The side towards the camera"),
        ];
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(cell * 3.0), egui::Sense::hover());
        for (col, row, side, icon, tip) in sides {
            let at = egui::Rect::from_min_size(origin + Vec2::new(col * cell, row * cell), Vec2::splat(cell));
            let mut child = crate::local::child_at(ui, ("ai_side", side), at);
            let tip = if selected > 0 { tip } else { why };
            if gz_ui::icon_button(&mut child, icon, tip, false, selected > 0).clicked() {
                run(
                    editor,
                    "scene.ai_map",
                    "link",
                    &[("side", Value::String(side.into())), ("camera", Value::Bool(true))],
                );
            }
        }
        let _ = rect;
        if let Some(i) = local_button_flow(
            ui,
            &[
                LocalButton::new("Select 0-Link", "Select the nodes that have no links")
                    .when(nodes > 0, "The AI map is empty"),
                LocalButton::new("Select 1-Link", "Select the nodes that have one link")
                    .when(nodes > 0, "The AI map is empty"),
                LocalButton::new("Select 2-Link", "Select the nodes that have two links")
                    .when(nodes > 0, "The AI map is empty"),
            ],
        ) {
            run(
                editor,
                "scene.ai_map",
                "select",
                &[("what", Value::String("links".into())), ("links", Value::from(i))],
            );
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_ai_more", "More", false) {
        let sel_why = "Select nodes first";
        match local_button_flow(
            ui,
            &[
                LocalButton::new(
                    "Smooth Selected",
                    "Even out the planes of the selected nodes with their neighbours'",
                )
                .when(selected > 0, sel_why),
                LocalButton::new("Reset Selected", "Lay the selected nodes level again")
                    .when(selected > 0, sel_why),
                LocalButton::new(
                    "Invert Selection",
                    "Select the nodes that are not selected, and let the others go",
                )
                .when(nodes > 0, "The AI map is empty"),
                LocalButton::new("Remove Links", "Remove the links between the selected nodes")
                    .when(selected > 1, "Select two nodes or more"),
                LocalButton::new("Invert Links", "Turn round the one-way links between the selected nodes")
                    .when(selected > 1, "Select two nodes or more"),
            ],
        ) {
            Some(0) => run(editor, "scene.ai_map", "smooth", &[]),
            Some(1) => run(editor, "scene.ai_map", "reset", &[]),
            Some(2) => run(editor, "scene.ai_map", "select", &[("what", Value::String("invert".into()))]),
            Some(3) => run(
                editor,
                "scene.ai_map",
                "link",
                &[("side", Value::String("all".into())), ("mode", Value::String("remove".into()))],
            ),
            Some(4) => run(
                editor,
                "scene.ai_map",
                "link",
                &[("side", Value::String("all".into())), ("mode", Value::String("invert".into()))],
            ),
            _ => {}
        }
        if danger(
            ui,
            "Delete Selected...",
            "Remove the selected nodes and their links",
            selected > 0,
            sel_why,
        ) {
            let ed = editor.clone();
            confirm(
                ui.ctx(),
                editor,
                spec(
                    "delete_ai_nodes",
                    "Delete Nodes",
                    "Remove the selected nodes of the AI map?",
                    vec![plural(selected, "node")],
                    "Delete",
                ),
                move || run(&ed, "scene.ai_map", "remove", &[]),
            );
        }
        ui.add_space(4.0);
        let p = palette(ui.ctx());
        text::label(ui, "Ignored Materials", Font::Strong, p.text, Clip::End);
        let any = !t.ignored.is_empty();
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Add...", "Pick a game material the AI map is not generated on"),
                LocalButton::new("Clear", "Take every material out of the list")
                    .when(any, "The list is empty"),
            ],
        ) {
            Some(0) => {
                *chooser = Some(Chooser::new(
                    Purpose::ToolCommand { cmd: "scene.ai_map", action: "set", arg: "ignore_materials" },
                    "Game Material",
                    Some(ListKey::GameMaterials),
                    "",
                ));
            }
            Some(1) => {
                run(editor, "scene.ai_map", "set", &[("unignore_materials", Value::Array(Vec::new()))])
            }
            _ => {}
        }
        if !any {
            text::status(ui, Tone::Muted, "None: nodes are made on every material.");
        }
        for name in &t.ignored {
            ui.horizontal(|ui| {
                if gz_ui::icon_button(ui, Icon::Close, "Take it out of the list", false, true).clicked() {
                    run(
                        editor,
                        "scene.ai_map",
                        "set",
                        &[("unignore_materials", Value::Array(vec![Value::String(name.to_string())]))],
                    );
                }
                text::label(ui, name, Font::Body, p.text, Clip::Start);
            });
        }
        ui.add_space(4.0);
        if let Some(a) = part {
            let h = &a.header;
            let mut send = None;
            PropertyGrid::new("tools").show(ui, |g| {
                panels.local_number_row(
                    g,
                    "can_up",
                    "Can Up",
                    "How far a node may lie above its neighbour, metres (0 to 10)",
                    h.params.can_up,
                    0.01,
                    "m",
                    &mut send,
                );
                panels.local_number_row(
                    g,
                    "can_down",
                    "Can Down",
                    "How far a node may lie below its neighbour, metres (0 to 10)",
                    h.params.can_down,
                    0.01,
                    "m",
                    &mut send,
                );
                panels.local_number_row(
                    g,
                    "smooth_height",
                    "Smooth Height",
                    "How close in height corners must be to be smoothed together, metres",
                    h.smooth_height,
                    0.01,
                    "m",
                    &mut send,
                );
                panels.local_number_row(
                    g,
                    "vis_radius",
                    "Visible Radius",
                    "How far round the camera nodes are drawn, metres (10 to 250)",
                    h.vis_radius,
                    0.5,
                    "m",
                    &mut send,
                );
            });
            if let Some((key, v)) = send {
                run(editor, "scene.ai_map", "set", &[(key, Value::from(v))]);
            }
            if let Some(v) = switch_row(
                ui,
                h.flags & gz_scene::bulk::aimap::flags::HIDE_NODES == 0,
                "Draw Nodes",
                "The viewport draws the nodes",
                true,
                "",
            ) {
                run(editor, "scene.ai_map", "set", &[("draw_nodes", Value::Bool(v))]);
            }
            if let Some(v) = switch_row(
                ui,
                h.flags & gz_scene::bulk::aimap::flags::SLOW_CALCULATE != 0,
                "Slow Calculate Mode",
                "Kept with the scene for the original editor; the generation here gives the same nodes either way",
                true,
                "",
            ) {
                run(editor, "scene.ai_map", "set", &[("slow_calculate", Value::Bool(v))]);
            }
        }
    }
}

// ---- Detail Objects -------------------------------------------------------------------------------

/// The Detail Objects tool's form.
pub(crate) fn detail_tool(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    snap: &SceneState,
    panels: &mut BulkPanels,
    chooser: &mut Option<Chooser>,
    want_dialog: &mut bool,
) {
    let d = snap.bulk.details.as_ref().map(|d| d.model.clone()).unwrap_or_default();
    let slots = d.slots.len();
    let objects = d.indices.objects.len();
    let selected = snap.tools.bulk.slots_selected.iter().take(slots).filter(|x| **x).count();
    let busy = editor.jobs.running_in("scene").is_some();
    let base = d.base_texture.clone().unwrap_or_default();
    if gz_ui_app::local::local_section(ui, "tool_do_commands", "Commands", true) {
        ground_line(ui, &d.snap_objects, "The detail layer");
        PropertyGrid::new("tools").show(ui, |g| {
            g.row(
                "Colour Picture",
                "A picture whose colours say what grows where: picking one lays the details from it",
                PropFlags::NONE,
                0,
                |ui| {
                    if let PickAction::Browse = gz_ui::grid::pick_field(ui, "do_base", &base, false) {
                        *chooser = Some(Chooser::new(
                            Purpose::ToolCommand {
                                cmd: "scene.details",
                                action: "import_texture",
                                arg: "texture",
                            },
                            "Colour Picture",
                            Some(ListKey::Textures),
                            &base,
                        ));
                    }
                },
            );
        });
        text::status(
            ui,
            Tone::Muted,
            &format!("{} · {} · {} selected", plural(slots, "slot"), plural(objects, "object"), selected),
        );
        let ready = !busy && !base.is_empty();
        let why = if busy { "Something is being generated already" } else { "Pick a colour picture first" };
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Import Again", "Lay the details from the colour picture again")
                    .when(ready, why),
                LocalButton::new(
                    "Detail Objects...",
                    "The objects the details are made of, and their colour indices",
                ),
            ],
        ) {
            Some(0) => run(
                editor,
                "scene.details",
                "import_texture",
                &[("texture", Value::String(base.to_string()))],
            ),
            Some(1) => *want_dialog = true,
            _ => {}
        }
        // Update Renderer is the tool's view flag: the viewport follows the details by itself
        if let Some(v) = switch_row(
            ui,
            d.flags & tool_flags::OBJECTS_DRAW != 0,
            "Show Details",
            "Show the current details in the viewport",
            true,
            "",
        ) {
            run(editor, "scene.details", "set", &[("draw", Value::Bool(v))]);
        }
        ui.horizontal_wrapped(|ui| {
            if danger(
                ui,
                "Clear Slots...",
                "Empty every detail slot of the level",
                slots > 0,
                "There are no slots",
            ) {
                let ed = editor.clone();
                confirm(
                    ui.ctx(),
                    editor,
                    spec(
                        "clear_detail_slots",
                        "Clear Detail Slots",
                        "Empty every detail slot of the level?",
                        vec![plural(slots, "detail slot")],
                        "Clear",
                    ),
                    move || run(&ed, "scene.details", "clear", &[]),
                );
            }
            if danger(
                ui,
                "Clear Details...",
                "Remove the detail objects and every slot",
                slots > 0 || objects > 0,
                "There are no details",
            ) {
                let ed = editor.clone();
                confirm(
                    ui.ctx(),
                    editor,
                    spec(
                        "clear_details",
                        "Clear Details",
                        "Remove every detail of the level?",
                        vec![plural(slots, "detail slot"), plural(objects, "detail object")],
                        "Clear",
                    ),
                    move || run(&ed, "scene.details", "clear", &[("all", Value::Bool(true))]),
                );
            }
        });
    }
    if gz_ui_app::local::local_section(ui, "tool_do_more", "More", false) {
        let mut send = None;
        PropertyGrid::new("tools").show(ui, |g| {
            panels.local_number_row(
                g,
                "density",
                "Objects per Square",
                "How densely the details are scattered: objects per square metre",
                d.density.unwrap_or(0.3),
                0.01,
                "",
                &mut send,
            );
        });
        if let Some((_, v)) = send {
            run(editor, "scene.details", "set", &[("density", Value::from(v.max(0.0)))]);
        }
    }
}

// ---- Wallmark -------------------------------------------------------------------------------------

/// The Wallmark tool's form.
pub(crate) fn wallmark_tool(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    snap: &SceneState,
    panels: &mut BulkPanels,
    chooser: &mut Option<Chooser>,
) {
    let model = snap.bulk.wallmarks.as_ref().and_then(|w| w.model.clone());
    let unreadable = snap.bulk.wallmarks.as_ref().is_some_and(|w| w.model.is_none());
    let w = model.map(|m| (*m).clone()).unwrap_or_default();
    let (count, selected) = (w.count(), w.selected_count());
    let placing = gz_ui_viewport::placing(editor);
    if unreadable {
        text::status(
            ui,
            Tone::Warning,
            "The wallmarks of this scene could not be read; they are kept as they are.",
        );
        return;
    }
    if gz_ui_app::local::local_section(ui, "tool_wm_commands", "Commands", true) {
        PropertyGrid::new("tools").show(ui, |g| {
            g.row(
                "Texture",
                "What a new mark shows",
                PropFlags::NONE,
                0,
                |ui| match gz_ui::grid::pick_field(ui, "wm_texture", &w.texture, false) {
                    PickAction::Browse => {
                        *chooser = Some(Chooser::new(
                            Purpose::ToolCommand { cmd: "scene.wallmarks", action: "set", arg: "texture" },
                            "Texture",
                            Some(ListKey::Textures),
                            &w.texture,
                        ));
                    }
                    PickAction::Clear | PickAction::None => {}
                },
            );
        });
        let mut send = None;
        PropertyGrid::new("tools").show(ui, |g| {
            panels.local_number_row(
                g,
                "width",
                "Width",
                "The width of a new mark, metres (0.01 to 10)",
                w.width,
                0.01,
                "m",
                &mut send,
            );
            panels.local_number_row(
                g,
                "height",
                "Height",
                "The height of a new mark, metres (0.01 to 10)",
                w.height,
                0.01,
                "m",
                &mut send,
            );
            panels.local_number_row(
                g,
                "rotate",
                "Rotate",
                "How a new mark is turned on the surface",
                w.rotate.to_degrees(),
                0.5,
                "°",
                &mut send,
            );
        });
        if let Some((key, v)) = send {
            run(editor, "scene.wallmarks", "set", &[(key, Value::from(v))]);
        }
        let ready = !w.texture.is_empty();
        if let Some(v) = switch_row(
            ui,
            placing,
            "Add Wallmarks",
            "A click on the level lays a mark there; Alt keeps adding; Ctrl+click moves the selected mark there",
            ready,
            "Pick a texture first",
        ) {
            gz_ui_viewport::set_placing(editor, v);
        }
        text::status(ui, Tone::Muted, &format!("{} · {} selected", plural(count, "wallmark"), selected));
        if danger(
            ui,
            "Delete Selected...",
            "Remove the selected wallmarks",
            selected > 0 && w.draws(),
            "Select wallmarks first",
        ) {
            let ed = editor.clone();
            confirm(
                ui.ctx(),
                editor,
                spec(
                    "delete_wallmarks",
                    "Delete Wallmarks",
                    "Remove the selected wallmarks?",
                    vec![plural(selected, "wallmark")],
                    "Delete",
                ),
                move || run(&ed, "scene.wallmarks", "remove", &[]),
            );
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_wm_more", "More", false) {
        PropertyGrid::new("tools").show(ui, |g| {
            g.row("Shader", "The engine shader a new mark is drawn with", PropFlags::NONE, 0, |ui| {
                if let PickAction::Browse = gz_ui::grid::pick_field(ui, "wm_shader", &w.shader, false) {
                    *chooser = Some(Chooser::new(
                        Purpose::ToolCommand { cmd: "scene.wallmarks", action: "set", arg: "shader" },
                        "Shader",
                        Some(ListKey::EngineShaders),
                        &w.shader,
                    ));
                }
            });
            g.row("Alignment", "What a new mark's up is taken from", PropFlags::NONE, 0, |ui| {
                let mut cur = usize::from(w.flags & wm_flags::AXIS_ALIGN != 0);
                if gz_ui::segmented(ui, &mut cur, &["By Camera", "By World Axis"], None) {
                    run(
                        editor,
                        "scene.wallmarks",
                        "set",
                        &[("alignment", Value::String(if cur == 1 { "world" } else { "camera" }.into()))],
                    );
                }
            });
        });
        if let Some(v) = switch_row(
            ui,
            w.draws(),
            "Draw Wallmarks",
            "The viewport draws the marks; they can be selected only while it does",
            true,
            "",
        ) {
            run(editor, "scene.wallmarks", "set", &[("draw", Value::Bool(v))]);
        }
        let why = "The marks are not drawn";
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Select All", "Select every wallmark")
                    .when(w.draws() && count > 0, if count == 0 { "There are no wallmarks" } else { why }),
                LocalButton::new("Invert Selection", "Select the marks that are not selected")
                    .when(w.draws() && count > 0, if count == 0 { "There are no wallmarks" } else { why }),
            ],
        ) {
            Some(0) => run(editor, "scene.wallmarks", "select", &[("what", Value::String("all".into()))]),
            Some(1) => run(editor, "scene.wallmarks", "select", &[("what", Value::String("invert".into()))]),
            _ => {}
        }
    }
}

// ---- the Scene menu -----------------------------------------------------------------------------

/// Select All, Deselect All, Invert Selection and Delete of the Scene menu in a painting mode:
/// `true` when it took the command.
pub(crate) fn menu_command(id: &str, editor: &Arc<Editor>, ctx: &Context, snap: &SceneState) -> bool {
    let (cmd, what): (&'static str, &str) = match snap.tools.mode {
        SceneMode::AiMap => ("scene.ai_map", "node"),
        SceneMode::Wallmarks => ("scene.wallmarks", "wallmark"),
        // the details have no selection of their own
        SceneMode::Details => return true,
        _ => return false,
    };
    let select = match id {
        "COMMAND_SELECT_ALL" => "all",
        "COMMAND_DESELECT_ALL" => "none",
        "COMMAND_INVERT_SELECTION_ALL" => "invert",
        "COMMAND_DELETE_SELECTION" => "",
        _ => return false,
    };
    if !select.is_empty() {
        run(editor, cmd, "select", &[("what", Value::String(select.into()))]);
        return true;
    }
    let selected = match snap.tools.mode {
        SceneMode::AiMap => snap.bulk.ai_map.as_ref().map_or(0, |a| {
            a.nodes.iter().flat_map(|t| t.iter()).filter(|n| n.flags & node_flags::SELECTED != 0).count()
        }),
        _ => snap.bulk.wallmarks.as_ref().and_then(|w| w.model.as_ref()).map_or(0, |m| m.selected_count()),
    };
    if selected == 0 {
        return true;
    }
    let ed = editor.clone();
    let title = if what == "node" { "Delete Nodes" } else { "Delete Wallmarks" };
    confirm(
        ctx,
        editor,
        spec(
            &format!("delete_{what}s"),
            title,
            &format!("Remove the selected {what}s?"),
            vec![plural(selected, what)],
            "Delete",
        ),
        move || run(&ed, cmd, "remove", &[]),
    );
    true
}
