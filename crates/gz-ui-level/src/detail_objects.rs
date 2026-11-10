//! The Detail Objects dialog `Detail Objects###dlg.detail_objects` (L, `UIDOShuffle.cpp`,
//! `UIDOOneColor.cpp`): Objects (Add... / Remove / Clear All... / Import... / Export..., the
//! list, a picture and the settings grid) and Colour Indices (Add Index / Clear All..., one
//! block per colour with Add / Take Out / Remove).
//!
//! Every change is a `SceneCmd::SetBulk` of the scene's details, one undo step each.

use std::sync::Arc;

use egui::{Color32, Ui, Vec2};
use gz_base::SmolStr;
use gz_editor::Editor;
use gz_fmt_level::{ColorIndex, DO_NO_WAVING, DetailColorIndices, DetailObjectRef, SceneDetails};
use gz_scene::{SceneCmd, SceneService};
use gz_ui::text::Clip;
use gz_ui::{Answer, ConfirmSpec, Dialog, DialogSize, Font, Icon, Kind, PropFlags, PropertyGrid, Tone, grid, list, palette};

use crate::choose::{Chooser, Purpose};
use crate::lists::ListKey;
use crate::local::{LocalButton, local_button_flow, local_thumbnail};
use crate::tools::asker;

/// A level holds this many detail objects at most (`dm_max_objects`).
pub const MAX_OBJECTS: usize = 64;

