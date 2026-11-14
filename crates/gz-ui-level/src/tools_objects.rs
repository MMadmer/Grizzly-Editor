//! The Zones mode's panel: the visibility cells (sectors) and their portals, for the author who
//! wants to cut the level up by hand. Its first line says what the build does by itself - one
//! cell for a level without any, the meshes no cell holds into the default cell, the portals
//! the cells lack - so an author knows they may never need this mode.
//!
//! The cells and the portals are lists with an eye per row; a click selects the row's cell or
//! portal. New Cell From Selection, Compute Portals and Validate are on the surface; what a click
//! on a mesh does, the portals' orientation and the level's bound are under More. The panel
//! posts `scene.zones` and `scene.shape` (the level bound); it never changes the scene itself.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Rect, Sense, Ui, Vec2, pos2};
use gz_editor::{Editor, Value, json};
use gz_scene::apply::SelectMode;
use gz_scene::objects::SHAPE_LEVEL_BOUND;
use gz_scene::objtools::sector::DEFAULT_SECTOR;
use gz_scene::{ObjClass, ObjData, ObjId, SceneCmd, SceneService, SceneState};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Button, Font, Icon, Kind, PropFlags, PropertyGrid, Section, Switch, Tone, icons, metrics, palette,
};

use crate::local::{args, confirm, local_eye_toggle, plural, removal, run};

/// What the first line says: the build makes the cells by itself.
pub const BUILD_LINE: &str = "The build makes the cells and their portals by itself: one cell for a level without any, the meshes no cell holds into the default cell, and the portals the cells lack. Draw cells here only to cut the level up by hand.";

fn zones(editor: &Arc<Editor>, action: &str, extra: &[(&str, Value)]) {
    let mut all = vec![("action", json!(action))];
    all.extend(extra.iter().cloned());
    run(editor, "scene.zones", args(&all));
}

/// The selected objects of a kind, by name: read off the selection, not the whole scene.
fn selected_names(snap: &SceneState, class: ObjClass) -> Vec<String> {
    snap.selection
        .iter()
        .filter_map(|id| snap.get(*id))
        .filter(|o| o.class == class)
        .map(|o| o.name.to_string())
        .collect()
}

/// The Zones mode's panel.
pub(crate) fn zones_panel(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
    let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else { return };
    text::status(ui, Tone::Muted, BUILD_LINE);
    ui.add_space(6.0);
    let selected_objects = snap
        .selection
        .iter()
        .filter_map(|id| snap.get(*id))
        .filter(|o| o.class == ObjClass::SceneObject)
        .count();
    ui.horizontal_wrapped(|ui| {
        if Button::new("New Cell From Selection")
            .kind(Kind::Primary)
            .icon(Icon::Sector)
            .enabled(selected_objects > 0, "Select the objects whose meshes make the cell first")
            .tip("Make a cell of the selected objects' static meshes")
            .show(ui)
            .clicked()
        {
            zones(editor, "create", &[]);
        }
        if Button::new("Compute Portals")
            .tip("Put the meshes no cell holds into the default cell and compute the portals the cells lack - what the build does by itself; the portals drawn by hand stay")
            .show(ui)
            .clicked()
        {
            zones(editor, "compute", &[]);
        }
        if Button::new("Validate").icon(Icon::Validate).tip("Check the cells as the build checks them").show(ui).clicked() {
            zones(editor, "validate", &[]);
        }
    });
    ui.add_space(6.0);
    let cells: Vec<(ObjId, String, usize, bool)> = snap
        .of_class(ObjClass::Sector)
        .map(|o| {
            let meshes = match &o.data {
                ObjData::Sector(s) => s.items.len(),
                _ => 0,
            };
            (o.id, o.name.to_string(), meshes, o.flags.visible)
        })
        .collect();
    let head = Section::new("zones.cells", "Cells").count(&cells.len().to_string()).show(ui);
    if head.open {
        if cells.is_empty() {
            text::status(ui, Tone::Muted, "None: the build makes the level one cell.");
        }
        for (id, name, meshes, visible) in &cells {
            zone_row(ui, editor, &svc, snap, *id, name, &plural(*meshes, "mesh"), *visible, Icon::Sector);
        }
    }
    let portals: Vec<(ObjId, String, String, bool)> = snap
        .of_class(ObjClass::Portal)
        .map(|o| {
            let between = match &o.data {
                ObjData::Portal(p) => format!("{} \u{2194} {}", p.front, p.back),
                _ => String::new(),
            };
            (o.id, o.name.to_string(), between, o.flags.visible)
        })
        .collect();
    let head = Section::new("zones.portals", "Portals").count(&portals.len().to_string()).show(ui);
    if head.open {
        if portals.is_empty() {
            text::status(ui, Tone::Muted, "None: the build computes the portals between the cells.");
        }
        for (id, name, between, visible) in &portals {
            zone_row(ui, editor, &svc, snap, *id, name, between, *visible, Icon::Portal);
        }
    }
    let sel_cells = selected_names(snap, ObjClass::Sector);
    let sel_portals = selected_names(snap, ObjClass::Portal);
    let mut gone = sel_cells.clone();
    gone.extend(sel_portals.iter().cloned());
    if Button::new("Remove Selected...")
        .kind(Kind::GhostDanger)
        .enabled(!gone.is_empty(), "Select cells or portals first")
        .tip("Remove the selected cells (with their portals) and portals")
        .show(ui)
        .clicked()
    {
        let ed = editor.clone();
        let names: Vec<Value> = gone.iter().map(|n| Value::String(n.clone())).collect();
        confirm(
            ui.ctx(),
            editor,
            removal(
                "remove_zones",
                "Remove Cells and Portals",
                "Remove these cells and portals?",
                gone,
                "Remove",
            ),
            move || zones(&ed, "remove", &[("objects", Value::Array(names))]),
        );
    }
    ui.add_space(6.0);
    Section::new("zones.more", "More")
        .default_open(false)
        .body(ui, |ui| more(ui, editor, snap, &sel_cells, &sel_portals));
}

