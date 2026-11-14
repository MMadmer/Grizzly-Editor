//! The Level tab's left panel: what the mode in the viewport works with.
//!
//! - **Select** and **Place**: the Place panel ([`crate::place`]) - everything a level can be
//!   given, dragged into the viewport or clicked to arm Place mode.
//! - **Paint**: the layer picked on top - AI Map, Foliage, Decals - and its panel
//!   (`tools_bulk`): the common case on the surface, the rest under More.
//! - **Zones**: the visibility cells and their portals (`tools_objects`), with a first line
//!   saying the build makes them by itself.
//!
//! The tab names the mode's panel ([`tab_label`]). Nothing here changes the scene itself: a
//! panel posts the scene's commands (`scene.mode`, `scene.ai_map`, `scene.details`,
//! `scene.wallmarks`, `scene.zones`) or the store's.

use std::sync::Arc;

use egui::Ui;
use gz_editor::{Editor, Value};
use gz_scene::model::SceneMode;
use gz_scene::{ObjClass, SceneCmd, SceneService, SceneState};
use gz_ui::{Asker, Icon, state};

use crate::choose::{Chooser, Purpose};
use crate::local::{self, args, run};
use crate::place::PlacePanel;
use crate::tools_bulk::BulkPanels;

/// The panel's own state.
pub struct ToolsPanel {
    pub place: PlacePanel,
    bulk: BulkPanels,
    /// The Colour Indices dialog asked for.
    pub want_detail_objects: bool,
}

/// The three layers Paint mode paints, in the panel's order: the mode, its label.
pub const LAYERS: [(SceneMode, &str); 3] =
    [(SceneMode::AiMap, "AI Map"), (SceneMode::Details, "Foliage"), (SceneMode::Wallmarks, "Decals")];

/// What the left panel's tab reads in a mode.
#[must_use]
pub fn tab_label_of(mode: SceneMode) -> &'static str {
    match mode {
        SceneMode::Select | SceneMode::Place => "Place",
        SceneMode::AiMap | SceneMode::Details | SceneMode::Wallmarks => "Paint",
        SceneMode::Zones => "Zones",
    }
}

/// What the left panel's tab reads now.
#[must_use]
pub fn tab_label(editor: &Editor) -> &'static str {
    let mode = editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .map_or(SceneMode::Select, |s| s.store.snapshot().tools.mode);
    tab_label_of(mode)
}

impl ToolsPanel {
    #[must_use]
    pub fn new(editor: &Editor) -> Self {
        Self { place: PlacePanel::new(editor), bulk: BulkPanels::default(), want_detail_objects: false }
    }

    /// The panel's body.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, chooser: &mut Option<Chooser>) {
        let svc = editor.project_opt().and_then(|s| s.services.get::<SceneService>());
        let Some(svc) = svc.filter(|s| s.store.snapshot().valid) else {
            state::empty_state(
                ui,
                Icon::Place,
                "No scene",
                "Open or create a scene to place things in it.",
                None,
            );
            return;
        };
        let snap = svc.store.snapshot();
        match snap.tools.mode {
            SceneMode::Select | SceneMode::Place => self.place.draw(ui, editor, &svc),
            SceneMode::Zones => {
                egui::ScrollArea::vertical()
                    .id_salt("gz-ui-level.zones")
                    .auto_shrink([false, false])
                    .show(ui, |ui| crate::tools_objects::zones_panel(ui, editor, &snap));
            }
            mode @ (SceneMode::AiMap | SceneMode::Details | SceneMode::Wallmarks) => {
                let mut cur = LAYERS.iter().position(|(m, _)| *m == mode).unwrap_or(0);
                let labels: Vec<&str> = LAYERS.iter().map(|(_, l)| *l).collect();
                let icons = [Icon::AIMap, Icon::DetailObjects, Icon::Wallmark];
                if gz_ui::segmented(ui, &mut cur, &labels, Some(&icons))
                    && let Some((m, _)) = LAYERS.get(cur)
                {
                    svc.store.post(SceneCmd::SetMode(*m), editor.ui_origin());
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt(("gz-ui-level.paint", mode.word()))
                    .auto_shrink([false, false])
                    .show(ui, |ui| match mode {
                        SceneMode::AiMap => {
                            crate::tools_bulk::aimap_panel(ui, editor, &snap, &mut self.bulk, chooser)
                        }
                        SceneMode::Details => crate::tools_bulk::foliage_panel(
                            ui,
                            editor,
                            &snap,
                            &mut self.bulk,
                            chooser,
                            &mut self.want_detail_objects,
                        ),
                        _ => crate::tools_bulk::decals_panel(ui, editor, &snap, &mut self.bulk, chooser),
                    });
            }
        }
    }

