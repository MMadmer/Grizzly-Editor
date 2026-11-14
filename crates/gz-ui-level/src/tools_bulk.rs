//! The Paint mode's panels: the AI map, the foliage (detail objects), the decals (wallmarks).
//! A panel reads the scene's snapshot and posts `scene.ai_map`, `scene.details` or
//! `scene.wallmarks`; it never changes the scene itself. The layers lie on the ground - every
//! static object - by themselves; the AI map is generated inside the AI Map Bounds volume when
//! the level has one.
//!
//! Each panel keeps the common case on its surface (Generate and the climb limits; the meshes,
//! the brush and Fill; the decal's texture and size) and the rest under one folded More. What a
//! panel counts over a whole layer (the AI map's selected nodes) is worked out off the UI thread.
//! Widget the kit lacks, local here: `local_number_row` (a number of a grid row sent once the
//! drag or the typing ends).

use std::sync::Arc;

use egui::{Ui, Vec2};
use gz_editor::{Editor, Value, json};
use gz_fmt_level::{node_flags, tool_flags};
use gz_query::Query;
use gz_scene::bulk::LinkMode;
use gz_scene::bulk::wallmark::flags as wm_flags;
use gz_scene::{ObjClass, ObjData, SceneState};
use gz_ui::text::{self, Clip};
use gz_ui::{Button, Font, Icon, Kind, PickAction, PropFlags, PropertyGrid, Section, Switch, Tone, palette};
use rustc_hash::FxHashMap;

use crate::choose::{Chooser, Purpose};
use crate::lists::ListKey;
use crate::local::{LocalButton, args, confirm, local_button_flow, local_danger, plural, removal, run};

/// What the panels keep between frames: numbers being dragged or typed, the mesh the foliage
/// brush paints, and the counts worked out off the UI thread.
#[derive(Default)]
pub struct BulkPanels {
    pending: FxHashMap<&'static str, f32>,
    /// The AI map's nodes and selected nodes, for the scene's revision.
    counts: Option<Query<(), (usize, usize)>>,
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

    /// The AI map's node count and how many are selected: the last answer while a new one is
    /// counted.
    fn ai_counts(&mut self, editor: &Editor, snap: &Arc<gz_store::Snap<SceneState>>) -> (usize, usize) {
        let q = self.counts.get_or_insert_with(|| Query::new(editor.exec.clone()));
        let s = snap.clone();
        q.get(&(), snap.rev, move || ai_counts(&s)).value().map_or((0, 0), |v| **v)
    }
}

/// Every node of the AI map, and the selected ones.
fn ai_counts(s: &SceneState) -> (usize, usize) {
    s.bulk.ai_map.as_ref().map_or((0, 0), |a| {
        let sel = a
            .nodes
            .iter()
            .flat_map(|tile| tile.iter())
            .filter(|n| n.flags & node_flags::SELECTED != 0)
            .count();
        (a.node_count(), sel)
    })
}

fn act(editor: &Arc<Editor>, cmd: &'static str, action: &str, extra: &[(&str, Value)]) {
    let mut all = vec![("action", json!(action))];
    all.extend(extra.iter().cloned());
    run(editor, cmd, args(&all));
}

/// The level's AI Map Bounds volume, when it has one.
fn bounds_volume(snap: &SceneState) -> Option<&str> {
    snap.of_class(ObjClass::Shape)
        .find(|o| {
            matches!(&o.data, ObjData::Shape(s) if s.usage == gz_scene::bulk::ground::SHAPE_AI_MAP_BOUNDS)
        })
        .map(|o| o.name.as_str())
}

// ---- AI Map ---------------------------------------------------------------------------------------

