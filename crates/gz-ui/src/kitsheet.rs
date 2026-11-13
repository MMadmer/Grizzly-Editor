//! The kit sheet (`-kitsheet`): every widget of the kit in every state it has, on one page, and
//! the tokens beside them. It is how a surface's author - or a critic - sees what exists before
//! writing a widget of their own.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Ui, Vec2, pos2};

use crate::button::{Button, Kind, icon_button, segmented};
use crate::check::{Check, Switch};
use crate::font::Font;
use crate::grid::{self, number, text_field};
use crate::icons::Icon;
use crate::menu::{MenuItem, menu_item, popup_body, popup_frame};
use crate::pick::{Combo, PickItem, PickList};
use crate::rows::{Col, ItemRows, Table, cell_text};
use crate::search::SearchField;
use crate::section::Section;
use crate::state::{badge, banner, banner_list, empty_state, spinner, status_dot};
use crate::text::{self, Clip};
use crate::theme::{Tone, metrics, palette};
use crate::toolbar::{Toolbar, ToolbarStyle};
use crate::units::em;

/// What the sheet's live widgets hold between frames.
#[derive(Clone, Debug)]
struct Sheet {
    check: [bool; 3],
    switch: [bool; 2],
    seg: usize,
    combo: usize,
    pick: String,
    text: String,
    search: String,
    number: f32,
    rows: Vec<(String, i32)>,
    selected: Option<usize>,
    panel: bool,
}

impl Default for Sheet {
    fn default() -> Self {
        Self {
            check: [false, true, false],
            switch: [false, true],
            seg: 1,
            combo: 0,
            pick: "spawn_npc".to_owned(),
            text: "Night 7".to_owned(),
            search: String::new(),
            number: 6.0,
            rows: vec![("Bread".to_owned(), 2), ("Medkit".to_owned(), 1), ("Ammo 9x18".to_owned(), 30)],
            selected: Some(1),
            panel: true,
        }
    }
}

fn heading(ui: &mut Ui, title: &str) {
    ui.add_space(metrics::S16);
    let p = palette(ui.ctx());
    text::label(ui, title, Font::Title, p.text, Clip::End);
    ui.add_space(metrics::S4);
}

fn caption(ui: &mut Ui, text_: &str) {
    let p = palette(ui.ctx());
    ui.add_sized(
        Vec2::new(em(9.0), metrics::FRAME),
        egui::Label::new(egui::RichText::new(text_).font(Font::Small.id()).color(p.text_muted)),
    );
}

/// A sentence about the sample above it.
fn note(ui: &mut Ui, text_: &str) {
    let p = palette(ui.ctx());
    ui.add_space(metrics::S4);
    ui.label(egui::RichText::new(text_).font(Font::Small.id()).color(p.text_muted));
}

fn swatch(ui: &mut Ui, color: Color32, name: &str) {
    let p = palette(ui.ctx());
    ui.vertical(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(56.0, 28.0), Sense::hover());
        ui.painter().rect(
            r,
            CornerRadius::same(4),
            color,
            Stroke::new(1.0, p.border_subtle),
            egui::StrokeKind::Inside,
        );
        ui.label(egui::RichText::new(name).font(Font::Small.id()).color(p.text_muted));
    });
}

fn pick_items() -> Vec<PickItem> {
    let p = crate::theme::Palette::new(crate::theme::Accent::default());
    vec![
        PickItem::new("wait", "Wait")
            .icon(Icon::Wait)
            .tint(p.family(1))
            .detail("flow.wait - waits a time or a condition"),
        PickItem::new("spawn_npc", "Spawn NPC")
            .icon(Icon::SpawnNpc)
            .tint(p.family(4))
            .detail("spawn.npc - characters of the module"),
        PickItem::new("give_item", "Give Item").icon(Icon::Item).tint(p.family(6)).detail("item.give"),
        PickItem::new("app_message", "App Message")
            .icon(Icon::PdaMsg)
            .tint(p.family(8))
            .detail("pda.message"),
        PickItem::new("end", "End").icon(Icon::End).tint(p.family(0)).detail("flow.end").enabled(false),
    ]
}

