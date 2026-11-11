//! The Tools panel's forms of the Sector, Portal, Way, Shape, Light and Fog Volume tools
//! (`UISectorTool`, `UIPortalTool`, `UIWayTool`, `UIShapeTool`, `UILightTool`,
//! `UIFogVolTool`). A form reads the scene's snapshot and posts `sector_tool`, `portal_tool`,
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
    snap.of_class(class).filter(|o| snap.is_selected(o.id) && o.flags.visible).map(|o| o.name.to_string()).collect()
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
            gz_ui::text::status(ui, Tone::Muted, "Meshes are added to the selected sector with Place in the viewport.");
            if local_button_flow(ui, &[LocalButton::new("Place", "A click in the viewport adds or removes meshes")]) == Some(0) {
                gz_ui_viewport::set_placing(editor, true);
            }
        }
        if let Some(v) = local_switch_row(ui, m.create_single, "New Sector From Mesh", "A mesh picked in the viewport becomes a sector of its own", true, "") {
            run(editor, "sector_tool", "set", &[("create_single", Value::Bool(v))]);
        }
        if let Some(v) = local_switch_row(
            ui,
            m.create_multiple,
            "New Sectors From Objects",
            "Each object picked in the viewport becomes a sector of its own",
            true,
            "",
        ) {
            run(editor, "sector_tool", "set", &[("create_multiple", Value::Bool(v))]);
        }
        PropertyGrid::new("tools").show(ui, |g| {
            g.row("Meshes", "What a mesh picked in the viewport does to the selected sector", PropFlags::NONE, 0, |ui| {
                let mut cur = usize::from(!m.mesh_add);
                if gz_ui::segmented(ui, &mut cur, &["Add", "Remove"], None) {
                    run(editor, "sector_tool", "set", &[("mesh_mode", Value::String(if cur == 0 { "add" } else { "remove" }.into()))]);
                }
            });
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
            run(editor, "sector_tool", "set", &[("box_pick", Value::Bool(v))]);
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
                LocalButton::new("Create Default", "Create the sector that holds whatever no other sector does")
                    .when(!has_default, "The default sector is there already"),
            ],
        ) {
            Some(0) => run(editor, "sector_tool", "validate", &[]),
            Some(1) => run(editor, "sector_tool", "capture_volume", &[]),
            Some(2) => run(editor, "sector_tool", "create_default", &[]),
            _ => {}
        }
        if danger(ui, "Remove Default...", "Remove the default sector", has_default, "There is no default sector") {
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
            confirm(ui, editor, spec, move || run(&ed, "sector_tool", "remove_default", &[]));
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_sector_more", "More", false) {
        local_button_flow(
            ui,
            &[LocalButton::new("Distribute Objects", "Hand the objects inside the selected sector to it")
                .when(false, "Distribute Objects is not available in this version")],
        );
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
        run(editor, "portal_tool", "invert", &[]);
    }
    ui.horizontal_wrapped(|ui| {
        if danger(ui, "Compute All...", "Remove every portal and compute them from the sectors again", true, "") {
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
            confirm(ui, editor, spec, move || run(&ed, "portal_tool", "compute_all", &[]));
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
            confirm(ui, editor, spec, move || run(&ed, "portal_tool", "compute_selected", &[]));
        }
    });
    if gz_ui_app::local::local_section(ui, "tool_portal_more", "More", false)
        && danger(ui, "Remove Duplicates...", "Remove the portals that repeat another one", portals > 0, "There are no portals")
    {
        let spec = ConfirmSpec {
            id: "remove_duplicate_portals".into(),
            title: "Remove Duplicate Portals".into(),
            headline: "Remove every portal that repeats another one?".into(),
            items: vec![format!("{} in the level are looked at", plural(portals, "portal"))],
            total: 1,
            consequence: "Undo brings them back.".into(),
            action: "Remove".into(),
            destructive: true,
        };
        let ed = editor.clone();
        confirm(ui, editor, spec, move || run(&ed, "portal_tool", "remove_duplicates", &[]));
    }
}

// ---- Way ----------------------------------------------------------------------------------------