/// The dialog's view state.
#[derive(Clone, Debug, Default)]
pub struct DetailObjects {
    pub open: bool,
    selected: Option<usize>,
    /// The entry picked in each colour's list.
    color_rows: Vec<Option<usize>>,
    /// Something changed: the details want initializing again.
    modified: bool,
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

/// `AppendDO` for every picked object not there yet, up to [`MAX_OBJECTS`]; a toast says
/// when some did not fit.
pub fn add_objects(ui: &Ui, editor: &Editor, svc: &SceneService, items: &[String]) {
    let mut model = current(svc);
    let mut full = false;
    for item in items {
        if model.indices.objects.iter().any(|o| o.reference.eq_ignore_ascii_case(item)) {
            continue;
        }
        if model.indices.objects.len() >= MAX_OBJECTS {
            full = true;
            break;
        }
        model.indices.objects.push(DetailObjectRef { reference: SmolStr::new(item), ..DetailObjectRef::default() });
    }
    if full {
        gz_ui::state::toast(ui.ctx(), Tone::Warning, "A level holds 64 detail objects at most; the rest were not added");
    }
    commit(svc, editor, model);
}

/// The objects a colour index scatters, picked in the chooser.
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
        let n_obj = model.indices.objects.len();
        let n_col = model.indices.colors.len();
        let status = if self.modified {
            "Changed: reinitialize the details after closing".to_owned()
        } else {
            format!("{n_obj} object{} \u{b7} {n_col} colour ind{}", if n_obj == 1 { "" } else { "s" }, if n_col == 1 { "ex" } else { "ices" })
        };
        let tone = if self.modified { Tone::Warning } else { Tone::Muted };
        let mut next = model.clone();
        let mut changed = false;
        Dialog::new("Detail Objects", "dlg.detail_objects", DialogSize::L).show(
            ui,
            &mut open,
            |ui| {
                ui.columns(2, |cols| {
                    if let [left, right] = cols {
                        changed |= self.objects(left, editor, &svc, &mut next, chooser);
                        changed |= self.colors(right, editor, &mut next, chooser);
                    }
                });
            },
            |f| {
                f.status(&status, tone);
                if f.button("Close", Kind::Primary, true, true, "") {
                    close = true;
                }
            },
        );
        if changed && next != model {
            self.modified = true;
            commit(&svc, editor, next);
        }
        if close || !open {
            self.open = false;
            if self.modified {
                editor.log.info("* The detail objects changed. Reinitialize the details to see it in the level.");
            }
            self.modified = false;
        }
    }

    fn objects(&mut self, ui: &mut Ui, editor: &Arc<Editor>, svc: &Arc<SceneService>, m: &mut SceneDetails, chooser: &mut Option<Chooser>) -> bool {
        let p = palette(ui.ctx());
        let mut changed = false;
        gz_ui::text::label(ui, "Objects", Font::Strong, p.text, Clip::End);
        let picked = self.selected.filter(|i| *i < m.indices.objects.len());
        let pressed = local_button_flow(
            ui,
            &[
                LocalButton::new("Add...", "Pick library objects to grow as details").when(m.indices.objects.len() < MAX_OBJECTS, "A level holds 64 detail objects at most"),
                LocalButton::new("Remove", "Take the object picked in the list out").when(picked.is_some(), "Pick an object in the list first"),
                LocalButton::new("Clear All...", "Take every object and every colour index out").when(!m.indices.objects.is_empty(), "The list is empty"),
                LocalButton::new("Import...", "Read the objects and colour indices from a file"),
                LocalButton::new("Export...", "Write the objects and colour indices to a file"),
            ],
        );
        match pressed {
            Some(0) => *chooser = Some(Chooser::new(Purpose::DetailObjects, "Detail Objects", Some(ListKey::Objects), "").multi()),
            Some(1) => {
                if let Some(i) = picked {
                    let name = m.indices.objects.remove(i).reference;
                    for c in &mut m.indices.colors {
                        c.objects.retain(|o| *o != name);
                    }
                    self.selected = None;
                    changed = true;
                }
            }
            Some(2) => {
                let items: Vec<String> = m.indices.objects.iter().take(10).map(|o| o.reference.to_string()).collect();
                let spec = ConfirmSpec {
                    id: "clear_detail_objects".into(),
                    title: "Clear Detail Objects".into(),
                    headline: "Take every detail object and colour index out?".into(),
                    total: m.indices.objects.len(),
                    items,
                    consequence: "Edit > Undo brings them back.".into(),
                    action: "Clear All".into(),
                    destructive: true,
                };
                let store = svc.store.clone();
                let origin = editor.ui_origin();
                let line = gz_ui::dialog::confirm(ui.ctx(), asker(editor), spec, move |answer| {
                    if answer != Answer::Yes {
                        return;
                    }
                    let snap = store.snapshot();
                    let mut bulk = snap.bulk.clone();
                    let mut model = bulk.details.as_ref().map(|d| (*d.model).clone()).unwrap_or_default();
                    model.indices = DetailColorIndices::default();
                    bulk.details = Some(Arc::new(gz_scene::tools::DetailsPart { raw: None, model: Arc::new(model) }));
                    store.post(SceneCmd::SetBulk(Box::new(bulk)), origin);
                });
                if let Some(line) = line {
                    editor.log.plain(line);
                }
            }
            Some(3) => import(editor, svc),
            Some(4) => export(editor, m.indices.clone()),
            _ => {}
        }
        ui.add_space(4.0);
        let names: Vec<String> = m.indices.objects.iter().map(|o| o.reference.to_string()).collect();
        list::list(ui, "detail_objects", Vec2::new(0.0, 200.0), |l| {
            for (i, name) in names.iter().enumerate() {
                if l.row(&format!("do_{name}"), picked == Some(i), |ui| {
                    gz_ui::text::label(ui, name, Font::Body, p.text, Clip::Start);
                })
                .clicked()
                {
                    self.selected = Some(i);
                }
            }
            if names.is_empty() {
                gz_ui::text::status(l.ui(), Tone::Muted, "No detail objects. Add library objects to grow them over the level.");
            }
        });
        if let Some(i) = picked
            && let Some(o) = m.indices.objects.get_mut(i)
        {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                local_thumbnail(ui, 64.0, Icon::DetailObjects, "This object has no thumbnail");
                gz_ui::text::label(ui, &o.reference, Font::Body, p.text, Clip::Start);
            });
            PropertyGrid::new("detail_objects").show(ui, |g| {
                let mut num = |g: &mut grid::Grid<'_>, label: &str, help: &str, v: &mut f32| {
                    g.row(label, help, PropFlags::NONE, 0, |ui| {
                        let mut x = *v;
                        if grid::number(ui, &mut x, 0.01, "").changed() {
                            *v = x;
                            changed = true;
                        }
                    });
                };
                num(g, "Minimum Scale", "The smallest a detail of this object grows", &mut o.min_scale);
                num(g, "Maximum Scale", "The largest a detail of this object grows", &mut o.max_scale);
                num(g, "Density Factor", "How densely this object grows against the others", &mut o.density_factor);
                g.row("No Waving", "The object stands still in the wind", PropFlags::INLINE, 0, |ui| {
                    let mut on = o.flags & DO_NO_WAVING != 0;
                    if grid::switch(ui, &mut on).changed() {
                        o.flags = if on { o.flags | DO_NO_WAVING } else { o.flags & !DO_NO_WAVING };
                        changed = true;
                    }
                });
            });
        }
        changed
    }

    fn colors(&mut self, ui: &mut Ui, editor: &Arc<Editor>, m: &mut SceneDetails, chooser: &mut Option<Chooser>) -> bool {
        let p = palette(ui.ctx());
        let mut changed = false;
        gz_ui::text::label(ui, "Colour Indices", Font::Strong, p.text, Clip::End);
        match local_button_flow(
            ui,
            &[
                LocalButton::new("Add Index", "A colour of the base texture and the objects that grow on it").when(m.indices.colors.len() < 255, "A level holds 255 colour indices at most"),
                LocalButton::new("Clear All...", "Take every colour index out").when(!m.indices.colors.is_empty(), "There are no colour indices"),
            ],
        ) {
            Some(0) => {
                m.indices.colors.push(ColorIndex { color: 0xff00_ff00, objects: Vec::new() });
                changed = true;
            }
            Some(1) => {
                let n = m.indices.colors.len();
                let spec = ConfirmSpec {
                    id: "clear_color_indices".into(),
                    title: "Clear Colour Indices".into(),
                    headline: "Take every colour index out?".into(),
                    items: vec![format!("{n} colour ind{}", if n == 1 { "ex" } else { "ices" })],
                    total: n,
                    consequence: "The detail objects stay.".into(),
                    action: "Clear All".into(),
                    destructive: true,
                };
                let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else { return false };
                let store = svc.store.clone();
                let origin = editor.ui_origin();
                let line = gz_ui::dialog::confirm(ui.ctx(), asker(editor), spec, move |answer| {
                    if answer != Answer::Yes {
                        return;
                    }
                    let snap = store.snapshot();
                    let mut bulk = snap.bulk.clone();
                    let mut model = bulk.details.as_ref().map(|d| (*d.model).clone()).unwrap_or_default();
                    model.indices.colors.clear();
                    bulk.details = Some(Arc::new(gz_scene::tools::DetailsPart { raw: None, model: Arc::new(model) }));
                    store.post(SceneCmd::SetBulk(Box::new(bulk)), origin);
                });
                if let Some(line) = line {
                    editor.log.plain(line);
                }
            }
            _ => {}
        }
        if m.indices.colors.is_empty() {
            gz_ui::text::status(ui, Tone::Muted, "No colour indices. Add one, then assign the objects that grow on its colour.");
        }
        self.color_rows.resize(m.indices.colors.len(), None);
        let objects: Vec<String> = m.indices.objects.iter().map(|o| o.reference.to_string()).collect();
        let mut remove = None;
        for (i, c) in m.indices.colors.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.add_space(6.0);
                gz_ui::card::card(ui, &format!("color_{i}"), &format!("Index {}", i + 1), Tone::Muted, false, |ui| {
                    ui.horizontal(|ui| {
                        let mut col = rgb(c.color);
                        if grid::color(ui, &mut col).changed() {
                            c.color = argb(col);
                            changed = true;
                        }
                    });
                    let picked = self.color_rows.get(i).copied().flatten().filter(|r| *r < c.objects.len());
                    match local_button_flow(
                        ui,
                        &[
                            LocalButton::new("Add...", "Objects of the list that grow on this colour").when(!objects.is_empty(), "Add detail objects first"),
                            LocalButton::new("Take Out", "Take the picked object off this colour").when(picked.is_some(), "Pick an object first"),
                            LocalButton::new("Remove", "Take this colour index out"),
                        ],
                    ) {
                        Some(0) => *chooser = Some(Chooser::new(Purpose::ColorIndexObjects(i), "Objects", None, "").with_items(objects.clone()).multi()),
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
                });
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
    let spawned = std::thread::Builder::new().name("gz-ui-level.dti".into()).spawn(move || {
        let Some(path) = rfd::FileDialog::new().set_title("Import Detail Objects").add_filter("Detail objects", &["dti"]).pick_file() else { return };
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| DetailColorIndices::read(&b).map_err(|e| e.to_string())) {
            Ok(indices) => {
                let mut model = current(&svc);
                model.indices = indices;
                commit(&svc, &ed, model);
                ed.log.info(format!("* Detail objects imported from {}", path.display()));
            }
            Err(e) => ed.notices.error(format!("The detail objects could not be read from {}: {e}", path.display())),
        }
    });
    if let Err(e) = spawned {
        editor.log.warn(format!("the file dialog did not open: {e}"));
    }
}

/// Writes the objects and colour indices to a `.dti` file the author picks.
fn export(editor: &Arc<Editor>, indices: DetailColorIndices) {
    let ed = editor.clone();
    let spawned = std::thread::Builder::new().name("gz-ui-level.dti".into()).spawn(move || {
        let Some(path) = rfd::FileDialog::new().set_title("Export Detail Objects").add_filter("Detail objects", &["dti"]).set_file_name("details.dti").save_file() else { return };
        match gz_editor::fsx::write_atomic(&path, &indices.write()) {
            Ok(()) => ed.log.info(format!("* Detail objects exported to {}", path.display())),
            Err(e) => ed.notices.error(format!("The detail objects could not be written to {}: {e}", path.display())),
        }
    });
    if let Err(e) = spawned {
        editor.log.warn(format!("the file dialog did not open: {e}"));
    }
}
