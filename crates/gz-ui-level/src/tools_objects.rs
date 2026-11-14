//! The Tools panel's forms of the Sector, Portal, Way, Shape, Light and Fog Volume tools
//! (`UISectorTool`, `UIPortalTool`, `UIWayTool`, `UIShapeTool`, `UILightTool`,
//! `UIFogVolTool`). A form reads the scene's snapshot and posts `scene.zones`,
//! `way_tool`, `shape_tool`, `light_tool` or `fog_tool`; it never changes the scene itself. What a
//! command answers comes back as a note (its `message`) or an error.
//!
//! Widget the kit lacks, local here: `local_switch_row` (a switch with its label and tip, off and
//! saying why when it cannot be used).

use std::sync::Arc;

use egui::Ui;
use gz_editor::{Args, Editor, Value};
use gz_scene::objects::SHAPE_LEVEL_BOUND;
use gz_scene::objtools::sector::DEFAULT_SECTOR;
use gz_scene::{ObjClass, ObjData, SceneState};
use gz_ui::text::{self, Clip};
use gz_ui::{Answer, ConfirmSpec, Font, Kind, PropFlags, PropertyGrid, Tone, palette};

use crate::local::{LocalButton, local_button_flow};
use crate::tools::asker;

/// Runs a tool command off the UI thread; its `message` (or the first `messages` line) becomes a
/// note, a refusal an error.
fn run(editor: &Arc<Editor>, cmd: &'static str, action: &str, extra: &[(&str, Value)]) {
    let mut all = vec![("action", Value::String(action.to_owned()))];
    all.extend(extra.iter().cloned());
    call(editor, cmd, &all);
}

/// Sets switches of the panels, which only the window reads.
fn set_state(editor: &Arc<Editor>, f: impl FnOnce(&mut gz_scene::objtools::ObjToolState) + Send + 'static) {
    let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<gz_scene::SceneService>()) else {
        return;
    };
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let _ = gz_scene::objtools::set_state(&svc, origin, f).await;
    });
}

/// Runs a command with these arguments off the UI thread, as [`run`] does.
fn call(editor: &Arc<Editor>, cmd: &'static str, extra: &[(&str, Value)]) {
    let mut args = Args::new();
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
        if let Some(m) = reply.get("message").and_then(Value::as_str) {
            ed.notices.note(m.to_owned());
        } else if reply.get("valid").and_then(Value::as_bool) == Some(true) {
            ed.notices.note("The sectors are valid.");
        } else if let Some(p) = reply.get("problems").and_then(Value::as_array).and_then(|p| p.first()) {
            ed.notices.error(format!("{} (the Output Log lists every problem)", p.as_str().unwrap_or("")));
        } else if let Some(n) = reply.get("note").and_then(Value::as_str) {
            ed.notices.note(n.to_owned());
        }
    });
}

