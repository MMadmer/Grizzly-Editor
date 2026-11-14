//! The Colour Indices dialog `Colour Indices###dlg.colour_indices` (L), under Paint > Foliage >
//! More: which foliage meshes each colour of a colour-index picture grows, as Import Again lays
//! them. A block per colour (its swatch, Add..., Take Out, Remove), Add Index, Clear All...,
//! and the `.dti` file that holds the meshes and the colours: Import... and Export....
//!
//! Every change is a `SceneCmd::SetBulk` of the scene's details, one undo step each. The meshes
//! themselves are the Foliage panel's list.

use gz_ui::file_dialog::FileRequest;
use std::sync::Arc;

use egui::{Color32, Ui, Vec2};
use gz_base::SmolStr;
use gz_editor::Editor;
use gz_fmt_level::{ColorIndex, DetailColorIndices, SceneDetails};
use gz_scene::{SceneCmd, SceneService};
use gz_ui::text::Clip;
use gz_ui::{Button, ConfirmSpec, Dialog, DialogSize, Font, Kind, Tone, grid, list, palette};

use crate::choose::{Chooser, Purpose};
use crate::local::{LocalButton, confirm, local_button_flow, plural};

/// The dialog's name.
pub const DIALOG: &str = "dlg.colour_indices";

/// The dialog's view state.
#[derive(Clone, Debug, Default)]
pub struct DetailObjects {
    pub open: bool,
    /// The entry picked in each colour's list.
    color_rows: Vec<Option<usize>>,
}

fn current(svc: &SceneService) -> SceneDetails {
    svc.store.snapshot().bulk.details.as_ref().map(|d| (*d.model).clone()).unwrap_or_default()
}

/// Replaces the scene's details with `model`.
fn commit(svc: &SceneService, editor: &Editor, model: SceneDetails) {
    let snap = svc.store.snapshot();
    let mut bulk = snap.bulk.clone();
    bulk.details = Some(Arc::new(gz_scene::tools::DetailsPart { raw: None, model: Arc::new(model) }));
    svc.store.post(SceneCmd::SetBulk(Box::new(bulk)), editor.ui_origin());
}

/// The meshes a colour index grows, picked in the chooser.
pub fn set_color_objects(editor: &Editor, svc: &SceneService, index: usize, items: &[String]) {
    let mut model = current(svc);
    if let Some(c) = model.indices.colors.get_mut(index) {
        for item in items {
            if !c.objects.iter().any(|o| o == item) && c.objects.len() < 255 {
                c.objects.push(SmolStr::new(item));
            }
        }
    }
    commit(svc, editor, model);
}

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn argb(c: Color32) -> u32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    0xff00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