pub(crate) fn way_tool(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
    let t = snap.tools.obj;
    if gz_ui_app::local::local_section(ui, "tool_way_edit", "Edit", true) {
        PropertyGrid::new("tools").show(ui, |g| {
            g.row("Select", "Pick whole ways, or the points of a way", PropFlags::NONE, 0, |ui| {
                let mut cur = usize::from(t.way_points);
                if gz_ui::segmented(ui, &mut cur, &["Ways", "Points"], None) {
                    run(editor, "way_tool", "mode", &[("mode", Value::String(if cur == 1 { "points" } else { "ways" }.into()))]);
                }
            });
        });
        if t.way_points {
            gz_ui::text::status(ui, Tone::Muted, "With Place, a click adds a point to the selected way.");
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_way_links", "Links", true) {
        if let Some(v) = local_switch_row(ui, t.auto_link, "Auto Link", "A point added to a way links to the selected point at once", true, "") {
            run(editor, "way_tool", "auto_link", &[("on", Value::Bool(v))]);
        }
        let why = "Switch Select to Points first";
        let pts = t.way_points;
        let pressed = local_button_flow(
            ui,
            &[
                LocalButton::new("Create 1-Link", "Link the selected points one way, each to the next").when(pts, why),
                LocalButton::new("Create 2-Link", "Link the selected points both ways").when(pts, why),
                LocalButton::new("Convert to 1-Link", "Make the links of the selected ways one-way"),
                LocalButton::new("Convert to 2-Link", "Make the links of the selected ways two-way"),
                LocalButton::new("Invert Link", "Turn the links of the selected points around").when(pts, why),
                LocalButton::new("Remove Link", "Remove the links of the selected points").when(pts, why),
            ],
        );
        let action = match pressed {
            Some(0) => "create_1link",
            Some(1) => "create_2link",
            Some(2) => "convert_1link",
            Some(3) => "convert_2link",
            Some(4) => "invert_link",
            Some(5) => "remove_link",
            _ => return,
        };
        run(editor, "way_tool", action, &[]);
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
                    run(editor, "shape_tool", "set", &[("shape", Value::String(if cur == 0 { "sphere" } else { "box" }.into()))]);
                }
            });
        });
    }
    if gz_ui_app::local::local_section(ui, "tool_shape_attach", "Attach", true) {
        if let Some(v) = local_switch_row(ui, t.attach_on_click, "Attach on Click", "A shape clicked in the viewport joins the selected shape", true, "") {
            run(editor, "shape_tool", "set", &[("attach_on_click", Value::Bool(v))]);
            if v {
                gz_ui_viewport::set_placing(editor, true);
            }
        }
        let selected = selected_names(snap, ObjClass::Shape);
        if danger(ui, "Detach Selected...", "Split the selected shapes into their parts", !selected.is_empty(), "Select shapes first") {
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
            confirm(ui, editor, spec, move || run(&ed, "shape_tool", "detach", &[]));
        }
    }
    if gz_ui_app::local::local_section(ui, "tool_shape_bound", "Level Bound", true) {
        if let Some(v) = local_switch_row(ui, t.edit_level_bound, "Edit Level Bound", "Show the box the level is bound by and edit it", true, "") {
            run(editor, "shape_tool", "edit_level_bound", &[("on", Value::Bool(v))]);
        }
        let has = snap.of_class(ObjClass::Shape).any(|o| matches!(&o.data, ObjData::Shape(s) if s.usage == SHAPE_LEVEL_BOUND));
        if local_button_flow(
            ui,
            &[LocalButton::new("Recalculate", "Fit the level bound to the level again")
                .when(t.edit_level_bound && has, "Turn Edit Level Bound on first")],
        ) == Some(0)
        {
            run(editor, "shape_tool", "recalculate", &[]);
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
    let action = match pressed {
        Some(0) => "enable_selected",
        Some(1) => "enable_all",
        Some(2) => "disable_selected",
        Some(3) => "disable_all",
        _ => return,
    };
    run(editor, "light_tool", action, &[]);
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
        Some(0) => run(editor, "fog_tool", "group", &[]),
        Some(1) => run(editor, "fog_tool", "ungroup", &[]),
        _ => {}
    }
}
