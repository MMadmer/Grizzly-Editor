//! The Tools panel (`UILeftBarForm.cpp`, `UI/Tools/*.cpp`): the tiles in one to three
//! columns, each [icon][label][eye]; the active tool filled with the accent; a sticky
//! "<Tool> Tool" header over the panel's own scroll; the tool's form - a tool with a list
//! (Object, Spawn, Particles) draws the search and the list first, its other sections start
//! closed; Snap Objects as a closed section. Detail Objects opens the L dialog
//! `dlg.detail_objects`.
//!
//! What a tool has picked to add (the Object tool's object, the Spawn tool's section, the
//! Particles tool's effect, the Group tool's group) is [`ToolChoice`], an `Editor` service the
//! viewport's Place reads.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Rect, Ui, Vec2, pos2};
use gz_base::{Origin, SmolStr};
use gz_editor::{Args, Editor, Value};
use gz_scene::apply::SelectMode;
use gz_scene::snap::SnapAction;
use gz_scene::tools::{FL_APPEND_RANDOM, ToolOpts};
use gz_scene::{ObjClass, ObjId, SceneCmd, SceneService, SceneState};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Answer, Asker, ConfirmSpec, Dialog, DialogSize, Font, Icon, Kind, SearchField, Tone, metrics, palette,
    state,
};
use parking_lot::Mutex;

use crate::choose::{Chooser, Purpose};
use crate::classes::{self, TOOL_CLASSES};
use crate::lists::{ListKey, Lists};
use crate::local::{LocalButton, local_button_flow, local_thumbnail, local_tool_tile, local_tree_list};

const TILE_H: f32 = 26.0;
/// Two columns in the 240 pt panel even with a scrollbar.
const TILE_MIN_W: f32 = 104.0;
const TILE_GAP: f32 = 4.0;
const HEADER_H: f32 = 28.0;

/// What the tools have picked to add: the viewport's Place reads it.
#[derive(Debug, Default)]
pub struct ToolChoice {
    pub object: Mutex<String>,
    pub spawn: Mutex<String>,
    pub particles: Mutex<String>,
    pub group: Mutex<String>,
    /// A zone entity is placed with a sphere (else a box).
    pub zone_box: Mutex<bool>,
}

impl ToolChoice {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<ToolChoice> {
        editor.services.get_or_init(ToolChoice::default)
    }
}

/// The panel's own state.
pub struct ToolsPanel {
    search: [String; 3],
    percent: f32,
    add_percent: bool,
    snap_row: Option<usize>,
    random: Option<RandomSettings>,
    /// The Detail Objects dialog asked for.
    pub want_detail_objects: bool,
    bulk: crate::tools_bulk::BulkPanels,
}

/// The Random Settings dialog's values (`FillAppendRandomProperties`).
#[derive(Clone, Debug)]
struct RandomSettings {
    min_scale: [f32; 3],
    max_scale: [f32; 3],
    min_rot: [f32; 3],
    max_rot: [f32; 3],
    /// `flAppendRandomScale`, `flAppendRandomScaleProportional`, `flAppendRandomRotation`.
    scale: bool,
    proportional: bool,
    rotate: bool,
    /// `m_AppendRandomObjects`: what Randomize picks from.
    objects: Vec<String>,
}