/// Paint > AI Map: Generate (inside the bounds volume or over the level), the climb limits,
/// Clear...; under More the brush, the links and the drawing.
pub(crate) fn aimap_panel(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    snap: &Arc<gz_store::Snap<SceneState>>,
    panels: &mut BulkPanels,
    chooser: &mut Option<Chooser>,
) {
    let part = snap.bulk.ai_map.as_ref();
    let (nodes, selected) = panels.ai_counts(editor, snap);
    let busy = editor.jobs.running_in("scene").is_some();
    if part.is_none() || nodes == 0 {
        text::status(ui, Tone::Muted, "No AI map yet. The build makes one by itself; Generate makes it now.");
    } else {
        text::status(ui, Tone::Muted, &format!("{} \u{b7} {} selected", plural(nodes, "node"), selected));
    }
    let bounds = bounds_volume(snap).map(str::to_owned);
    ui.horizontal_wrapped(|ui| match &bounds {
        Some(name) => {
            text::label(ui, &format!("Inside {name}"), Font::Body, palette(ui.ctx()).text_secondary, Clip::Middle)
                .on_hover_text("The AI Map Bounds volume the map is generated inside");
        }
        None => {
            text::label(ui, "Over the whole level", Font::Body, palette(ui.ctx()).text_secondary, Clip::End);
            if Button::new("Place Bounds Volume")
                .kind(Kind::Ghost)
                .icon(Icon::Shape)
                .tip("Place an AI Map Bounds volume on the ground in front of the camera: the map is then generated inside it only")
                .show(ui)
                .clicked()
            {
                crate::place::place_bounds_volume(editor);
            }
        }
    });
    ui.add_space(4.0);
    if let Some(a) = part {
        let h = &a.header;
        let mut send = None;
        PropertyGrid::new("tools").show(ui, |g| {
            panels.local_number_row(
                g,
                "can_up",
                "Can Climb",
                "How far a node may lie above its neighbour, metres (0 to 10)",
                h.params.can_up,
                0.01,
                "m",
                &mut send,
            );
            panels.local_number_row(
                g,
                "can_down",
                "Can Drop",
                "How far a node may lie below its neighbour, metres (0 to 10)",
                h.params.can_down,
                0.01,
                "m",
                &mut send,
            );
        });
        if let Some((key, v)) = send {
            act(editor, "scene.ai_map", "set", &[(key, Value::from(v))]);
        }
    }
    ui.horizontal_wrapped(|ui| {
        let gen_tip = if bounds.is_some() {
            "Make the AI map over the ground inside the AI Map Bounds volume"
        } else {
            "Make the AI map over the ground of the whole level"
        };
        if Button::new("Generate")
            .kind(Kind::Primary)
            .icon(Icon::BuildAIMap)
            .enabled(!busy, "Something is being generated already")
            .tip(gen_tip)
            .show(ui)
            .clicked()
        {
            act(editor, "scene.ai_map", "generate", &[]);
        }
        if local_danger(ui, "Clear...", "Remove every node of the AI map", nodes > 0, "The AI map is empty") {
            let ed = editor.clone();
            confirm(
                ui.ctx(),
                editor,
                removal(
                    "clear_ai_map",
                    "Clear AI Map",
                    "Remove every node of the AI map?",
                    vec![plural(nodes, "node")],
                    "Clear",
                ),
                move || act(&ed, "scene.ai_map", "clear", &[]),
            );
        }
    });
    ui.add_space(6.0);
    Section::new("paint.ai_map.more", "More").default_open(false).body(ui, |ui| {
        aimap_more(ui, editor, snap, panels, chooser, nodes, selected);
    });
}