    /// What `level.view` answers of the painting and zone panels.
    #[must_use]
    pub fn paint_state(&self, editor: &Arc<Editor>, snap: &SceneState) -> Value {
        crate::tools_bulk::state(editor, snap, &self.bulk)
    }
}

/// The kit's question asker for the window's own questions.
#[must_use]
pub fn asker(editor: &Editor) -> Asker {
    if editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
}

/// The Pivot to an Object... chooser: the objects of the first selected group; a pick moves the
/// selected groups' pivot to it (`scene.group action=pivot_to`). `None` without a selected group.
pub(crate) fn pivot_chooser(snap: &SceneState) -> Option<Chooser> {
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
            Purpose::ToolCommand { cmd: "scene.group", action: "pivot_to", arg: "object" },
            "Object",
            None,
            "",
        )
        .with_items(members),
    )
}

/// Save Group As...: the selected group as a `.group` asset of the project, where the system's
/// dialog says (`scene.group action=save`).
pub(crate) fn save_group_as(
    editor: &Arc<Editor>,
    ctx: &egui::Context,
    svc: &SceneService,
    snap: &SceneState,
) {
    let name = snap
        .selected_in_order()
        .into_iter()
        .filter_map(|id| snap.get(id))
        .find(|o| o.class == ObjClass::Group)
        .map_or_else(|| "group".to_owned(), |g| crate::outliner::leaf(&g.name).to_owned());
    let dir = svc.root.clone();
    let ed = editor.clone();
    let request = gz_ui::file_dialog::FileRequest::save("Save Group As", &format!("{name}.group"))
        .filter("Group", &["group"])
        .in_folder(dir)
        .creating_folder();
    let picked = gz_ui::file_dialog::pick(Some(ctx), asker(editor), request, move |f| {
        run(
            &ed,
            "scene.group",
            args(&[
                ("action", Value::String("save".into())),
                ("path", Value::String(f.display().to_string())),
            ]),
        );
    });
    if let Err(line) = picked {
        editor.log.plain(line);
    }
}

/// Reset All Surfaces' question, naming how many objects it touches; `who` answers it.
pub(crate) fn ask_reset_all(editor: &Arc<Editor>, ctx: &egui::Context, who: Asker) {
    let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else { return };
    let snap = svc.store.snapshot();
    let names: Vec<Value> = snap
        .of_class(ObjClass::SceneObject)
        .filter(|o| o.flags.visible)
        .map(|o| Value::String(o.name.to_string()))
        .collect();
    let count = names.len();
    let spec = gz_ui::ConfirmSpec {
        id: "reset_surfaces".into(),
        title: "Reset Surfaces".into(),
        headline: "Reset the surfaces of every object in the level?".into(),
        items: vec![local::plural(count, "visible object")],
        total: 1,
        consequence: "Surface changes made on these objects in this level are lost.".into(),
        action: "Reset All".into(),
        destructive: true,
    };
    let ed = editor.clone();
    let line = gz_ui::dialog::confirm(ctx, who, spec, move |a| {
        if a == gz_ui::Answer::Yes {
            let mut set = serde_json::Map::new();
            set.insert("surfaces.reset".into(), Value::String("Reset".into()));
            run(&ed, "scene.set", args(&[("objects", Value::Array(names)), ("set", Value::Object(set))]));
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
    fn the_tab_names_the_modes_panel() {
        assert_eq!(tab_label_of(SceneMode::Select), "Place");
        assert_eq!(tab_label_of(SceneMode::Place), "Place");
        assert_eq!(tab_label_of(SceneMode::Details), "Paint");
        assert_eq!(tab_label_of(SceneMode::Zones), "Zones");
        assert_eq!(LAYERS.len(), 3);
    }
}