impl Default for ToolsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolsPanel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            search: Default::default(),
            percent: 50.0,
            add_percent: false,
            snap_row: None,
            random: None,
            want_detail_objects: false,
            bulk: crate::tools_bulk::BulkPanels::default(),
        }
    }

    /// The panel's body.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, chooser: &mut Option<Chooser>) {
        let svc = editor.project_opt().and_then(|s| s.services.get::<SceneService>());
        let Some(svc) = svc.filter(|s| s.store.snapshot().valid) else {
            state::empty_state(
                ui,
                Icon::Settings,
                "No scene",
                "Open or create a scene to use the tools.",
                None,
            );
            return;
        };
        let snap = svc.store.snapshot();
        let origin = editor.ui_origin();
        self.tiles(ui, &svc, &snap, origin);
        ui.add_space(4.0);
        let target = snap.tools.target;
        header(ui, target);
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("gz-ui-level.tool_body").auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.push_id(target.id(), |ui| self.body(ui, editor, &svc, &snap, target, chooser));
                ui.add_space(6.0);
                self.snap_objects(ui, editor, &svc, &snap);
            },
        );
        self.draw_random(ui, &svc, chooser);
    }

    fn tiles(&mut self, ui: &mut Ui, svc: &SceneService, snap: &SceneState, origin: Origin) {
        let avail = ui.available_width();
        let cols = (((avail + TILE_GAP) / (TILE_MIN_W + TILE_GAP)) as usize).clamp(1, 3);
        let tile_w = (avail - TILE_GAP * (cols as f32 - 1.0)) / cols as f32;
        let rows = TOOL_CLASSES.len().div_ceil(cols);
        let h = rows as f32 * (TILE_H + TILE_GAP) - TILE_GAP;
        let (area, _) = ui.allocate_exact_size(Vec2::new(avail, h.max(0.0)), egui::Sense::hover());
        for (n, c) in TOOL_CLASSES.iter().enumerate() {
            let pos = pos2(
                area.min.x + (n % cols) as f32 * (tile_w + TILE_GAP),
                area.min.y + (n / cols) as f32 * (TILE_H + TILE_GAP),
            );
            let rect = Rect::from_min_size(pos, Vec2::new(tile_w, TILE_H));
            let part = snap.tools.part(c.class);
            let active = snap.tools.target == c.class;
            let click = local_tool_tile(
                ui,
                Id::unique(("gz-ui-level.tile", c.class.id())),
                rect,
                c.icon,
                c.label,
                c.title,
                active,
                part.visible,
            );
            if click.eye {
                svc.store.post(
                    SceneCmd::ToolFlags { class: c.class, visible: Some(!part.visible), readonly: None },
                    origin,
                );
            } else if click.tile && !active {
                svc.store.post(SceneCmd::SetTarget(c.class), origin);
            }
        }
    }

    fn body(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        class: ObjClass,
        chooser: &mut Option<Chooser>,
    ) {
        let choice = ToolChoice::of(editor);
        match class {
            ObjClass::SceneObject => self.object_tool(ui, editor, svc, snap, &choice, chooser),
            ObjClass::SpawnPoint => self.spawn_tool(ui, editor, svc, snap, &choice),
            ObjClass::Particles => self.particles_tool(ui, editor, svc, snap, &choice),
            ObjClass::Group => self.group_tool(ui, editor, svc, snap, &choice, chooser),
            ObjClass::DetailObjects => crate::tools_bulk::detail_tool(
                ui,
                editor,
                snap,
                &mut self.bulk,
                chooser,
                &mut self.want_detail_objects,
            ),
            ObjClass::AiMap => crate::tools_bulk::aimap_tool(ui, editor, snap, &mut self.bulk, chooser),
            ObjClass::Wallmark => crate::tools_bulk::wallmark_tool(ui, editor, snap, &mut self.bulk, chooser),
            ObjClass::Sector => crate::tools_objects::sector_tool(ui, editor, snap),
            ObjClass::Portal => crate::tools_objects::portal_tool(ui, editor, snap),
            ObjClass::Way => crate::tools_objects::way_tool(ui, editor, snap),
            ObjClass::Shape => crate::tools_objects::shape_tool(ui, editor, snap),
            ObjClass::Light => crate::tools_objects::light_tool(ui, editor, snap),
            ObjClass::FogVolume => crate::tools_objects::fog_tool(ui, editor, snap),
            _ => {
                gz_ui::text::status(ui, Tone::Muted, "This tool has no settings of its own.");
            }
        }
    }

    /// The list a tool adds from, under its search, `index` naming the tool's own search.
    fn list(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        index: usize,
        key: ListKey,
        current: &str,
    ) -> Option<String> {
        let search = self.search.get_mut(index)?;
        SearchField::new(search).hint("Search").min_em(8.0).id_salt(&format!("tool_search_{index}")).show(ui);
        ui.add_space(4.0);
        let Some(items) = Lists::get(editor, &key) else {
            state::busy_line(ui, &format!("Reading the {}...", key.noun()));
            return None;
        };
        if items.is_empty() {
            gz_ui::text::status(ui, Tone::Muted, &format!("No {} were found.", key.noun()));
            return None;
        }
        // the list takes what the closed sections leave, and at least four rows
        let h = (ui.available_height() - 140.0).clamp(metrics::ROW * 4.0 + 10.0, 480.0);
        let search = self.search.get(index).cloned().unwrap_or_default();
        local_tree_list(ui, &format!("tool_list_{index}"), &items, &search, current, h)
    }

    fn object_tool(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        choice: &ToolChoice,
        chooser: &mut Option<Chooser>,
    ) {
        let p = palette(ui.ctx());
        let current = choice.object.lock().clone();
        let avail = ui.available_width();
        let side = ((ui.available_height() * 0.2).clamp(56.0, 96.0)).min(avail).floor();
        ui.horizontal(|ui| {
            local_thumbnail(
                ui,
                side,
                Icon::SceneObject,
                if current.is_empty() {
                    "Pick an object in the list, or browse the library"
                } else {
                    "This object has no thumbnail"
                },
            );
            ui.vertical(|ui| {
                let leaf = current.rsplit('\\').next().unwrap_or("");
                text::label(
                    ui,
                    if current.is_empty() { "Nothing picked" } else { leaf },
                    Font::Body,
                    p.text,
                    Clip::End,
                );
                if let Some(o) = (!current.is_empty()).then(|| svc.library.cached(&current)).flatten() {
                    text::label(
                        ui,
                        &format!("Faces: {}", o.tris.len()),
                        Font::Body,
                        p.text_secondary,
                        Clip::End,
                    );
                    text::label(
                        ui,
                        &format!("Vertices: {}", o.verts.len()),
                        Font::Body,
                        p.text_secondary,
                        Clip::End,
                    );
                }
                match local_button_flow(
                    ui,
                    &[
                        LocalButton::new("Browse...", "Pick the object from the library"),
                        LocalButton::new("Refresh List", "Read the object library again"),
                    ],
                ) {
                    Some(0) => {
                        *chooser = Some(Chooser::new(
                            Purpose::ToolObject,
                            "Object",
                            Some(ListKey::Objects),
                            &current,
                        ));
                    }
                    Some(1) => crate::lists::Lists::refresh(editor, &ListKey::Objects),
                    _ => {}
                }
            });
        });
        ui.add_space(6.0);
        if let Some(item) = self.list(ui, editor, 0, ListKey::Objects, &current) {
            set_choice(editor, ObjClass::SceneObject, &item);
            let lib = svc.library.clone();
            editor.exec.spawn_io(move || {
                let _ = lib.load(&item);
            });
        }
        ui.add_space(6.0);
        let part = snap.tools.part(ObjClass::SceneObject);
        let random = part.flags & FL_APPEND_RANDOM != 0;
        if gz_ui_app::local::local_section(
            ui,
            "tool_add_objects",
            if random { "Add Objects (Randomized)" } else { "Add Objects" },
            false,
        ) {
            ui.horizontal(|ui| {
                let mut on = random;
                if gz_ui::grid::switch(ui, &mut on).on_hover_text("Each object added gets a random scale and rotation, picked from what Random Settings holds").changed() {
                    let mut t = part.clone();
                    t.flags = if on { t.flags | FL_APPEND_RANDOM } else { t.flags & !FL_APPEND_RANDOM };
                    svc.store.post(SceneCmd::SetWorld { level: Box::new(snap.meta.level.clone()), tools: vec![t], serial: snap.serial, label: "Randomize Added Objects".into() }, editor.ui_origin());
                }
                text::label(ui, "Randomize Added Objects", Font::Body, p.text, Clip::End);
            });
            let pressed = local_button_flow(
                ui,
                &[
                    LocalButton::new("Add Several...", "Pick several objects and add them all at the origin"),
                    LocalButton::new(
                        "Random Settings...",
                        "The scale and rotation ranges Randomize picks from",
                    ),
                ],
            );
            match pressed {
                Some(0) => {
                    *chooser =
                        Some(Chooser::new(Purpose::ToolObject, "Objects", Some(ListKey::Objects), "").multi())
                }
                Some(1) => {
                    if let ToolOpts::Objects(o) = &part.opts {
                        let deg =
                            |v: gz_scene::math::Vec3| [v.x.to_degrees(), v.y.to_degrees(), v.z.to_degrees()];
                        self.random = Some(RandomSettings {
                            min_scale: o.min_scale.to_array(),
                            max_scale: o.max_scale.to_array(),
                            min_rot: deg(o.min_rotation),
                            max_rot: deg(o.max_rotation),
                            scale: part.flags & gz_scene::tools::FL_APPEND_RANDOM_SCALE != 0,
                            proportional: part.flags & gz_scene::tools::FL_APPEND_RANDOM_PROPORTIONAL != 0,
                            rotate: part.flags & gz_scene::tools::FL_APPEND_RANDOM_ROTATION != 0,
                            objects: o.objects.iter().map(ToString::to_string).collect(),
                        });
                    }
                }
                _ => {}
            }
        }
        self.select_by_reference(ui, editor, svc, snap, ObjClass::SceneObject, &current, "object");
        if gz_ui_app::local::local_section(ui, "tool_object_surfaces", "Surfaces", false) {
            match local_button_flow(
                ui,
                &[
                    LocalButton::new(
                        "Reset Selected",
                        "Give the selected objects the surfaces of their reference again",
                    ),
                    LocalButton::new(
                        "Reset All...",
                        "Give every object in the level the surfaces of its reference again",
                    ),
                ],
            ) {
                Some(0) => tool_call(
                    editor,
                    "object_tool",
                    &[("action", Value::String("reset_surfaces".into())), ("selected", Value::Bool(true))],
                ),
                Some(1) => ask_reset_all(editor, ui.ctx(), asker(editor)),
                _ => {}
            }
        }
    }

    fn spawn_tool(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        choice: &ToolChoice,
    ) {
        let current = choice.spawn.lock().clone();
        if let Some(item) = self.list(ui, editor, 1, ListKey::SpawnSections, &current) {
            set_choice(editor, ObjClass::SpawnPoint, &item);
        }
        ui.add_space(6.0);
        gz_ui::PropertyGrid::new("tools").show(ui, |g| {
            g.row(
                "Zone Shape",
                "The shape a restrictor or another zone entity is placed with",
                gz_ui::PropFlags::NONE,
                0,
                |ui| {
                    let mut cur = usize::from(*choice.zone_box.lock());
                    if gz_ui::segmented(ui, &mut cur, &["Sphere", "Box"], None) {
                        *choice.zone_box.lock() = cur == 1;
                    }
                },
            );
        });
        let section = current.rsplit('\\').next().unwrap_or("").to_owned();
        self.select_by_reference(ui, editor, svc, snap, ObjClass::SpawnPoint, &section, "entry");
        let attaching = snap.tools.obj.spawn_attach;
        if gz_ui_app::local::local_section(
            ui,
            "tool_spawn_attached",
            if attaching { "Attached Objects (Attaching)" } else { "Attached Objects" },
            false,
        ) {
            ui.horizontal(|ui| {
                let mut on = attaching;
                if gz_ui::grid::switch(ui, &mut on)
                    .on_hover_text(
                        "A click on an object in the viewport attaches it to the selected spawn point",
                    )
                    .changed()
                {
                    tool_call(
                        editor,
                        "spawn_attach",
                        &[("action", Value::String("on_click".into())), ("on", Value::Bool(on))],
                    );
                    if on {
                        gz_ui_viewport::Viewport::of(editor)
                            .with(|s| s.tool.action = gz_ui_viewport::Action::Place);
                    }
                }
                text::label(ui, "Attach on Click", Font::Body, palette(ui.ctx()).text, Clip::End);
            });
            let holders: Vec<String> = snap
                .of_class(ObjClass::SpawnPoint)
                .filter(|o| snap.is_selected(o.id))
                .filter(|o| matches!(&o.data, gz_scene::ObjData::Spawn(s) if s.attached.is_some()))
                .map(|o| o.name.to_string())
                .collect();
            if local_button_flow(
                ui,
                &[LocalButton::new(
                    "Detach From Selected...",
                    "Take the attached object off the selected spawn points",
                )
                .when(!holders.is_empty(), "No selected spawn point holds an attached object")],
            ) == Some(0)
            {
                let spec = gz_ui::ConfirmSpec {
                    id: "detach_spawn_objects".into(),
                    title: "Detach Objects".into(),
                    headline: "Detach the attached objects from these spawn points?".into(),
                    total: holders.len(),
                    items: holders.into_iter().take(10).collect(),
                    consequence: "Edit > Undo attaches them again.".into(),
                    action: "Detach".into(),
                    destructive: true,
                };
                let ed = editor.clone();
                let line = gz_ui::dialog::confirm(ui.ctx(), asker(editor), spec, move |a| {
                    if a == gz_ui::Answer::Yes {
                        tool_call(&ed, "spawn_attach", &[("action", Value::String("detach".into()))]);
                    }
                });
                if let Some(l) = line {
                    editor.log.plain(l);
                }
            }
        }
    }

    fn particles_tool(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        choice: &ToolChoice,
    ) {
        let current = choice.particles.lock().clone();
        if let Some(item) = self.list(ui, editor, 2, ListKey::Particles, &current) {
            set_choice(editor, ObjClass::Particles, &item);
        }
        ui.add_space(6.0);
        if gz_ui_app::local::local_section(ui, "tool_particles_commands", "Commands", false) {
            self.same_buttons(ui, editor, svc, snap, ObjClass::Particles, &current, "effect");
            // `PlaySelected`: the selected visible systems play or stop; the systems are the
            // viewport's, reached off the UI thread
            let mut asked = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if gz_ui::icon_button(ui, Icon::Play, "Play the selected particle systems", false, true)
                    .clicked()
                {
                    asked = Some(true);
                }
                if gz_ui::icon_button(ui, Icon::Stop, "Stop the selected particle systems", false, true)
                    .clicked()
                {
                    asked = Some(false);
                }
            });
            if let Some(on) = asked {
                let ed = editor.clone();
                editor.exec.spawn(async move {
                    if let Err(e) = gz_ui_viewport::play_particles(&ed, &[], on).await {
                        ed.log.warn(format!(
                            "the particle systems did not {}: {e}",
                            if on { "play" } else { "stop" }
                        ));
                    }
                });
            }
        }
    }

    fn group_tool(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        choice: &ToolChoice,
        chooser: &mut Option<Chooser>,
    ) {
        let origin = editor.ui_origin();
        if gz_ui_app::local::local_section(ui, "tool_group_commands", "Commands", true) {
            let any = !snap.selection.is_empty();
            let groups = snap
                .selected_in_order()
                .iter()
                .any(|id| snap.get(*id).is_some_and(|o| o.class == ObjClass::Group));
            match local_button_flow(
                ui,
                &[
                    LocalButton::new("Group", "Put the selected objects into a new group")
                        .when(any, "Select the objects first"),
                    LocalButton::new("Ungroup", "Take the selected groups apart")
                        .when(groups, "Select a group first"),
                    LocalButton::new(
                        "Make Thumbnail",
                        "Take the library thumbnail of the selected group from the viewport",
                    )
                    .when(groups, "Select a group first"),
                    LocalButton::new("Save As...", "Save the selected group to the library")
                        .when(groups, "Select a group first"),
                ],
            ) {
                Some(0) => svc
                    .store
                    .post(SceneCmd::Group { name: SmolStr::new(snap.gen_name(Some("group"))) }, origin),
                Some(1) => svc.store.post(SceneCmd::Ungroup, origin),
                Some(2) => tool_call(editor, "group_thumbnail", &[]),
                Some(3) => save_group_as(editor, ui.ctx(), svc, snap),
                _ => {}
            }
        }
        if gz_ui_app::local::local_section(ui, "tool_group_pivot", "Pivot", false) {
            let selected_group = snap
                .selected_in_order()
                .into_iter()
                .filter_map(|id| snap.get(id))
                .find(|o| o.class == ObjClass::Group)
                .cloned();
            match local_button_flow(
                ui,
                &[
                    LocalButton::new(
                        "Center to Group",
                        "Move the pivot of the groups to the middle of their objects",
                    ),
                    LocalButton::new(
                        "Align to Object...",
                        "Move the pivot of the selected group to one of its objects",
                    )
                    .when(selected_group.is_some(), "Select a group first"),
                ],
            ) {
                Some(0) => {
                    tool_call(editor, "group_tool", &[("action", Value::String("center_pivot".into()))])
                }
                Some(1) => {
                    if let Some(c) = pivot_chooser(snap) {
                        *chooser = Some(c);
                    }
                }
                _ => {}
            }
        }
        let current = choice.group.lock().clone();
        if gz_ui_app::local::local_section(ui, "tool_group_add", "Group to Add", true) {
            gz_ui::PropertyGrid::new("tools").show(ui, |g| {
                g.row("Current", "The library group the viewport adds", gz_ui::PropFlags::NONE, 0, |ui| {
                    match gz_ui::grid::pick_field(ui, "tool_group", &current, true) {
                        gz_ui::PickAction::Browse => {
                            *chooser = Some(Chooser::new(
                                Purpose::ToolGroup,
                                "Group",
                                Some(ListKey::Groups),
                                &current,
                            ))
                        }
                        gz_ui::PickAction::Clear => set_choice(editor, ObjClass::Group, ""),
                        gz_ui::PickAction::None => {}
                    }
                });
            });
            let groups = snap
                .selected_in_order()
                .iter()
                .any(|id| snap.get(*id).is_some_and(|o| o.class == ObjClass::Group));
            if local_button_flow(
                ui,
                &[LocalButton::new(
                    "Reload References",
                    "Read the library groups of the selected groups again",
                )
                .when(groups, "Select a group first")],
            ) == Some(0)
            {
                tool_call(editor, "group_tool", &[("action", Value::String("reload".into()))]);
            }
        }
        self.select_by_reference(ui, editor, svc, snap, ObjClass::Group, &current, "group");
    }

    /// Select Same, Deselect Same.
    #[allow(clippy::too_many_arguments)]
    fn same_buttons(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        class: ObjClass,
        reference: &str,
        noun: &str,
    ) {
        let picked = !reference.is_empty();
        let why = format!("Pick an {noun} in the list first");
        let tip_select = format!("Select every visible object made from the {noun} picked in the list");
        let tip_deselect = format!("Deselect every object made from the {noun} picked in the list");
        if let Some(i @ (0 | 1)) = local_button_flow(
            ui,
            &[
                LocalButton::new("Select Same", &tip_select).when(picked, &why),
                LocalButton::new("Deselect Same", &tip_deselect).when(picked, &why),
            ],
        ) {
            let ids: Vec<ObjId> = snap
                .of_class(class)
                .filter(|o| {
                    (i == 1 || o.flags.visible)
                        && o.ref_name().is_some_and(|r| r.eq_ignore_ascii_case(reference))
                })
                .map(|o| o.id)
                .collect();
            svc.store.post(
                SceneCmd::Select { ids, mode: if i == 0 { SelectMode::Add } else { SelectMode::Remove } },
                editor.ui_origin(),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn select_by_reference(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        class: ObjClass,
        reference: &str,
        noun: &str,
    ) {
        if !gz_ui_app::local::local_section(
            ui,
            &format!("tool_select_ref_{}", class.id()),
            "Select by Reference",
            false,
        ) {
            return;
        }
        self.same_buttons(ui, editor, svc, snap, class, reference, noun);
        let p = palette(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            let pressed = gz_ui::Button::new("Select % of Same").tip("Of the objects that share a reference with the selected ones, select this share at random").show(ui).clicked();
            ui.allocate_ui(Vec2::new(gz_ui::units::em(5.0), metrics::FRAME), |ui| gz_ui::grid::number(ui, &mut self.percent, 0.5, "%"));
            self.percent = self.percent.clamp(0.0, 100.0);
            gz_ui::grid::switch(ui, &mut self.add_percent).on_hover_text("Keep what is selected now; otherwise those objects are deselected before the pick");
            text::label(ui, "Add to Selection", Font::Body, p.text, Clip::End);
            if pressed {
                let ids = percent_of_same(snap, class, self.percent, now_seed());
                let mode = if self.add_percent { SelectMode::Add } else { SelectMode::Set };
                svc.store.post(SceneCmd::Select { ids, mode }, editor.ui_origin());
            }
        });
    }

    fn snap_objects(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
    ) {
        let list = gz_scene::snap::tool_list(snap);
        let count = list.len();
        let title = if count > 0 { format!("Snap Objects ({count})") } else { "Snap Objects".to_owned() };
        if !gz_ui_app::local::local_section(ui, "tool_snap_objects", &title, false) {
            return;
        }
        let p = palette(ui.ctx());
        let origin = editor.ui_origin();
        for (label, tip, on, use_list) in [
            (
                "Snap to Listed Objects",
                "Object snap uses only the objects in this list, and the viewport highlights them",
                snap.tools.snap_use,
                true,
            ),
            (
                "Pick in Viewport",
                "A click on a scene object adds it to the list; Ctrl+click takes a listed one out",
                snap.tools.snap_pick,
                false,
            ),
        ] {
            ui.horizontal(|ui| {
                let mut v = on;
                if gz_ui::grid::switch(ui, &mut v).on_hover_text(tip).changed() {
                    let cmd = if use_list {
                        SceneCmd::Snap { action: None, use_list: Some(v), pick: None }
                    } else {
                        SceneCmd::Snap { action: None, use_list: None, pick: Some(v) }
                    };
                    svc.store.post(cmd, origin);
                }
                text::label(ui, label, Font::Body, p.text, Clip::End);
            });
        }
        let pressed = local_button_flow(
            ui,
            &[
                LocalButton::new("Set From Selection", "Replace the list with the selected objects"),
                LocalButton::new("Add Selection", "Add the selected objects to the list"),
                LocalButton::new("Remove Selection", "Take the selected objects out of the list"),
                LocalButton::new("Select Listed", "Select the objects in the list")
                    .when(count > 0, "The list is empty"),
                LocalButton::new("Clear...", "Empty the list").when(count > 0, "The list is empty"),
            ],
        );
        let action = |name: &str| SnapAction::parse(name);
        match pressed {
            Some(0) => svc.store.post(
                SceneCmd::Snap { action: action("set_from_selection"), use_list: None, pick: None },
                origin,
            ),
            Some(1) => svc
                .store
                .post(SceneCmd::Snap { action: action("add_selection"), use_list: None, pick: None }, origin),
            Some(2) => svc.store.post(
                SceneCmd::Snap { action: action("remove_selection"), use_list: None, pick: None },
                origin,
            ),
            Some(3) => svc
                .store
                .post(SceneCmd::Snap { action: action("select"), use_list: None, pick: None }, origin),
            Some(4) => {
                let names: Vec<String> =
                    list.iter().filter_map(|id| snap.get(*id)).take(10).map(|o| o.name.to_string()).collect();
                let spec = ConfirmSpec {
                    id: "clear_snap_list".into(),
                    title: "Clear Snap Objects".into(),
                    headline: "Take every object out of the snap list?".into(),
                    items: names,
                    total: count,
                    consequence: "The objects stay in the scene.".into(),
                    action: "Clear".into(),
                    destructive: false,
                };
                let store = svc.store.clone();
                let line = gz_ui::dialog::confirm(ui.ctx(), asker(editor), spec, move |answer| {
                    if answer == Answer::Yes {
                        store.post(
                            SceneCmd::Snap { action: SnapAction::parse("clear"), use_list: None, pick: None },
                            origin,
                        );
                    }
                });
                if let Some(line) = line {
                    editor.log.plain(line);
                }
            }
            _ => {}
        }
        if count == 0 {
            gz_ui::text::status(ui, Tone::Muted, "Empty. Select objects, then Set From Selection.");
            return;
        }
        let h = count as f32 * metrics::ROW + 10.0;
        gz_ui::list::list(ui, "snap_list", Vec2::new(0.0, h), |l| {
            for (i, id) in list.iter().enumerate() {
                let Some(o) = snap.get(*id) else { continue };
                let name = o.name.to_string();
                if l.row(&format!("snap_{}", id.0), self.snap_row == Some(i), |ui| {
                    text::label(ui, &name, Font::Body, p.text, Clip::End);
                })
                .clicked()
                {
                    self.snap_row = Some(i);
                }
            }
        });
    }

    /// The Random Settings sheet's Objects, picked in the chooser.
    pub(crate) fn set_random_objects(&mut self, items: Vec<String>) {
        if let Some(r) = self.random.as_mut() {
            r.objects = items;
        }
    }

    /// The `Random Append` sheet (`FillAppendRandomPropertiesBegin`): the Scale and Rotate
    /// switches, Scale\\Proportional (one factor for the three axes), the ranges, the Objects.
    fn draw_random(&mut self, ui: &mut Ui, svc: &SceneService, chooser: &mut Option<Chooser>) {
        let Some(r) = self.random.as_mut() else { return };
        let mut open = true;
        let mut ok = false;
        let mut cancel = false;
        let mut pick = false;
        Dialog::new("Random Append", "dlg.properties", DialogSize::M).show(
            ui,
            &mut open,
            |ui| {
                gz_ui::PropertyGrid::new("dialog").show(ui, |g| {
                    g.row(
                        "Scale",
                        "Each object added gets a random scale",
                        gz_ui::PropFlags::NONE,
                        0,
                        |ui| {
                            gz_ui::grid::switch(ui, &mut r.scale);
                        },
                    );
                    if r.scale {
                        g.row(
                            "Proportional",
                            "One random factor for the three axes",
                            gz_ui::PropFlags::NONE,
                            1,
                            |ui| {
                                gz_ui::grid::switch(ui, &mut r.proportional);
                            },
                        );
                        if r.proportional {
                            g.row(
                                "Minimum",
                                "The smallest scale an added object gets",
                                gz_ui::PropFlags::NONE,
                                1,
                                |ui| {
                                    ui.add(
                                        egui::DragValue::new(&mut r.min_scale[0])
                                            .speed(0.001)
                                            .range(0.001..=1000.0)
                                            .max_decimals(3),
                                    );
                                },
                            );
                            g.row(
                                "Maximum",
                                "The largest scale an added object gets",
                                gz_ui::PropFlags::NONE,
                                1,
                                |ui| {
                                    ui.add(
                                        egui::DragValue::new(&mut r.max_scale[0])
                                            .speed(0.001)
                                            .range(0.001..=1000.0)
                                            .max_decimals(3),
                                    );
                                },
                            );
                        } else {
                            g.row(
                                "Minimum",
                                "The smallest scale an added object gets, per axis",
                                gz_ui::PropFlags::NONE,
                                1,
                                |ui| gz_ui::grid::vec3_field(ui, "Scale Minimum", &mut r.min_scale, 0.001),
                            );
                            g.row(
                                "Maximum",
                                "The largest scale an added object gets, per axis",
                                gz_ui::PropFlags::NONE,
                                1,
                                |ui| gz_ui::grid::vec3_field(ui, "Scale Maximum", &mut r.max_scale, 0.001),
                            );
                        }
                    }
                    g.row(
                        "Rotate",
                        "Each object added gets a random rotation",
                        gz_ui::PropFlags::NONE,
                        0,
                        |ui| {
                            gz_ui::grid::switch(ui, &mut r.rotate);
                        },
                    );
                    if r.rotate {
                        g.row(
                            "Minimum",
                            "The smallest rotation, per axis, in degrees",
                            gz_ui::PropFlags::NONE,
                            1,
                            |ui| gz_ui::grid::vec3_field(ui, "Rotate Minimum", &mut r.min_rot, 0.5),
                        );
                        g.row(
                            "Maximum",
                            "The largest rotation, per axis, in degrees",
                            gz_ui::PropFlags::NONE,
                            1,
                            |ui| gz_ui::grid::vec3_field(ui, "Rotate Maximum", &mut r.max_rot, 0.5),
                        );
                    }
                    g.row(
                        "Objects",
                        "What Randomize picks the added object from",
                        gz_ui::PropFlags::NONE,
                        0,
                        |ui| {
                            let text = r.objects.join(", ");
                            if gz_ui::grid::pick_field(ui, "random_objects", &text, true)
                                == gz_ui::PickAction::Browse
                            {
                                pick = true;
                            }
                        },
                    );
                });
            },
            |f| {
                if f.button("OK", Kind::Primary, true, true, "") {
                    ok = true;
                }
                if f.button("Cancel", Kind::Secondary, false, true, "") {
                    cancel = true;
                }
            },
        );
        if pick {
            *chooser =
                Some(Chooser::new(Purpose::RandomObjects, "Objects", Some(ListKey::Objects), "").multi());
        }
        if ok {
            let snap = svc.store.snapshot();
            let mut t = snap.tools.part(ObjClass::SceneObject).clone();
            let mut set = |bit: u32, on: bool| t.flags = if on { t.flags | bit } else { t.flags & !bit };
            set(gz_scene::tools::FL_APPEND_RANDOM_SCALE, r.scale);
            set(gz_scene::tools::FL_APPEND_RANDOM_PROPORTIONAL, r.proportional);
            set(gz_scene::tools::FL_APPEND_RANDOM_ROTATION, r.rotate);
            if let ToolOpts::Objects(o) = &mut t.opts {
                o.objects = r.objects.iter().take(32).map(SmolStr::new).collect();
                let rad = |v: [f32; 3]| {
                    gz_scene::math::Vec3::new(v[0].to_radians(), v[1].to_radians(), v[2].to_radians())
                };
                o.min_scale = gz_scene::math::Vec3::from_array(r.min_scale);
                o.max_scale = gz_scene::math::Vec3::from_array(r.max_scale);
                o.min_rotation = rad(r.min_rot);
                o.max_rotation = rad(r.max_rot);
            }
            svc.store.post(
                SceneCmd::SetWorld {
                    level: Box::new(snap.meta.level.clone()),
                    tools: vec![t],
                    serial: snap.serial,
                    label: "Random Settings".into(),
                },
                Origin::Ui,
            );
        }
        if ok || cancel || !open {
            self.random = None;
        }
    }
}

/// The Group tool's Align to Object... chooser: the objects of the first selected group; a pick
/// moves the selected groups' pivot to it (`group_tool action=align_pivot`). `None` without a
/// selected group.
pub(crate) fn pivot_chooser(snap: &gz_scene::SceneState) -> Option<Chooser> {
    let g = snap
        .selected_in_order()
        .into_iter()
        .filter_map(|id| snap.get(id))
        .find(|o| o.class == ObjClass::Group)?;
    let gz_scene::ObjData::Group(gd) = &g.data else { return None };
    let members: Vec<String> =
        gd.members.iter().filter_map(|m| snap.get(*m)).map(|o| o.name.to_string()).collect();
    Some(
        Chooser::new(
            Purpose::ToolCommand { cmd: "group_tool", action: "align_pivot", arg: "object" },
            "Object",
            None,
            "",
        )
        .with_items(members),
    )
}

/// What a tool adds now: kept for the panel, and handed to the viewport's Place (a spawn
/// tool's item is `<group>\<section>`; Place takes the section).
pub fn set_choice(editor: &Editor, class: ObjClass, item: &str) {
    let choice = ToolChoice::of(editor);
    let slot = match class {
        ObjClass::SceneObject => &choice.object,
        ObjClass::SpawnPoint => &choice.spawn,
        ObjClass::Particles => &choice.particles,
        _ => &choice.group,
    };
    *slot.lock() = item.to_owned();
    let place = if class == ObjClass::SpawnPoint { item.rsplit('\\').next().unwrap_or(item) } else { item };
    gz_ui_viewport::set_place_item(editor, class, Some(place).filter(|p| !p.is_empty()));
}

/// The kit's question asker for the window's own questions.
#[must_use]
pub fn asker(editor: &Editor) -> Asker {
    if editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
}

fn header(ui: &mut Ui, class: ObjClass) {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER_H), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(metrics::RADIUS), p.raised);
    gz_ui::icons::draw(
        painter,
        classes::icon(class),
        Rect::from_min_size(
            pos2((rect.min.x + 8.0).round(), (rect.center().y - 8.0).round()),
            Vec2::splat(16.0),
        ),
        p.accent_bright,
    );
    let title = format!("{} Tool", classes::title(class));
    let cut = text::paint(
        painter,
        &Font::Strong.id(),
        pos2(rect.min.x + 32.0, rect.center().y),
        Align2::LEFT_CENTER,
        p.text,
        &title,
        Some((rect.width() - 40.0).max(0.0)),
        Clip::End,
    );
    if cut {
        gz_ui::audit::truncated(&ctx, &title, &title, rect);
    }
}