#[allow(clippy::too_many_arguments)]
fn aimap_more(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    snap: &SceneState,
    panels: &mut BulkPanels,
    chooser: &mut Option<Chooser>,
    nodes: usize,
    selected: usize,
) {
    let t = &snap.tools.bulk;
    let part = snap.bulk.ai_map.as_ref();
    let p = palette(ui.ctx());
    text::label(ui, "Brush", Font::Strong, p.text, Clip::End);
    text::status(
        ui,
        Tone::Muted,
        "A drag on the ground adds nodes, the brush's size round the pointer; Ctrl + click selects them.",
    );
    let mut ignore = t.ignore_constraints;
    if Switch::new(&mut ignore, "Ignore Constraints")
        .tip("Link and add nodes even where the climb limits would refuse them")
        .show(ui)
        .changed()
    {
        act(editor, "scene.ai_map", "set", &[("ignore_constraints", Value::Bool(ignore))]);
    }
    let mut auto = t.auto_link;
    if Switch::new(&mut auto, "Auto Link")
        .tip("A node added links to its neighbours at once")
        .show(ui)
        .changed()
    {
        act(editor, "scene.ai_map", "set", &[("auto_link", Value::Bool(auto))]);
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
        act(
            editor,
            "scene.ai_map",
            "set",
            &[("brush_size", Value::from(v.round().clamp(1.0, 100.0) as u32))],
        );
    }
    ui.add_space(6.0);
    text::label(ui, "Links", Font::Strong, p.text, Clip::End);
    PropertyGrid::new("tools").show(ui, |g| {
        g.row(
            "Side Buttons",
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
                    act(editor, "scene.ai_map", "set", &[("mode", json!(m))]);
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
    let _ = ui.allocate_exact_size(Vec2::splat(cell * 3.0), egui::Sense::hover());
    for (col, row, side, icon, tip) in sides {
        let at = egui::Rect::from_min_size(origin + Vec2::new(col * cell, row * cell), Vec2::splat(cell));
        let mut child = crate::local::child_at(ui, ("ai_side", side), at);
        let tip = if selected > 0 { tip } else { why };
        if gz_ui::icon_button(&mut child, icon, tip, false, selected > 0).clicked() {
            act(editor, "scene.ai_map", "link", &[("side", json!(side)), ("camera", Value::Bool(true))]);
        }
    }
    let empty = "The AI map is empty";
    let sel_why = "Select nodes first";
    match local_button_flow(
        ui,
        &[
            LocalButton::new("Select Unlinked", "Select the nodes that have no links").when(nodes > 0, empty),
            LocalButton::new("Invert Selection", "Select the nodes that are not selected")
                .when(nodes > 0, empty),
            LocalButton::new("Smooth Selected", "Even out the selected nodes with their neighbours")
                .when(selected > 0, sel_why),
            LocalButton::new("Lay Selected Level", "Lay the selected nodes level again")
                .when(selected > 0, sel_why),
            LocalButton::new("Remove Links", "Remove the links between the selected nodes")
                .when(selected > 1, "Select two nodes or more"),
            LocalButton::new("Turn Links Round", "Turn the one-way links between the selected nodes round")
                .when(selected > 1, "Select two nodes or more"),
        ],
    ) {
        Some(0) => {
            act(editor, "scene.ai_map", "select", &[("what", json!("links")), ("links", Value::from(0))])
        }
        Some(1) => act(editor, "scene.ai_map", "select", &[("what", json!("invert"))]),
        Some(2) => act(editor, "scene.ai_map", "smooth", &[]),
        Some(3) => act(editor, "scene.ai_map", "reset", &[]),
        Some(4) => act(editor, "scene.ai_map", "link", &[("side", json!("all")), ("mode", json!("remove"))]),
        Some(5) => act(editor, "scene.ai_map", "link", &[("side", json!("all")), ("mode", json!("invert"))]),
        _ => {}
    }
    if local_danger(
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
            removal(
                "delete_ai_nodes",
                "Delete Nodes",
                "Remove the selected nodes of the AI map?",
                vec![plural(selected, "node")],
                "Delete",
            ),
            move || act(&ed, "scene.ai_map", "remove", &[]),
        );
    }
    ui.add_space(6.0);
    let head = Section::new("paint.ai_map.materials", "Ignored Materials")
        .count(&t.ignored.len().to_string())
        .default_open(false)
        .action(Icon::Plus, "Add a game material the AI map is not generated on")
        .show(ui);
    if head.action == Some(0) {
        *chooser = Some(Chooser::new(
            Purpose::ToolCommand { cmd: "scene.ai_map", action: "set", arg: "ignore_materials" },
            "Game Material",
            Some(ListKey::GameMaterials),
            "",
        ));
    }
    if head.open {
        if t.ignored.is_empty() {
            text::status(ui, Tone::Muted, "None: nodes are made on every material.");
        }
        for name in &t.ignored {
            ui.horizontal(|ui| {
                if gz_ui::icon_button(ui, Icon::Close, "Take it out of the list", false, true).clicked() {
                    act(editor, "scene.ai_map", "set", &[("unignore_materials", json!([name.as_str()]))]);
                }
                text::label(ui, name, Font::Body, p.text, Clip::Start);
            });
        }
    }
    if let Some(a) = part {
        let h = &a.header;
        ui.add_space(6.0);
        text::label(ui, "Drawing", Font::Strong, p.text, Clip::End);
        let mut send = None;
        PropertyGrid::new("tools").show(ui, |g| {
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
                "Drawn Within",
                "How far round the camera nodes are drawn, metres (10 to 250)",
                h.vis_radius,
                0.5,
                "m",
                &mut send,
            );
        });
        if let Some((key, v)) = send {
            act(editor, "scene.ai_map", "set", &[(key, Value::from(v))]);
        }
        let mut draw = h.flags & gz_scene::bulk::aimap::flags::HIDE_NODES == 0;
        if Switch::new(&mut draw, "Draw Nodes").tip("The viewport draws the nodes").show(ui).changed() {
            act(editor, "scene.ai_map", "set", &[("draw_nodes", Value::Bool(draw))]);
        }
    }
}

// ---- Foliage ----------------------------------------------------------------------------------------

/// The foliage brush the viewport paints with: the mesh, how far it reaches, how dense.
fn brush(editor: &Editor) -> gz_ui_viewport::Brush {
    gz_ui_viewport::brush(editor)
}

/// Paint > Foliage: the meshes (dropped in from the Place panel or the Content Browser, or
/// added), the selected mesh's density, scale and waving, the brush, Fill; under More the
/// colour-index picture.
pub(crate) fn foliage_panel(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    snap: &SceneState,
    panels: &mut BulkPanels,
    chooser: &mut Option<Chooser>,
    want_dialog: &mut bool,
) {
    let d = snap.bulk.details.as_ref().map(|d| d.model.clone()).unwrap_or_default();
    let slots = d.slots.len();
    let meshes = &d.indices.objects;
    let busy = editor.jobs.running_in("scene").is_some();
    let p = palette(ui.ctx());
    if meshes.is_empty() {
        text::status(ui, Tone::Muted, "No foliage yet: drop a mesh here, pick it, and paint on the ground.");
    } else {
        text::status(
            ui,
            Tone::Muted,
            &format!("{} \u{b7} {} covered", plural(meshes.len(), "mesh"), plural(slots, "slot")),
        );
    }
    let mut b = brush(editor);
    // the meshes: a drop zone, a row each, the brush paints the one picked
    let head = Section::new("paint.foliage.meshes", "Meshes")
        .count(&meshes.len().to_string())
        .action(Icon::Plus, "Add library objects as foliage meshes")
        .show(ui);
    if head.action == Some(0) {
        *chooser =
            Some(Chooser::new(Purpose::DetailObjects, "Foliage Meshes", Some(ListKey::Objects), "").multi());
    }
    let list_top = ui.cursor().min;
    if head.open {
        if meshes.is_empty() {
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), egui::Sense::hover());
            ui.painter().rect_stroke(
                r.shrink(2.0),
                egui::CornerRadius::same(gz_ui::metrics::RADIUS),
                egui::Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "Drop objects here",
                Font::Body.id(),
                p.text_muted,
            );
        }
        let mut remove: Option<String> = None;
        for m in meshes {
            let picked = b.mesh.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(&m.reference));
            ui.horizontal(|ui| {
                let w = ui.available_width() - gz_ui::metrics::ICON_BUTTON - 6.0;
                let (r, resp) =
                    ui.allocate_exact_size(Vec2::new(w, gz_ui::metrics::ROW), egui::Sense::click());
                if picked {
                    ui.painter().rect_filled(r, egui::CornerRadius::same(3), p.selection_focused);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, egui::CornerRadius::same(3), p.hover);
                }
                gz_ui::icons::draw(
                    ui.painter(),
                    Icon::DetailObjects,
                    egui::Rect::from_min_size(r.min + Vec2::new(4.0, 4.0), Vec2::splat(14.0)),
                    p.text_secondary,
                );
                text::paint(
                    ui.painter(),
                    &Font::Body.id(),
                    r.min + Vec2::new(24.0, r.height() * 0.5),
                    egui::Align2::LEFT_CENTER,
                    p.text,
                    crate::outliner::leaf(&m.reference),
                    Some(w - 28.0),
                    Clip::Middle,
                );
                let resp = resp.on_hover_text(format!("{}\nClick to paint with it", m.reference));
                if resp.clicked() {
                    b.mesh = Some(m.reference.clone());
                    gz_ui_viewport::set_brush(editor, b.clone());
                }
                if gz_ui::icon_button(ui, Icon::Close, "Take the mesh out of the foliage", false, true)
                    .clicked()
                {
                    remove = Some(m.reference.to_string());
                }
            });
        }
        if let Some(r) = remove {
            let ed = editor.clone();
            let name = r.clone();
            confirm(
                ui.ctx(),
                editor,
                removal(
                    "remove_foliage_mesh",
                    "Remove Foliage Mesh",
                    "Take this mesh out of the foliage, with everywhere it grows?",
                    vec![r],
                    "Remove",
                ),
                move || act(&ed, "scene.details", "remove", &[("objects", json!([name]))]),
            );
        }
    }
    let list = egui::Rect::from_min_max(
        list_top,
        egui::pos2(ui.max_rect().max.x, ui.cursor().min.y.max(list_top.y + 44.0)),
    );
    let accepts = |p: &gz_ui::drop::Payload| !dropped_objects(p).is_empty();
    if let Some(payload) = gz_ui::drop::Zone::new("Foliage Meshes", list, &accepts).take(ui) {
        add_objects(editor, &dropped_objects(&payload));
    }
    // the picked mesh's own settings
    if let Some(m) = b.mesh.as_ref().and_then(|n| meshes.iter().find(|m| m.reference.eq_ignore_ascii_case(n)))
    {
        ui.add_space(4.0);
        text::label(ui, crate::outliner::leaf(&m.reference), Font::Strong, p.text, Clip::Middle);
        let mut send = None;
        PropertyGrid::new("tools").show(ui, |g| {
            panels.local_number_row(
                g,
                "density",
                "Density",
                "How densely this mesh grows, 0 to 1",
                m.density_factor,
                0.01,
                "",
                &mut send,
            );
            panels.local_number_row(
                g,
                "min_scale",
                "Smallest",
                "The smallest scale an instance gets",
                m.min_scale,
                0.01,
                "",
                &mut send,
            );
            panels.local_number_row(
                g,
                "max_scale",
                "Largest",
                "The largest scale an instance gets",
                m.max_scale,
                0.01,
                "",
                &mut send,
            );
        });
        if let Some((key, v)) = send {
            act(
                editor,
                "scene.details",
                "set",
                &[("object", json!(m.reference.as_str())), (key, Value::from(v))],
            );
        }
        let mut waving = m.flags & gz_fmt_level::DO_NO_WAVING == 0;
        if Switch::new(&mut waving, "Sways in the Wind").show(ui).changed() {
            act(
                editor,
                "scene.details",
                "set",
                &[("object", json!(m.reference.as_str())), ("waving", Value::Bool(waving))],
            );
        }
    }
    // the brush
    ui.add_space(6.0);
    text::label(ui, "Brush", Font::Strong, p.text, Clip::End);
    let mut send = None;
    PropertyGrid::new("tools").show(ui, |g| {
        panels.local_number_row(
            g,
            "brush_radius",
            "Brush Size",
            "How far round the pointer the brush paints, metres",
            b.radius,
            0.1,
            "m",
            &mut send,
        );
        panels.local_number_row(
            g,
            "brush_density",
            "Strength",
            "How dense a stroke paints, 0 to 1",
            b.density,
            0.01,
            "",
            &mut send,
        );
    });
    if let Some((key, v)) = send {
        if key == "brush_radius" {
            b.radius = v.clamp(0.5, 100.0);
        } else {
            b.density = v.clamp(0.0, 1.0);
        }
        gz_ui_viewport::set_brush(editor, b.clone());
    }
    text::status(ui, Tone::Muted, "Drag on the ground to paint the picked mesh; hold Shift to erase it.");
    let mesh = b.mesh.clone();
    ui.horizontal_wrapped(|ui| {
        if Button::new("Fill the Ground")
            .enabled(
                mesh.is_some() && !busy,
                if busy { "Something is being painted already" } else { "Pick a mesh first" },
            )
            .tip("Cover the whole ground with the picked mesh")
            .show(ui)
            .clicked()
            && let Some(m) = &mesh
        {
            act(
                editor,
                "scene.details",
                "fill",
                &[("object", json!(m.as_str())), ("density", Value::from(b.density))],
            );
        }
        if local_danger(ui, "Clear...", "Empty every slot of the foliage", slots > 0, "Nothing grows yet") {
            let ed = editor.clone();
            confirm(
                ui.ctx(),
                editor,
                removal(
                    "clear_details",
                    "Clear Foliage",
                    "Remove the foliage from the whole level?",
                    vec![plural(slots, "covered slot")],
                    "Clear",
                ),
                move || act(&ed, "scene.details", "clear", &[]),
            );
        }
    });
    ui.add_space(6.0);
    Section::new("paint.foliage.more", "More").default_open(false).body(ui, |ui| {
        let base = d.base_texture.clone().unwrap_or_default();
        PropertyGrid::new("tools").show(ui, |g| {
            g.row(
                "Colour Picture",
                "A picture whose colours say what grows where: picking one lays the foliage from it",
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
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Import Again", "Lay the foliage from the colour picture again").when(
                    !busy && !base.is_empty(),
                    if busy { "Something is being painted already" } else { "Pick a colour picture first" },
                ),
                LocalButton::new("Colour Indices...", "Which meshes each colour of the picture grows"),
            ],
        ) {
            Some(0) => act(editor, "scene.details", "import_texture", &[("texture", json!(base.as_str()))]),
            Some(1) => *want_dialog = true,
            _ => {}
        }
        let mut send = None;
        PropertyGrid::new("tools").show(ui, |g| {
            panels.local_number_row(
                g,
                "layer_density",
                "Objects per Square",
                "How densely the whole layer scatters: objects per square metre",
                d.density.unwrap_or(0.3),
                0.01,
                "",
                &mut send,
            );
        });
        if let Some((_, v)) = send {
            act(editor, "scene.details", "set", &[("density", Value::from(v.max(0.0)))]);
        }
        let mut draw = d.flags & tool_flags::OBJECTS_DRAW != 0;
        if Switch::new(&mut draw, "Draw Foliage").tip("The viewport draws the foliage").show(ui).changed() {
            act(editor, "scene.details", "set", &[("draw", Value::Bool(draw))]);
        }
    });
}