/// The sheet, in a column of the width `ui` gives (scroll it yourself).
pub fn sheet(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = egui::Id::unique("gz-ui.kitsheet");
    let mut s = ctx.data(|d| d.get_temp::<Sheet>(id)).unwrap_or_default();
    ui.set_max_width(metrics::PAGE_W.max(ui.available_width().min(1100.0)));
    ui.spacing_mut().item_spacing = Vec2::new(metrics::S8, metrics::S6);

    heading(ui, "Buttons");
    ui.horizontal(|ui| {
        caption(ui, "Kinds");
        let _ = Button::new("Build").primary().show(ui);
        let _ = Button::new("Secondary").show(ui);
        let _ = Button::new("Ghost").kind(Kind::Ghost).show(ui);
        let _ = Button::new("Panel On").kind(Kind::Ghost).icon(Icon::SidebarLeft).checked(true).show(ui);
        let _ = Button::new("Delete...").kind(Kind::GhostDanger).show(ui);
        let _ = Button::new("Delete").kind(Kind::Danger).show(ui);
    });
    ui.horizontal(|ui| {
        caption(ui, "Off, icons");
        let _ = Button::new("Off").enabled(false, "Why it is off.").show(ui);
        let _ = Button::new("Off Primary").primary().enabled(false, "Why it is off.").show(ui);
        let _ = icon_button(ui, Icon::Save, "Rest", false, true);
        let _ = icon_button(ui, Icon::Minimap, "On", true, true);
        let _ = icon_button(ui, Icon::Delete, "Off", false, false);
    });
    ui.horizontal(|ui| {
        caption(ui, "Segmented");
        segmented(ui, &mut s.seg, &["Auto", "Manual", "Never"], None);
    });

    heading(ui, "Toolbar");
    ui.horizontal(|ui| {
        let mut tb = Toolbar::begin(ui, "kitsheet", ToolbarStyle::Row);
        tb.group(10);
        let _ = tb.action("save", Icon::Save, "Save", "Ctrl+S", true, false, Kind::Ghost);
        let _ = tb.action("undo", Icon::Undo, "Undo", "", false, false, Kind::Ghost);
        tb.group(5);
        if tb.action(
            "panel",
            Icon::SidebarLeft,
            "Graph Panel",
            "A toggle: on shows the line",
            true,
            s.panel,
            Kind::Ghost,
        ) {
            s.panel = !s.panel;
        }
        tb.status("3 warnings", Tone::Warning);
        tb.spring();
        tb.group(100);
        let _ =
            tb.action("build", Icon::BuildMod, "Build Mod", "The one Primary", true, false, Kind::Primary);
        tb.end();
    });

    heading(ui, "Check and Switch");
    ui.horizontal(|ui| {
        caption(ui, "Check");
        let [a, b, c] = &mut s.check;
        let _ = Check::new(a, "Off").show(ui);
        let _ = Check::new(b, "On").show(ui);
        let _ = Check::new(c, "Mixed").mixed(true).show(ui);
        let mut off = true;
        let _ = Check::new(&mut off, "Disabled").enabled(false, "Why it is off.").show(ui);
    });
    ui.horizontal(|ui| {
        caption(ui, "Switch");
        let [a, b] = &mut s.switch;
        let _ = Switch::new(a, "Off").show(ui);
        let _ = Switch::new(b, "On").show(ui);
        let mut off = false;
        let _ = Switch::new(&mut off, "Disabled").enabled(false, "Why it is off.").show(ui);
    });

    heading(ui, "Fields, Combo and PickList");
    ui.horizontal(|ui| {
        caption(ui, "Fields");
        ui.scope(|ui| {
            ui.set_width(em(10.0));
            text_field(ui, &mut s.text);
        });
        ui.scope(|ui| {
            ui.set_width(em(6.0));
            let _ = number(ui, &mut s.number, 0.1, "s");
        });
        ui.scope(|ui| {
            ui.set_width(em(12.0));
            SearchField::new(&mut s.search).hint("Search").id_salt("kitsheet").show(ui);
        });
    });
    ui.horizontal(|ui| {
        caption(ui, "Combo");
        ui.scope(|ui| {
            ui.set_width(em(10.0));
            grid::combo(ui, "kitsheet.combo", &mut s.combo, &["storyline", "side", "restrictor"]);
        });
        let items = pick_items();
        let shown = items.iter().find(|i| i.id == s.pick).map_or("", |i| i.label.as_str()).to_owned();
        let list = PickList::new("kitsheet.pick", &items).current(&s.pick).remember("kitsheet");
        if let Some(i) = Combo::new("kitsheet.pick", &shown).icon(Icon::Flow).width(em(10.0)).show(ui, list)
            && let Some(it) = items.get(i)
        {
            s.pick.clone_from(&it.id);
        }
        let _ =
            Combo::new("kitsheet.mixed", "x").width(em(8.0)).mixed(true).show(ui, PickList::new("m", &[]));
        let _ = Combo::new("kitsheet.error", "unknown_kind")
            .width(em(8.0))
            .error(true)
            .show(ui, PickList::new("e", &[]));
        let _ = Combo::new("kitsheet.off", "Off")
            .width(em(6.0))
            .enabled(false, "Why it is off.")
            .show(ui, PickList::new("o", &[]));
    });
    ui.horizontal(|ui| {
        caption(ui, "PickList");
        let items = pick_items();
        // as a popup holds it: as wide as the list
        ui.allocate_ui(Vec2::new(em(18.0) + 16.0, 260.0), |ui| {
            popup_frame(&ctx).show(ui, |ui| {
                let _ = PickList::new("kitsheet.inline", &items)
                    .current("spawn_npc")
                    .remember("kitsheet")
                    .width(em(18.0))
                    .max_height(180.0)
                    .show(ui);
            });
        });
    });

    heading(ui, "Section");
    let _ =
        Section::new("kitsheet.open", "Variables").count("11").action(Icon::Plus, "Add a variable").show(ui);
    ui.indent("kitsheet.section_body", |ui| caption(ui, "the section's body"));
    let _ = Section::new("kitsheet.closed", "More").default_open(false).show(ui);

    heading(ui, "ItemRows");
    let cols = [Col::new("Item", em(14.0)), Col::new("Count", em(5.0))];
    let edit =
        ItemRows::new("kitsheet.rows", &cols, s.rows.len()).add_label("Add Item").show(ui, |ui, row, col| {
            if let Some((name, count)) = s.rows.get_mut(row) {
                if col == 0 {
                    text_field(ui, name);
                } else {
                    let _ = grid::integer(ui, count, 0.2, "");
                }
            }
        });
    if let Some(r) = edit.removed {
        s.rows.remove(r);
    }
    if edit.added {
        s.rows.push((String::new(), 1));
    }
    if let Some((from, to)) = edit.moved {
        crate::rows::apply_move(&mut s.rows, from, to);
    }

    heading(ui, "Table");
    let tcols = [
        Col::new("Tag", em(6.0)).sortable(),
        Col::new("Title", em(12.0)).sortable(),
        Col::new("State", em(6.0)),
    ];
    let data = [
        ("v1.2.0", "Night 7", "Latest"),
        ("v1.1.0", "Night 6", "Published"),
        ("v1.0.0", "First", "Published"),
    ];
    ui.scope(|ui| {
        ui.set_height(26.0 * 4.0 + 24.0);
        let ev = Table::new("kitsheet", &tcols, data.len())
            .selected(s.selected)
            .sorted(0, false)
            .height(26.0 * 3.0 + 4.0)
            .show(ui, |ui, row, col| {
                let (a, b, c) = data[row];
                cell_text(ui, [a, b, c][col], col == 2);
            });
        if let Some(r) = ev.clicked {
            s.selected = Some(r);
        }
    });

    heading(ui, "Menus");
    ui.vertical(|ui| {
        // as wide as a popup makes it: the widest row, 200 pt at least
        ui.set_max_width(metrics::MENU_MIN_W + 60.0);
        popup_frame(&ctx).show(ui, |ui| {
            popup_body(ui, |ui| {
                let _ = menu_item(
                    ui,
                    "Save",
                    MenuItem { icon: Icon::Save, keys: "Ctrl+S", ..MenuItem::default() },
                );
                let _ = menu_item(ui, "Graph Panel", MenuItem { selected: true, ..MenuItem::default() });
                let _ = menu_item(ui, "Reset Layout", MenuItem::default());
                let _ = menu_item(
                    ui,
                    "Off",
                    MenuItem { enabled: false, tip: "Why it is off.", ..MenuItem::default() },
                );
                ui.separator();
                let _ = menu_item(
                    ui,
                    "Delete...",
                    MenuItem { icon: Icon::Delete, danger: true, ..MenuItem::default() },
                );
            });
        });
    });
    note(ui, "One icon column: a row without an icon starts where the others do.");

    heading(ui, "States");
    ui.horizontal(|ui| {
        badge(ui, "Trader", Tone::Info);
        badge(ui, "Template", Tone::Muted);
        badge(ui, "cheat", Tone::Warning);
        status_dot(ui, Tone::Error, "A problem");
        status_dot(ui, Tone::Required, "Required");
        spinner(ui);
    });
    let _ = banner(
        ui,
        Tone::Warning,
        "The file changed on the disk while it had unsaved edits.",
        Some("Reload"),
        Some("Overwrite..."),
    );
    let _ = banner_list(
        ui,
        Tone::Error,
        "The module cannot be built: 2 problems.",
        &[
            ("quests/night7.nqasset: task 'silence' has no title", Some("Open")),
            ("mod.ltx: the version is not a version", None),
        ],
    );
    ui.add_space(metrics::S16);
    ui.scope(|ui| {
        ui.set_height(150.0);
        let _ = empty_state(
            ui,
            Icon::Search,
            "No matches",
            "No command, panel or tab has these words in its name.",
            Some("Clear Search"),
        );
    });

    heading(ui, "Tokens");
    ui.horizontal_wrapped(|ui| {
        for (i, c) in p.graph_family.iter().enumerate() {
            swatch(ui, *c, &format!("family.{i}"));
        }
    });
    ui.horizontal_wrapped(|ui| {
        for (c, n) in [
            (p.wire_base, "wire.base"),
            (p.wire_hot, "wire.hot"),
            (p.wire_selected, "wire.selected"),
            (p.wire_dim, "wire.dim"),
            (p.pin_exec, "pin.exec"),
            (p.pin_input, "pin.input"),
            (p.pin_hot, "pin.hot"),
            (p.pin_target, "pin.target"),
            (p.frame_fill, "frame.fill"),
            (p.frame_title, "frame.title"),
        ] {
            swatch(ui, c, n);
        }
    });
    ui.horizontal(|ui| {
        for (shadow, n) in [
            (p.shadow_popup, "shadow.popup"),
            (p.shadow_dialog, "shadow.dialog"),
            (p.shadow_card, "shadow.card"),
        ] {
            let (r, _) = ui.allocate_exact_size(Vec2::new(120.0, 64.0), Sense::hover());
            let card = Rect::from_min_size(r.min + Vec2::new(12.0, 8.0), Vec2::new(96.0, 40.0));
            ui.painter().add(shadow.as_shape(card, CornerRadius::same(6)));
            ui.painter().rect_filled(card, CornerRadius::same(6), p.overlay);
            ui.painter().text(
                pos2(card.center().x, card.center().y),
                egui::Align2::CENTER_CENTER,
                n,
                Font::Small.id(),
                p.text_secondary,
            );
        }
    });
    ui.add_space(metrics::S24);
    ctx.data_mut(|d| d.insert_temp(id, s));
}