/// A cell's or a portal's row: its eye, its glyph, its name and what it holds; a click selects it.
#[allow(clippy::too_many_arguments)]
fn zone_row(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    svc: &SceneService,
    snap: &SceneState,
    id: ObjId,
    name: &str,
    detail: &str,
    visible: bool,
    icon: Icon,
) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), metrics::ROW), Sense::hover());
    let rid = Id::unique(("gz-ui-level.zone_row", id.0));
    let resp = ui.interact(rect, rid, Sense::click());
    if snap.is_selected(id) {
        ui.painter().rect_filled(rect, CornerRadius::same(3), p.selection_focused);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), p.hover);
    }
    let eye = Rect::from_min_size(rect.min, Vec2::splat(metrics::ROW));
    if local_eye_toggle(
        ui,
        rid.with("eye"),
        eye,
        visible,
        if visible { "Hide in the viewport" } else { "Show in the viewport" },
    ) {
        svc.store.post(SceneCmd::Show { ids: vec![id], visible: !visible }, editor.ui_origin());
    }
    let x = eye.max.x + 4.0;
    icons::draw(
        ui.painter(),
        icon,
        Rect::from_min_size(pos2(x, rect.center().y - 7.0), Vec2::splat(14.0)),
        p.text_secondary,
    );
    let detail_w = (rect.width() * 0.4).min(text::width(ui.ctx(), &Font::Small.id(), detail) + 4.0);
    text::paint(
        ui.painter(),
        &Font::Body.id(),
        pos2(x + 20.0, rect.center().y),
        Align2::LEFT_CENTER,
        p.text,
        name,
        Some((rect.max.x - x - 24.0 - detail_w).max(0.0)),
        Clip::Middle,
    );
    text::paint(
        ui.painter(),
        &Font::Small.id(),
        pos2(rect.max.x - 4.0, rect.center().y),
        Align2::RIGHT_CENTER,
        p.text_muted,
        detail,
        Some(detail_w),
        Clip::Middle,
    );
    if resp.clicked() {
        let mode = if ui.input(|i| i.modifiers.command) { SelectMode::Toggle } else { SelectMode::Set };
        svc.store.post(SceneCmd::Select { ids: vec![id], mode }, editor.ui_origin());
    }
}