/// A switch with its label: off and saying why when `enabled` is false.
fn local_switch_row(ui: &mut Ui, on: bool, label: &str, tip: &str, enabled: bool, why: &str) -> Option<bool> {
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

fn confirm(ui: &Ui, editor: &Arc<Editor>, spec: ConfirmSpec, then: impl FnOnce() + Send + 'static) {
    let line = gz_ui::dialog::confirm(ui.ctx(), asker(editor), spec, move |answer| {
        if answer == Answer::Yes {
            then();
        }
    });
    if let Some(line) = line {
        editor.log.plain(line);
    }
}

fn selected_names(snap: &SceneState, class: ObjClass) -> Vec<String> {
    snap.of_class(class)
        .filter(|o| snap.is_selected(o.id) && o.flags.visible)
        .map(|o| o.name.to_string())
        .collect()
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

// ---- Sector -------------------------------------------------------------------------------------

pub(crate) fn sector_tool(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
    let m = snap.tools.obj.sector;
    let placing = gz_ui_viewport::placing(editor);
    if gz_ui_app::local::local_section(ui, "tool_sector_edit", "Edit", true) {
        if !placing {
            gz_ui::text::status(
                ui,
                Tone::Muted,
                "Meshes are added to the selected sector with Place in the viewport.",
            );
            if local_button_flow(
                ui,
                &[LocalButton::new("Place", "A click in the viewport adds or removes meshes")],
            ) == Some(0)
            {
                gz_ui_viewport::set_placing(editor, true);
            }
        }
        if let Some(v) = local_switch_row(
            ui,
            m.create_single,
            "New Sector From Mesh",
            "A mesh picked in the viewport becomes a sector of its own",
            true,
            "",
        ) {
            run(
                editor,
                "scene.zones",
                "mode",
                &[("click", Value::String(if v { "new_from_mesh" } else { "edit" }.into()))],
            );
        }
        if let Some(v) = local_switch_row(
            ui,
            m.create_multiple,
            "New Sectors From Objects",
            "Each object picked in the viewport becomes a sector of its own",
            true,
            "",
        ) {
            run(
                editor,
                "scene.zones",
                "mode",
                &[("click", Value::String(if v { "new_from_object" } else { "edit" }.into()))],
            );
        }
        PropertyGrid::new("tools").show(ui, |g| {
            g.row(
                "Meshes",
                "What a mesh picked in the viewport does to the selected sector",
                PropFlags::NONE,
                0,
                |ui| {
                    let mut cur = usize::from(!m.mesh_add);
                    if gz_ui::segmented(ui, &mut cur, &["Add", "Remove"], None) {
                        run(
                            editor,
                            "scene.zones",
                            "mode",
                            &[("meshes", Value::String(if cur == 0 { "add" } else { "remove" }.into()))],
                        );
                    }
                },
            );
        });
        let creating = m.create_single || m.create_multiple;
        if let Some(v) = local_switch_row(
            ui,
            m.box_pick && !creating,
            "Box Pick",
            "Pick the meshes inside a dragged box",
            !creating,
            "Not while a new sector is being made",
        ) {
            run(editor, "scene.zones", "mode", &[("box_pick", Value::Bool(v))]);
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_sector_commands", "Commands", true) {
        let has_default = snap.find(ObjClass::Sector, DEFAULT_SECTOR).is_some();
        let selected = selected_names(snap, ObjClass::Sector).len();
        let one_why = if selected == 0 { "Select a sector first" } else { "Select only one sector" };
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Validate", "Check the sectors and report what is wrong with them"),
                LocalButton::new("Capture Volume", "Grow the selected sector over the objects inside it")
                    .when(selected == 1, one_why),
                LocalButton::new(
                    "Compute",
                    "Put the meshes no sector holds into the default sector and compute the portals - what the build does by itself",
                ),
            ],
        ) {
            Some(0) => run(editor, "scene.zones", "validate", &[]),
            Some(1) => {
                if let Some(name) = selected_names(snap, ObjClass::Sector).first() {
                    run(
                        editor,
                        "scene.zones",
                        "assign",
                        &[("sector", Value::String(name.clone())), ("inside", Value::Bool(true))],
                    );
                }
            }
            Some(2) => run(editor, "scene.zones", "compute", &[]),
            _ => {}
        }
        if danger(
            ui,
            "Remove Default...",
            "Remove the default sector",
            has_default,
            "There is no default sector",
        ) {
            let portals = gz_scene::objtools::sector::sector_portals(snap, DEFAULT_SECTOR).len();
            let mut items = vec!["Sector_DEFAULT".to_owned()];
            if portals > 0 {
                items.push(format!("{} of it", plural(portals, "portal")));
            }
            let spec = ConfirmSpec {
                id: "remove_default_sector".into(),
                title: "Remove Default Sector".into(),
                headline: "Remove the default sector?".into(),
                total: items.len(),
                items,
                consequence: "Undo brings it back.".into(),
                action: "Remove".into(),
                destructive: true,
            };
            let ed = editor.clone();
            confirm(ui, editor, spec, move || {
                run(
                    &ed,
                    "scene.zones",
                    "remove",
                    &[("objects", Value::Array(vec![Value::String(DEFAULT_SECTOR.into())]))],
                )
            });
        }
    }
}

// ---- Portal -------------------------------------------------------------------------------------

pub(crate) fn portal_tool(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
    if !gz_ui_app::local::local_section(ui, "tool_portal_commands", "Commands", true) {
        return;
    }
    let portals = snap.count(ObjClass::Portal);
    let sectors = selected_names(snap, ObjClass::Sector);
    let any_portal_selected = !selected_names(snap, ObjClass::Portal).is_empty();
    if local_button_flow(
        ui,
        &[LocalButton::new("Invert Orientation", "Turn the selected portals around")
            .when(any_portal_selected, "Select portals first")],
    ) == Some(0)
    {
        run(editor, "scene.zones", "invert", &[]);
    }
    ui.horizontal_wrapped(|ui| {
        if danger(
            ui,
            "Compute All...",
            "Remove every portal and compute them from the sectors again",
            true,
            "",
        ) {
            let spec = ConfirmSpec {
                id: "compute_all_portals".into(),
                title: "Compute All Portals".into(),
                headline: "Remove every portal and compute them again?".into(),
                items: vec![format!("{} in the level", plural(portals, "portal"))],
                total: 1,
                consequence: "The portals are replaced by the computed ones. Undo brings them back.".into(),
                action: "Compute".into(),
                destructive: true,
            };
            let ed = editor.clone();
            confirm(ui, editor, spec, move || run(&ed, "scene.zones", "compute", &[]));
        }
        if danger(
            ui,
            "Compute Selected...",
            "Compute the portals of the selected sectors again",
            sectors.len() >= 2,
            "Select two sectors or more first",
        ) {
            let total = sectors.len();
            let spec = ConfirmSpec {
                id: "compute_selected_portals".into(),
                title: "Compute Selected Portals".into(),
                headline: "Remove the portals of these sectors and compute them again?".into(),
                items: sectors.iter().take(10).cloned().collect(),
                total,
                consequence: "Their portals are replaced by the computed ones. Undo brings them back.".into(),
                action: "Compute".into(),
                destructive: true,
            };
            let ed = editor.clone();
            let names: Vec<Value> = sectors.iter().map(|n| Value::String(n.clone())).collect();
            confirm(ui, editor, spec, move || {
                run(&ed, "scene.zones", "compute", &[("objects", Value::Array(names))])
            });
        }
    });
}

// ---- Way ----------------------------------------------------------------------------------------

pub(crate) fn way_tool(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
    let t = snap.tools.obj;
    if gz_ui_app::local::local_section(ui, "tool_way_edit", "Edit", true) {
        gz_ui::text::status(
            ui,
            Tone::Muted,
            "With Place, a click adds a point to the selected way path; its points are picked and moved like objects.",
        );
    }
    if gz_ui_app::local::local_section(ui, "tool_way_links", "Links", true) {
        if let Some(v) = local_switch_row(
            ui,
            t.auto_link,
            "Auto Link",
            "A point added to a way links to the selected point at once",
            true,
            "",
        ) {
            run(editor, "scene.way", "auto_link", &[("on", Value::Bool(v))]);
        }
        let why = "Select points of the way path first";
        let pts = snap.of_class(ObjClass::Way).any(|o| {
            snap.is_selected(o.id)
                && matches!(&o.data, ObjData::Way(w) if w.points.iter().any(|p| p.selected))
        });
        let pressed = local_button_flow(
            ui,
            &[
                LocalButton::new("Create 1-Link", "Link the selected points one way, each to the next")
                    .when(pts, why),
                LocalButton::new("Create 2-Link", "Link the selected points both ways").when(pts, why),
                LocalButton::new("Convert to 1-Link", "Make the links of the selected ways one-way"),
                LocalButton::new("Convert to 2-Link", "Make the links of the selected ways two-way"),
                LocalButton::new("Invert Link", "Turn the links of the selected points around")
                    .when(pts, why),
                LocalButton::new("Remove Link", "Remove the links of the selected points").when(pts, why),
            ],
        );
        let action = match pressed {
            Some(0) => "link_one_way",
            Some(1) => "link_both_ways",
            Some(2) => "make_one_way",
            Some(3) => "make_both_ways",
            Some(4) => "invert_links",
            Some(5) => "unlink",
            _ => return,
        };
        run(editor, "scene.way", action, &[]);
    }
}

// ---- Shape --------------------------------------------------------------------------------------

pub(crate) fn shape_tool(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
    let t = snap.tools.obj;
    if gz_ui_app::local::local_section(ui, "tool_shape_add", "Add Shapes", true) {
        PropertyGrid::new("tools").show(ui, |g| {
            g.row("Shape", "What a click in the viewport adds", PropFlags::NONE, 0, |ui| {
                let mut cur = usize::from(!t.shape_sphere);
                if gz_ui::segmented(ui, &mut cur, &["Sphere", "Box"], None) {
                    set_state(editor, move |st| st.shape_sphere = cur == 0);
                }
            });
        });
    }
    if gz_ui_app::local::local_section(ui, "tool_shape_attach", "Attach", true) {
        if let Some(v) = local_switch_row(
            ui,
            t.attach_on_click,
            "Attach on Click",
            "A shape clicked in the viewport joins the selected shape",
            true,
            "",
        ) {
            set_state(editor, move |st| st.attach_on_click = v);
            if v {
                gz_ui_viewport::set_placing(editor, true);
            }
        }
        let selected = selected_names(snap, ObjClass::Shape);
        if danger(
            ui,
            "Detach Selected...",
            "Split the selected shapes into their parts",
            !selected.is_empty(),
            "Select shapes first",
        ) {
            let total = selected.len();
            let spec = ConfirmSpec {
                id: "detach_shapes".into(),
                title: "Detach Shapes".into(),
                headline: "Split these shapes into their parts?".into(),
                items: selected.iter().take(10).cloned().collect(),
                total,
                consequence: "Each part becomes a shape of its own. Undo puts them back together.".into(),
                action: "Detach".into(),
                destructive: true,
            };
            let ed = editor.clone();
            confirm(ui, editor, spec, move || run(&ed, "scene.shape", "split", &[]));
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_shape_bound", "Level Bound", true) {
        if let Some(v) = local_switch_row(
            ui,
            t.edit_level_bound,
            "Edit Level Bound",
            "Show the box the level is bound by and edit it",
            true,
            "",
        ) {
            run(editor, "scene.shape", "level_bound", &[("on", Value::Bool(v))]);
        }
        let has = snap
            .of_class(ObjClass::Shape)
            .any(|o| matches!(&o.data, ObjData::Shape(s) if s.usage == SHAPE_LEVEL_BOUND));
        if local_button_flow(
            ui,
            &[LocalButton::new("Recalculate", "Fit the level bound to the level again")
                .when(t.edit_level_bound && has, "Turn Edit Level Bound on first")],
        ) == Some(0)
        {
            run(editor, "scene.shape", "level_bound", &[("recalculate", Value::Bool(true))]);
        }
    }
}

// ---- Light --------------------------------------------------------------------------------------

pub(crate) fn light_tool(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
    if !gz_ui_app::local::local_section(ui, "tool_light_viewport", "Viewport Lighting", true) {
        return;
    }
    let (mut on, mut all) = (0, 0);
    for o in snap.of_class(ObjClass::Light) {
        if let ObjData::Light(l) = &o.data {
            all += 1;
            on += usize::from(l.use_in_d3d);
        }
    }
    let p = palette(ui.ctx());
    text::label(ui, &format!("{on} of {all} light the viewport"), Font::Body, p.text_secondary, Clip::End);
    let pressed = local_button_flow(
        ui,
        &[
            LocalButton::new("Enable Selected", "The selected lights light the viewport"),
            LocalButton::new("Enable All", "Every light lights the viewport"),
            LocalButton::new("Disable Selected", "The selected lights stop lighting the viewport"),
            LocalButton::new("Disable All", "No light lights the viewport"),
        ],
    );
    let (all, on) = match pressed {
        Some(0) => (false, true),
        Some(1) => (true, true),
        Some(2) => (false, false),
        Some(3) => (true, false),
        _ => return,
    };
    // the lights' own property: the selected ones, or every one by name
    let mut set = serde_json::Map::new();
    set.insert("viewport".into(), Value::Bool(on));
    let mut extra = vec![("set", Value::Object(set))];
    if all {
        let names = snap.of_class(ObjClass::Light).map(|o| Value::String(o.name.to_string())).collect();
        extra.push(("objects", Value::Array(names)));
    }
    call(editor, "scene.set", &extra);
}

// ---- Fog Volume ---------------------------------------------------------------------------------

pub(crate) fn fog_tool(ui: &mut Ui, editor: &Arc<Editor>, _snap: &SceneState) {
    if !gz_ui_app::local::local_section(ui, "tool_fog_commands", "Commands", true) {
        return;
    }
    match local_button_flow(
        ui,
        &[
            LocalButton::new("Group Selected", "Make one fog volume group of the selected volumes"),
            LocalButton::new("Ungroup Current", "Break the current fog volume group up"),
        ],
    ) {
        Some(0) => call(editor, "scene.fog_group", &[]),
        Some(1) => call(editor, "scene.fog_group", &[("ungroup", Value::Bool(true))]),
        _ => {}
    }
}