/// A seed for the % pick: the clock is as random as a pick needs.
fn now_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64 | 1)
}

/// `MultiSelByRefObject`: of the objects that share a reference with a selected one, a share
/// picked at random.
#[must_use]
pub fn percent_of_same(snap: &SceneState, class: ObjClass, percent: f32, seed: u64) -> Vec<ObjId> {
    let refs: rustc_hash::FxHashSet<String> = snap
        .of_class(class)
        .filter(|o| snap.is_selected(o.id))
        .filter_map(|o| o.ref_name().map(str::to_lowercase))
        .collect();
    let mut state = seed.max(1);
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let share = f64::from(percent.clamp(0.0, 100.0)) / 100.0;
    snap.of_class(class)
        .filter(|o| o.flags.visible && o.ref_name().is_some_and(|r| refs.contains(&r.to_lowercase())))
        .filter(|_| next() < share)
        .map(|o| o.id)
        .collect()
}

/// Adds every picked object at the origin (`MultiAppend`).
pub fn add_several(editor: &Arc<Editor>, items: &[String]) {
    for item in items {
        let mut args = Args::new();
        args.insert("ref".into(), Value::String(item.clone()));
        for k in ["x", "y", "z"] {
            args.insert(k.into(), Value::from(0.0));
        }
        editor.post("place_object", args);
    }
}

