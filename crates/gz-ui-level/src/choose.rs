//! The chooser `Select <what>###dlg.choose` (L): a search over a list, folders opened where a
//! match sits, the item picked (`UIChooseForm`). What comes from a list is picked, never typed:
//! an empty list says what it lacks.
//!
//! Who asked decides what a pick does ([`Purpose`]); the dialog only answers what was picked.

use std::sync::Arc;

use egui::Ui;
use gz_editor::Editor;
use gz_ui::text::Clip;
use gz_ui::{Dialog, DialogSize, Kind, SearchField, Tone, state};

use crate::lists::{ListKey, Lists};
use crate::local::local_tree_list;

/// What the pick is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A choose row of Details (target 0) or Level Settings (target 1).
    Property { target: usize, key: String },
    /// The Object tool's current object.
    ToolObject,
    /// The Group tool's group to add.
    ToolGroup,
    /// Objects the Detail Objects dialog adds (several).
    DetailObjects,
    /// The objects a colour index scatters (several), by the index's position.
    ColorIndexObjects(usize),
    /// A tool command takes the pick as an argument (`aimap_tool action=ignore_material name=`).
    ToolCommand { cmd: &'static str, action: &'static str, arg: &'static str },
    /// The Random Settings sheet's Objects (several).
    RandomObjects,
}

/// One open chooser.
#[derive(Clone, Debug)]
pub struct Chooser {
    pub purpose: Purpose,
    pub title: String,
    pub list: Option<ListKey>,
    /// Items given by the caller instead of a list (a detail object's colour picks from the
    /// dialog's own objects).
    pub items: Option<Arc<Vec<String>>>,
    pub multi: bool,
    pub search: String,
    pub picked: Vec<String>,
    /// The value it started from.
    pub current: String,
}

impl Chooser {
    #[must_use]
    pub fn new(purpose: Purpose, what: &str, list: Option<ListKey>, current: &str) -> Self {
        Self {
            purpose,
            title: format!("Select {what}"),
            list,
            items: None,
            multi: false,
            search: String::new(),
            picked: if current.is_empty() { Vec::new() } else { vec![current.to_owned()] },
            current: current.to_owned(),
        }
    }

    #[must_use]
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self.picked.clear();
        self
    }

    /// Takes several items, starting from those of the comma list it was opened on (a model's
    /// fixed bones).
    #[must_use]
    pub fn multi_from_current(mut self) -> Self {
        self.multi = true;
        self.picked =
            self.current.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
        self
    }

    #[must_use]
    pub fn with_items(mut self, items: Vec<String>) -> Self {
        self.items = Some(Arc::new(items));
        self
    }
}

/// What the dialog did this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Cancelled,
    /// Picked: the items (one unless the chooser takes several; empty for None).
    Picked(Vec<String>),
}

/// Draws the chooser. The caller keeps it while [`Outcome::Open`] comes back.
pub fn draw(ui: &mut Ui, editor: &Arc<Editor>, c: &mut Chooser) -> Outcome {
    let mut open = true;
    let mut outcome = Outcome::Open;
    let items = match (&c.items, &c.list) {
        (Some(items), _) => Some(items.clone()),
        (None, Some(key)) => Lists::get(editor, key),
        // a list its owner did not give: nothing to pick
        (None, None) => Some(Arc::new(Vec::new())),
    };
    let mut double = None;
    let any = !c.picked.is_empty();
    let status = if c.current.is_empty() { String::new() } else { format!("Now: {}", c.current) };
    let multi = c.multi;
    let (mut select, mut none, mut cancel) = (false, false, false);
    Dialog::new(&c.title, "dlg.choose", DialogSize::L).show(
        ui,
        &mut open,
        |ui| {
            SearchField::new(&mut c.search).hint("Search").id_salt("choose_search").show(ui);
            ui.add_space(4.0);
            match &items {
                None => state::busy_line(
                    ui,
                    &format!("Reading the {}...", c.list.as_ref().map_or("items", ListKey::noun)),
                ),
                Some(list) if list.is_empty() => {
                    state::empty_state(
                        ui,
                        gz_ui::Icon::Search,
                        "Nothing to pick",
                        &c.list
                            .as_ref()
                            .map_or_else(|| "No items were found.".to_owned(), ListKey::empty_text),
                        None,
                    );
                }
                Some(list) => {
                    let current = c.picked.last().cloned().unwrap_or_default();
                    let h = (ui.available_height() - 40.0).clamp(160.0, 380.0);
                    if let Some(item) = local_tree_list(ui, "choose", list, &c.search, &current, h) {
                        let again =
                            ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
                        if c.multi {
                            if let Some(i) = c.picked.iter().position(|p| *p == item) {
                                c.picked.remove(i);
                            } else {
                                c.picked.push(item.clone());
                            }
                        } else {
                            c.picked = vec![item.clone()];
                        }
                        if again && !c.multi {
                            double = Some(item);
                        }
                    }
                    if c.multi && !c.picked.is_empty() {
                        ui.add_space(4.0);
                        gz_ui::text::label(
                            ui,
                            &format!("{} picked: {}", c.picked.len(), c.picked.join(", ")),
                            gz_ui::Font::Small,
                            gz_ui::palette(ui.ctx()).text_secondary,
                            Clip::End,
                        );
                    }
                }
            }
        },
        |f| {
            f.status(&status, Tone::Muted);
            select = f.button("Select", Kind::Primary, true, any, "Pick an item first.");
            none = !multi && f.button("None", Kind::Secondary, false, true, "");
            cancel = f.button("Cancel", Kind::Secondary, false, true, "");
        },
    );
    if select {
        outcome = Outcome::Picked(c.picked.clone());
    } else if none {
        outcome = Outcome::Picked(Vec::new());
    } else if cancel {
        outcome = Outcome::Cancelled;
    }
    if let Some(item) = double {
        return Outcome::Picked(vec![item]);
    }
    if !open && outcome == Outcome::Open {
        return Outcome::Cancelled;
    }
    outcome
}