/// The library objects a payload carries: Place panel rows of objects, the Content Browser's
/// `.object` files.
fn dropped_objects(p: &gz_ui::drop::Payload) -> Vec<String> {
    gz_ui_viewport::place::Placeable::of_payload(p)
        .into_iter()
        .filter(|x| x.kind == gz_ui_viewport::place::Kind::Object)
        .filter_map(|x| x.reference)
        .collect()
}

/// Adds library objects as foliage meshes (`scene.details action=add`).
pub fn add_objects(editor: &Arc<Editor>, items: &[String]) {
    if !items.is_empty() {
        act(editor, "scene.details", "add", &[("objects", json!(items))]);
    }
}

// ---- Decals -----------------------------------------------------------------------------------------

/// Paint > Decals: the texture and the size a click lays; under More the shader, the alignment,
/// the drawing and the selection.
pub(crate) fn decals_panel(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    snap: &SceneState,
    panels: &mut BulkPanels,
    chooser: &mut Option<Chooser>,
) {
    let model = snap.bulk.wallmarks.as_ref().and_then(|w| w.model.clone());
    if snap.bulk.wallmarks.as_ref().is_some_and(|w| w.model.is_none()) {
        text::status(
            ui,
            Tone::Warning,
            "The decals of this scene could not be read; they are kept as they are.",
        );
        return;
    }
    let w = model.map(|m| (*m).clone()).unwrap_or_default();
    let (count, selected) = (w.count(), w.selected_count());
    let ready = !w.texture.is_empty();
    text::status(
        ui,
        Tone::Muted,
        &if ready {
            format!(
                "A click on the level lays a decal. {} \u{b7} {} selected",
                plural(count, "decal"),
                selected
            )
        } else {
            "Pick a texture, then click on the level to lay a decal.".to_owned()
        },
    );
    PropertyGrid::new("tools").show(ui, |g| {
        g.row("Texture", "What a new decal shows", PropFlags::NONE, 0, |ui| {
            if let PickAction::Browse = gz_ui::grid::pick_field(ui, "wm_texture", &w.texture, false) {
                *chooser = Some(Chooser::new(
                    Purpose::ToolCommand { cmd: "scene.wallmarks", action: "set", arg: "texture" },
                    "Texture",
                    Some(ListKey::Textures),
                    &w.texture,
                ));
            }
        });
    });
    let mut send = None;
    PropertyGrid::new("tools").show(ui, |g| {
        panels.local_number_row(
            g,
            "width",
            "Width",
            "The width of a new decal, metres (0.01 to 10)",
            w.width,
            0.01,
            "m",
            &mut send,
        );
        panels.local_number_row(
            g,
            "height",
            "Height",
            "The height of a new decal, metres (0.01 to 10)",
            w.height,
            0.01,
            "m",
            &mut send,
        );
        panels.local_number_row(
            g,
            "rotate",
            "Turn",
            "How a new decal is turned on the surface",
            w.rotate.to_degrees(),
            0.5,
            "\u{b0}",
            &mut send,
        );
    });
    if let Some((key, v)) = send {
        act(editor, "scene.wallmarks", "set", &[(key, Value::from(v))]);
    }
    if local_danger(
        ui,
        "Delete Selected...",
        "Remove the selected decals",
        selected > 0 && w.draws(),
        "Select decals first",
    ) {
        let ed = editor.clone();
        confirm(
            ui.ctx(),
            editor,
            removal(
                "delete_wallmarks",
                "Delete Decals",
                "Remove the selected decals?",
                vec![plural(selected, "decal")],
                "Delete",
            ),
            move || act(&ed, "scene.wallmarks", "remove", &[]),
        );
    }
    ui.add_space(6.0);
    Section::new("paint.decals.more", "More").default_open(false).body(ui, |ui| {
        PropertyGrid::new("tools").show(ui, |g| {
            g.row("Shader", "The engine shader a new decal is drawn with", PropFlags::NONE, 0, |ui| {
                if let PickAction::Browse = gz_ui::grid::pick_field(ui, "wm_shader", &w.shader, false) {
                    *chooser = Some(Chooser::new(
                        Purpose::ToolCommand { cmd: "scene.wallmarks", action: "set", arg: "shader" },
                        "Shader",
                        Some(ListKey::EngineShaders),
                        &w.shader,
                    ));
                }
            });
            g.row("Alignment", "What a new decal's up is taken from", PropFlags::NONE, 0, |ui| {
                let mut cur = usize::from(w.flags & wm_flags::AXIS_ALIGN != 0);
                if gz_ui::segmented(ui, &mut cur, &["By Camera", "By World Axis"], None) {
                    act(
                        editor,
                        "scene.wallmarks",
                        "set",
                        &[("alignment", json!(if cur == 1 { "world" } else { "camera" }))],
                    );
                }
            });
        });
        let mut draw = w.draws();
        if Switch::new(&mut draw, "Draw Decals")
            .tip("The viewport draws the decals; they can be selected only while it does")
            .show(ui)
            .changed()
        {
            act(editor, "scene.wallmarks", "set", &[("draw", Value::Bool(draw))]);
        }
        let none = if count == 0 { "There are no decals" } else { "The decals are not drawn" };
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Select All", "Select every decal").when(w.draws() && count > 0, none),
                LocalButton::new("Invert Selection", "Select the decals that are not selected")
                    .when(w.draws() && count > 0, none),
            ],
        ) {
            Some(0) => act(editor, "scene.wallmarks", "select", &[("what", json!("all"))]),
            Some(1) => act(editor, "scene.wallmarks", "select", &[("what", json!("invert"))]),
            _ => {}
        }
    });
}

/// What `level.view` answers of the Paint mode's panels.
pub(crate) fn state(editor: &Arc<Editor>, snap: &SceneState, _panels: &BulkPanels) -> Value {
    let b = brush(editor);
    let d = snap.bulk.details.as_ref().map(|d| d.model.clone()).unwrap_or_default();
    let w = snap.bulk.wallmarks.as_ref().and_then(|w| w.model.clone());
    let (nodes, selected) = ai_counts(snap);
    json!({
        "layer": crate::tools::LAYERS.iter().find(|(m, _)| *m == snap.tools.mode).map(|(_, l)| *l),
        "ai_map": {
            "nodes": nodes,
            "selected": selected,
            "bounds": bounds_volume(snap),
        },
        "foliage": {
            "meshes": d.indices.objects.iter().map(|m| m.reference.as_str()).collect::<Vec<_>>(),
            "slots": d.slots.len(),
            "brush": { "mesh": b.mesh.as_deref(), "radius": b.radius, "strength": b.density },
        },
        "decals": {
            "texture": w.as_ref().map(|w| w.texture.clone()),
            "count": w.as_ref().map_or(0, |w| w.count()),
        },
    })
}