/// Runs a tool command off the UI thread; its error goes to the notices.
fn tool_call(editor: &Arc<Editor>, command: &'static str, pairs: &[(&str, Value)]) {
    let mut a = gz_editor::Args::new();
    for (k, v) in pairs {
        a.insert((*k).into(), v.clone());
    }
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let r = ed.call(command, a, ed.ui_origin()).await;
        if r.get("ok").and_then(Value::as_bool) != Some(true) {
            ed.notices
                .error(r.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned());
        }
    });
}

/// The Group tool's Save As...: the library's `rawdata\\groups` in the system's dialog.
fn save_group_as(editor: &Arc<Editor>, ctx: &egui::Context, svc: &SceneService, snap: &SceneState) {
    let name = snap
        .selected_in_order()
        .into_iter()
        .filter_map(|id| snap.get(id))
        .find(|o| o.class == ObjClass::Group)
        .and_then(|g| g.ref_name().map(str::to_owned))
        .unwrap_or_else(|| "group".to_owned());
    let dir = svc.root.join("rawdata").join("groups");
    let leaf = name.rsplit('\\').next().unwrap_or("group").to_owned();
    let ed = editor.clone();
    let request = gz_ui::file_dialog::FileRequest::save("Save Group As", &format!("{leaf}.group"))
        .filter("Group", &["group"])
        .in_folder(dir)
        .creating_folder();
    let picked = gz_ui::file_dialog::pick(Some(ctx), asker(editor), request, move |f| {
        tool_call(
            &ed,
            "group_tool",
            &[("action", Value::String("save_as".into())), ("file", Value::String(f.display().to_string()))],
        );
    });
    if let Err(line) = picked {
        editor.log.plain(line);
    }
}