fn more(ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState, sel_cells: &[String], sel_portals: &[String]) {
    let m = snap.tools.obj.sector;
    let p = palette(ui.ctx());
    text::label(ui, "A Click on a Mesh", Font::Strong, p.text, Clip::End);
    PropertyGrid::new("tools").show(ui, |g| {
        g.row("Does", "What a click on a mesh in the viewport does in this mode", PropFlags::NONE, 0, |ui| {
            let mut cur = if m.create_single {
                1
            } else if m.create_multiple {
                2
            } else {
                0
            };
            if gz_ui::segmented(ui, &mut cur, &["Edit the Cell", "New Cell", "New Cells per Object"], None) {
                let click = ["edit", "new_from_mesh", "new_from_object"][cur.min(2)];
                zones(editor, "mode", &[("click", json!(click))]);
            }
        });
        g.row(
            "Meshes",
            "Whether a click adds the mesh to the selected cell or takes it out",
            PropFlags::NONE,
            0,
            |ui| {
                let mut cur = usize::from(!m.mesh_add);
                if gz_ui::segmented(ui, &mut cur, &["Add", "Take Out"], None) {
                    zones(editor, "mode", &[("meshes", json!(if cur == 0 { "add" } else { "remove" }))]);
                }
            },
        );
    });
    let creating = m.create_single || m.create_multiple;
    let mut boxed = m.box_pick && !creating;
    if Switch::new(&mut boxed, "Pick by a Dragged Box")
        .enabled(!creating, "Not while a click makes new cells")
        .show(ui)
        .changed()
    {
        zones(editor, "mode", &[("box_pick", Value::Bool(boxed))]);
    }
    ui.add_space(6.0);
    let one = sel_cells.len() == 1;
    let has_default = snap.find(ObjClass::Sector, DEFAULT_SECTOR).is_some();
    ui.horizontal_wrapped(|ui| {
        if Button::new("Take In What Is Inside")
            .enabled(one, if sel_cells.is_empty() { "Select a cell first" } else { "Select only one cell" })
            .tip("Every static mesh inside the selected cell's volume joins it")
            .show(ui)
            .clicked()
            && let Some(name) = sel_cells.first()
        {
            zones(editor, "assign", &[("sector", json!(name)), ("inside", Value::Bool(true))]);
        }
        if Button::new("Turn Portals Round")
            .enabled(!sel_portals.is_empty(), "Select portals first")
            .tip("Turn the selected portals the other way")
            .show(ui)
            .clicked()
        {
            zones(editor, "invert", &[]);
        }
        if Button::new("Remove the Default Cell...")
            .kind(Kind::GhostDanger)
            .enabled(has_default, "There is no default cell")
            .tip("Remove the default cell")
            .show(ui)
            .clicked()
        {
            let ed = editor.clone();
            confirm(
                ui.ctx(),
                editor,
                removal(
                    "remove_default_sector",
                    "Remove the Default Cell",
                    "Remove the default cell and its portals?",
                    vec![DEFAULT_SECTOR.to_owned()],
                    "Remove",
                ),
                move || zones(&ed, "remove", &[("objects", json!([DEFAULT_SECTOR]))]),
            );
        }
    });
    ui.add_space(6.0);
    text::label(ui, "Level Bound", Font::Strong, p.text, Clip::End);
    let t = snap.tools.obj;
    let mut edit = t.edit_level_bound;
    if Switch::new(&mut edit, "Edit the Level Bound")
        .tip("Show the box the level is bound by, to move and size it like an object")
        .show(ui)
        .changed()
    {
        run(editor, "scene.shape", args(&[("action", json!("level_bound")), ("on", Value::Bool(edit))]));
    }
    let has = snap
        .of_class(ObjClass::Shape)
        .any(|o| matches!(&o.data, ObjData::Shape(s) if s.usage == SHAPE_LEVEL_BOUND));
    if Button::new("Fit to the Level")
        .enabled(t.edit_level_bound && has, "Turn Edit the Level Bound on first")
        .tip("Make the level bound again round what the scene holds")
        .show(ui)
        .clicked()
    {
        run(
            editor,
            "scene.shape",
            args(&[("action", json!("level_bound")), ("recalculate", Value::Bool(true))]),
        );
    }
}

/// What `level.view` answers of the Zones panel.
#[must_use]
pub fn state(snap: &SceneState) -> Value {
    let names = |c: ObjClass| snap.of_class(c).map(|o| o.name.to_string()).collect::<Vec<_>>();
    json!({ "first_line": BUILD_LINE, "cells": names(ObjClass::Sector), "portals": names(ObjClass::Portal) })
}