impl DetailObjects {
    /// Draws the dialog while it is open; `chooser` is where Add... opens its chooser.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, chooser: &mut Option<Chooser>) {
        if !self.open {
            return;
        }
        let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else {
            self.open = false;
            return;
        };
        let model = current(&svc);
        let mut open = true;
        let mut close = false;
        let status = format!(
            "{} \u{b7} {}",
            plural(model.indices.objects.len(), "mesh"),
            if model.indices.colors.len() == 1 {
                "1 colour".to_owned()
            } else {
                format!("{} colours", model.indices.colors.len())
            }
        );
        let mut next = model.clone();
        let mut changed = false;
        Dialog::new("Colour Indices", DIALOG, DialogSize::L).show(
            ui,
            &mut open,
            |ui| changed |= self.colors(ui, editor, &svc, &mut next, chooser),
            |f| {
                f.status(&status, Tone::Muted);
                if f.button("Close", Kind::Primary, true, true, "") {
                    close = true;
                }
            },
        );
        if changed && next != model {
            commit(&svc, editor, next);
        }
        if close || !open {
            self.open = false;
        }
    }

    fn colors(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        m: &mut SceneDetails,
        chooser: &mut Option<Chooser>,
    ) -> bool {
        let p = palette(ui.ctx());
        let mut changed = false;
        gz_ui::text::status(
            ui,
            Tone::Muted,
            "Each colour of the picture grows the meshes listed under it. Paint > Foliage > Import Again lays them.",
        );
        ui.add_space(4.0);
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Add Colour", "A colour of the picture and the meshes that grow on it")
                    .when(m.indices.colors.len() < 255, "A level holds 255 colours at most"),
                LocalButton::new("Import...", "Read the meshes and the colours from a file"),
                LocalButton::new("Export...", "Write the meshes and the colours to a file"),
            ],
        ) {
            Some(0) => {
                m.indices.colors.push(ColorIndex { color: 0xff00_ff00, objects: Vec::new() });
                changed = true;
            }
            Some(1) => import(editor, svc),
            Some(2) => export(editor, m.indices.clone()),
            _ => {}
        }
        if Button::new("Clear All...")
            .kind(Kind::GhostDanger)
            .enabled(!m.indices.colors.is_empty(), "There are no colours")
            .tip("Take every colour out")
            .show(ui)
            .clicked()
        {
            let n = m.indices.colors.len();
            let spec = ConfirmSpec {
                id: "clear_colour_indices".into(),
                title: "Clear Colours".into(),
                headline: "Take every colour out?".into(),
                items: vec![if n == 1 { "1 colour".to_owned() } else { format!("{n} colours") }],
                total: 1,
                consequence: "The meshes stay. Edit > Undo brings the colours back.".into(),
                action: "Clear All".into(),
                destructive: true,
            };
            let store = svc.store.clone();
            let origin = editor.ui_origin();
            confirm(ui.ctx(), editor, spec, move || {
                let snap = store.snapshot();
                let mut bulk = snap.bulk.clone();
                let mut model = bulk.details.as_ref().map(|d| (*d.model).clone()).unwrap_or_default();
                model.indices.colors.clear();
                bulk.details =
                    Some(Arc::new(gz_scene::tools::DetailsPart { raw: None, model: Arc::new(model) }));
                store.post(SceneCmd::SetBulk(Box::new(bulk)), origin);
            });
        }
        if m.indices.colors.is_empty() {
            gz_ui::text::status(ui, Tone::Muted, "No colours yet. Add one, then the meshes that grow on it.");
        }
        self.color_rows.resize(m.indices.colors.len(), None);
        let objects: Vec<String> = m.indices.objects.iter().map(|o| o.reference.to_string()).collect();
        let mut remove = None;
        for (i, c) in m.indices.colors.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.add_space(6.0);
                gz_ui::card::card(
                    ui,
                    &format!("color_{i}"),
                    &format!("Colour {}", i + 1),
                    Tone::Muted,
                    false,
                    |ui| {
                        ui.horizontal(|ui| {
                            let mut col = rgb(c.color);
                            if grid::color(ui, &mut col).changed() {
                                c.color = argb(col);
                                changed = true;
                            }
                        });
                        let picked =
                            self.color_rows.get(i).copied().flatten().filter(|r| *r < c.objects.len());
                        match local_button_flow(
                            ui,
                            &[
                                LocalButton::new("Add...", "Foliage meshes that grow on this colour")
                                    .when(!objects.is_empty(), "Add foliage meshes in Paint > Foliage first"),
                                LocalButton::new("Take Out", "Take the picked mesh off this colour")
                                    .when(picked.is_some(), "Pick a mesh first"),
                                LocalButton::new("Remove", "Take this colour out"),
                            ],
                        ) {
                            Some(0) => {
                                *chooser = Some(
                                    Chooser::new(Purpose::ColorIndexObjects(i), "Foliage Meshes", None, "")
                                        .with_items(objects.clone())
                                        .multi(),
                                );
                            }
                            Some(1) => {
                                if let Some(r) = picked {
                                    c.objects.remove(r);
                                    changed = true;
                                }
                            }
                            Some(2) => remove = Some(i),
                            _ => {}
                        }
                        let h = (c.objects.len().max(1) as f32) * gz_ui::metrics::ROW + 10.0;
                        let names: Vec<String> = c.objects.iter().map(ToString::to_string).collect();
                        list::list(ui, &format!("color_list_{i}"), Vec2::new(0.0, h), |l| {
                            for (r, name) in names.iter().enumerate() {
                                if l.row(&format!("c{i}_{name}"), picked == Some(r), |ui| {
                                    gz_ui::text::label(ui, name, Font::Body, p.text, Clip::Start);
                                })
                                .clicked()
                                    && let Some(slot) = self.color_rows.get_mut(i)
                                {
                                    *slot = Some(r);
                                }
                            }
                        });
                    },
                );
            });
        }
        if let Some(i) = remove {
            m.indices.colors.remove(i);
            self.color_rows.remove(i.min(self.color_rows.len().saturating_sub(1)));
            changed = true;
        }
        changed
    }
}

/// Reads a `.dti` file the author picks into the scene's details.
fn import(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let ed = editor.clone();
    let svc = svc.clone();
    let request = FileRequest::open("Import Colour Indices").filter("Colour indices", &["dti"]);
    let picked =
        gz_ui::file_dialog::pick(&editor.exec, None, crate::tools::asker(editor), request, move |path| {
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| DetailColorIndices::read(&b).map_err(|e| e.to_string()))
            {
                Ok(indices) => {
                    let mut model = current(&svc);
                    model.indices = indices;
                    commit(&svc, &ed, model);
                    ed.log.info(format!("Colour indices imported from {}", path.display()));
                }
                Err(e) => ed
                    .notices
                    .error(format!("The colour indices could not be read from {}: {e}", path.display())),
            }
        });
    if let Err(line) = picked {
        editor.log.finding(&line);
    }
}

/// Writes the meshes and the colours to a `.dti` file the author picks.
fn export(editor: &Arc<Editor>, indices: DetailColorIndices) {
    let ed = editor.clone();
    let request =
        FileRequest::save("Export Colour Indices", "details.dti").filter("Colour indices", &["dti"]);
    // the guard of every writer: nothing under the linked game's folder
    let guard = gz_project::Project::of(editor).map(|(_, p)| p.write_guard(None)).unwrap_or_default();
    let picked =
        gz_ui::file_dialog::pick(&editor.exec, None, crate::tools::asker(editor), request, move |path| {
            let io = ed.clone();
            io.exec.spawn_io(move || {
                let done = guard.check(&path).map_err(|e| e.to_string()).and_then(|()| {
                    gz_editor::fsx::write_atomic(&path, &indices.write()).map_err(|e| {
                        format!("The colour indices could not be written to {}: {e}", path.display())
                    })
                });
                match done {
                    Ok(()) => ed.log.info(format!("Colour indices exported to {}", path.display())),
                    Err(e) => ed.notices.error(e),
                }
            });
        });
    if let Err(line) = picked {
        editor.log.finding(&line);
    }
}