/// Reset All Surfaces' question, naming how many objects it touches; `who` answers it.
pub(crate) fn ask_reset_all(editor: &Arc<Editor>, ctx: &egui::Context, who: gz_ui::Asker) {
    let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else { return };
    let snap = svc.store.snapshot();
    let count = snap.of_class(ObjClass::SceneObject).filter(|o| o.flags.visible).count();
    let spec = gz_ui::ConfirmSpec {
        id: "reset_surfaces".into(),
        title: "Reset Surfaces".into(),
        headline: "Reset the surfaces of every object in the level?".into(),
        items: vec![format!("{count} visible scene object{}", if count == 1 { "" } else { "s" })],
        total: 1,
        consequence: "Surface changes made on these objects in this level are lost.".into(),
        action: "Reset All".into(),
        destructive: true,
    };
    let ed = editor.clone();
    let line = gz_ui::dialog::confirm(ctx, who, spec, move |a| {
        if a == gz_ui::Answer::Yes {
            tool_call(
                &ed,
                "object_tool",
                &[("action", Value::String("reset_surfaces".into())), ("selected", Value::Bool(false))],
            );
        }
    });
    if let Some(l) = line {
        editor.log.plain(l);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_share_picks_nothing() {
        let s = SceneState::default();
        assert!(percent_of_same(&s, ObjClass::SceneObject, 0.0, 7).is_empty());
    }
}
