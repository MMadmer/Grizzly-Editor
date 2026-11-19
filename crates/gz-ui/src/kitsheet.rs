//! The kit sheet (`-kitsheet`): every widget of the kit in every state it has, on one page, and
//! the tokens beside them. It is how a surface's author - or a critic - sees what exists before
//! writing a widget of their own. Its Theme row shows the sheet in any theme without saving the
//! choice (`-theme <id>` starts it in one).

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Ui, Vec2, pos2};

use crate::button::{Button, Kind, icon_button, keycaps, keycaps_width, segmented};
use crate::card::{card, card_busy};
use crate::check::{Check, Switch};
use crate::dialog::{Dialog, DialogSize};
use crate::font::Font;
use crate::form;
use crate::grid::{self, PropFlags, PropertyGrid, number, text_field};
use crate::icons::Icon;
use crate::list::{Column, NavRowSpec, nav_row, table_headers, table_headers_ruled};
use crate::markdown::markdown_view;
use crate::menu::{MenuItem, menu_item, popup_body, popup_frame};
use crate::pick::{Combo, PickItem, PickList};
use crate::rows::{Col, ItemRows, Table, cell_text};
use crate::search::SearchField;
use crate::section::Section;
use crate::state::{
    badge, banner, banner_list, busy_line, empty_state, error_state, spinner, spinner_at, status_dot, toast,
};
use crate::tabs::{PanelTab, tab_strip, tab_width};
use crate::text::{self, Clip};
use crate::theme::{Tone, metrics, palette};
use crate::toolbar::{Toolbar, ToolbarStyle};
use crate::units::em;

/// The Markdown preview's sample: a block of every kind.
const MARKDOWN_SAMPLE: &str = "# Night 7\n\
    A module's **quest** opens with a *briefing*, a ~~draft~~ and the task `nq.night7.start`. \
    See [the format](docs/nq/NQ_FORMAT.md) or [the tasks](#tasks).\n\n\
    ## Tasks\n\n\
    - [x] Meet the trader\n- [ ] Bring the medkit\n  1. Find it\n  2. Carry it back\n\n\
    > The author's note: the trader leaves at dawn.\n\n\
    | Item | Count | Price |\n|:--|:--:|--:|\n| Bread | 2 | 40 |\n| Medkit | 1 | 350 |\n\n\
    ```lua\nnq.give(\"medkit\", 1) -- a long line scrolls sideways rather than wrapping, the way code reads best\n```\n\n\
    ---\n\n\
    ![The trader's hut](pictures/hut.png)\n";

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
    tab: usize,
    page: usize,
    /// The form fields: a hidden one, one with its hint showing, one typed in, one as wide as its row.
    secret: String,
    hinted: String,
    hint_typed: String,
    row_empty: String,
    row_typed: String,
    /// The property grid's values.
    grid_text: String,
    grid_number: f32,
    /// A dialog the sheet shows: the plain one, the one with a job running.
    dialog: bool,
    dialog_busy: bool,
    selected_nav: usize,
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
            tab: 1,
            page: 0,
            secret: String::new(),
            hinted: String::new(),
            hint_typed: "Night 7".to_owned(),
            row_empty: String::new(),
            row_typed: "my_mod.status".to_owned(),
            grid_text: "Night 7".to_owned(),
            grid_number: 12.0,
            dialog: false,
            dialog_busy: false,
            selected_nav: 1,
        }
    }
}

fn heading(ui: &mut Ui, title: &'static str) {
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

fn pick_items(p: &crate::theme::Palette) -> Vec<PickItem> {
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

    heading(ui, "Theme");
    ui.horizontal(|ui| {
        caption(ui, "Shown in");
        let current = crate::theme::Theme::find(p.theme).unwrap_or_else(crate::theme::Theme::default_theme);
        ui.scope(|ui| {
            ui.set_width(em(16.0));
            if let Some(t) = crate::theme::theme_combo(ui, "kitsheet.theme", current) {
                crate::theme::install(&ctx, t);
                ctx.request_repaint();
            }
        });
    });

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
    ui.horizontal(|ui| {
        caption(ui, "Keycaps");
        // what is bound now, as menu rows and tooltips show it
        for (keys, enabled) in [("Ctrl+S", true), ("Ctrl+Shift+P", true), ("F5", false)] {
            let (r, _) =
                ui.allocate_exact_size(Vec2::new(keycaps_width(ui, keys), metrics::S16), Sense::hover());
            keycaps(ui, pos2(r.max.x, r.center().y), keys, enabled);
            ui.add_space(metrics::S8);
        }
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

    heading(ui, "Panel tabs");
    ui.horizontal(|ui| {
        caption(ui, "Docked");
        // a dock lays its tabs out: the front one has the cross, the others show it on hover
        let (strip, _) = ui.allocate_exact_size(Vec2::new(em(24.0), metrics::PANEL_TABS), Sense::hover());
        ui.painter().rect_filled(strip, CornerRadius::ZERO, p.chrome);
        let mut x = strip.min.x + 2.0;
        for (i, label) in ["Outliner", "Details", "Place"].into_iter().enumerate() {
            let w = tab_width(&ctx, label, true);
            let r = Rect::from_min_size(pos2(x, strip.min.y + 2.0), Vec2::new(w, metrics::PANEL_TABS - 2.0));
            x += w;
            let id = egui::Id::unique(("gz-ui.kitsheet.tab", i));
            if PanelTab::new(id, r, label).front(s.tab == i).closable(true).show(ui).clicked {
                s.tab = i;
            }
        }
    });
    ui.horizontal(|ui| {
        caption(ui, "Flat strip");
        ui.scope(|ui| {
            ui.set_width(em(18.0));
            let labels = ["Problems (2)", "Pages", "Vars", "Log"].map(crate::Tx::from);
            let _ = tab_strip(ui, "kitsheet", &labels, &mut s.page);
        });
    });
    note(ui, "The strip shares its width among the tabs when it is too narrow for their labels.");

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
        caption(ui, "Form fields");
        // with the hint showing, typed in, and hidden (empty, then typed)
        form::hint_field(ui, egui::Id::unique("kitsheet.hint"), &mut s.hinted, "my_mod", em(8.0));
        form::hint_field(ui, egui::Id::unique("kitsheet.hint_typed"), &mut s.hint_typed, "my_mod", em(8.0));
    });
    ui.horizontal(|ui| {
        caption(ui, "Hidden");
        form::secret_field(ui, egui::Id::unique("kitsheet.secret"), &mut s.secret, "Password", em(9.0));
        let mut typed = "hunter2".to_owned();
        form::secret_field(ui, egui::Id::unique("kitsheet.secret_typed"), &mut typed, "Password", em(9.0));
    });
    ui.horizontal(|ui| {
        caption(ui, "As wide as its row");
        ui.vertical(|ui| {
            ui.set_width(em(24.0));
            form::row_field(ui, "kitsheet.row_empty", &mut s.row_empty, "my_mod.status");
            form::row_field(ui, "kitsheet.row_typed", &mut s.row_typed, "my_mod.status");
        });
    });
    ui.horizontal(|ui| {
        caption(ui, "Combo");
        ui.scope(|ui| {
            ui.set_width(em(10.0));
            grid::combo(ui, "kitsheet.combo", &mut s.combo, &["storyline", "side", "restrictor"]);
        });
        let items = pick_items(&p);
        let shown = items.iter().find(|i| i.id == s.pick).map_or(crate::Tx::EMPTY, |i| i.label.clone());
        let list = PickList::new("kitsheet.pick", &items).current(&s.pick).remember("kitsheet");
        if let Some(i) = Combo::new("kitsheet.pick", shown).icon(Icon::Flow).width(em(10.0)).show(ui, list)
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
        // a value the list does not hold, nothing chosen, a list still being read
        let _ = Combo::new("kitsheet.warn", "not_in_the_list")
            .width(em(9.0))
            .warn(true)
            .tip("Kept as written.")
            .show(ui, PickList::new("w", &[]));
        let _ = Combo::new("kitsheet.none", "None")
            .width(em(6.0))
            .placeholder(true)
            .show(ui, PickList::new("n", &[]));
        let _ = Combo::new("kitsheet.busy", "Reading")
            .width(em(7.0))
            .busy(true, "The list is being read.")
            .show(ui, PickList::new("b", &[]));
    });
    ui.horizontal(|ui| {
        caption(ui, "PickList");
        let items = pick_items(&p);
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
    ui.horizontal(|ui| {
        caption(ui, "PickList, grouped");
        let themes = crate::theme::pick_items();
        ui.allocate_ui(Vec2::new(em(18.0) + 16.0, 260.0), |ui| {
            popup_frame(&ctx).show(ui, |ui| {
                // no search of its own: the list above keeps the keys
                let _ = PickList::new("kitsheet.grouped", &themes)
                    .without_search()
                    .current(p.theme)
                    .width(em(18.0))
                    .max_height(180.0)
                    .show(ui);
            });
        });
    });
    ui.horizontal(|ui| {
        caption(ui, "PickList, empty");
        ui.allocate_ui(Vec2::new(em(18.0) + 16.0, 90.0), |ui| {
            popup_frame(&ctx).show(ui, |ui| {
                let _ = PickList::new("kitsheet.none_to_pick", &[]).width(em(18.0)).max_height(80.0).show(ui);
            });
        });
    });
    ui.horizontal(|ui| {
        caption(ui, "Checker");
        // under a picture with transparency: 8 pt cells, 4 in a small one
        for (side, cell) in [(64.0, 8.0), (24.0, 4.0)] {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
            crate::checker(ui.painter(), rect.min, rect, cell);
        }
    });

    heading(ui, "Section");
    let _ =
        Section::new("kitsheet.open", "Variables").count("11").action(Icon::Plus, "Add a variable").show(ui);
    ui.indent("kitsheet.section_body", |ui| caption(ui, "the section's body"));
    let _ = Section::new("kitsheet.closed", "More").default_open(false).show(ui);
    // a list still being read has no count: a spinner stands where the count would
    let _ =
        Section::new("kitsheet.reading", "Objects").default_open(false).busy("Reading the list...").show(ui);

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

    ui.add_space(metrics::S8);
    let _ = ItemRows::new("kitsheet.rows_empty", &cols, 0)
        .add_label("Add Item")
        .empty_text("Nothing in the kit yet.")
        .show(ui, |_, _, _| {});

    heading(ui, "Property grid");
    PropertyGrid::new("kitsheet").show(ui, |g| {
        g.row("Title", "What the player reads.", PropFlags::REQUIRED, 0, |ui| {
            text_field(ui, &mut s.grid_text)
        });
        g.row("Duration", "How long it lasts.", PropFlags::MODIFIED, 0, |ui| {
            number(ui, &mut s.grid_number, 0.1, "s")
        });
        g.row("Version", "Not a version.", PropFlags::INVALID, 0, |ui| {
            let mut v = "1.x".to_owned();
            text_field(ui, &mut v)
        });
        g.message(Tone::Error, "The version is not a version.");
        g.row("Source", "Where it was made.", PropFlags::READ_ONLY, 0, |ui| {
            crate::grid::read_only(ui, r"gamedata\configs\night7.ltx", Clip::Start)
        });
        g.row("Radius", "Two objects differ.", PropFlags::MIXED, 0, |ui| {
            let mut v = 0.0;
            number(ui, &mut v, 0.1, "m")
        });
    });

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

    heading(ui, "Markdown");
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(metrics::RADIUS))
        .show(ui, |ui| {
            ui.set_height(em(20.0));
            let mut goto = 0;
            let _ = markdown_view(ui, "kitsheet.markdown", MARKDOWN_SAMPLE, &mut goto);
        });
    note(ui, "Only what is in view is laid out. A link goes back to the caller; an anchor link scrolls.");

    heading(ui, "Lists");
    let cols = [Column::new("Name", em(8.0)), Column::new("Count", em(4.0)), Column::new("On", em(3.0))];
    ui.horizontal(|ui| {
        caption(ui, "Headers");
        ui.vertical(|ui| {
            ui.set_width(em(15.0));
            table_headers(ui, &cols);
            ui.add_space(metrics::S4);
            // the look of the header of a Table, for rows drawn by the caller
            table_headers_ruled(ui, &cols);
        });
    });
    ui.horizontal(|ui| {
        caption(ui, "Nav rows");
        ui.vertical(|ui| {
            ui.set_width(em(15.0));
            let rows = [
                (
                    "Quests",
                    NavRowSpec {
                        icon: Icon::Quest,
                        count: Some(crate::Tx::raw("12")),
                        ..NavRowSpec::default()
                    },
                ),
                (
                    "Characters",
                    NavRowSpec { icon: Icon::Character, dot: Some(Tone::Error), ..NavRowSpec::default() },
                ),
                ("Behaviours", NavRowSpec { icon: Icon::Behaviour, dim: true, ..NavRowSpec::default() }),
                (
                    "PDA",
                    NavRowSpec {
                        icon: Icon::PdaInterface,
                        action: Icon::Close,
                        action_tip: "Remove".into(),
                        ..NavRowSpec::default()
                    },
                ),
            ];
            for (i, (label, spec)) in rows.iter().enumerate() {
                if nav_row(ui, label, *label, s.selected_nav == i, spec).clicked {
                    s.selected_nav = i;
                }
            }
        });
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
                    MenuItem { enabled: false, tip: "Why it is off.".into(), ..MenuItem::default() },
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
        // inside a field that is busy
        let (field, _) = ui.allocate_exact_size(Vec2::new(em(8.0), metrics::FRAME), Sense::hover());
        ui.painter().rect_filled(field, CornerRadius::same(metrics::RADIUS), p.input);
        spinner_at(
            &ctx,
            ui.painter(),
            Rect::from_center_size(
                pos2(field.max.x - 14.0, field.center().y),
                Vec2::splat(metrics::GLYPH_STATUS),
            ),
        );
    });
    let _ = banner(ui, Tone::Info, "A newer version of the game is linked.", None, None);
    let _ = banner(
        ui,
        Tone::Warning,
        "The file changed on the disk while it had unsaved edits.",
        Some("Reload".into()),
        Some("Overwrite...".into()),
    );
    let _ = banner_list(
        ui,
        Tone::Error,
        "The module cannot be built: 2 problems.",
        &[
            (crate::Tx::raw("quests/night7.nqasset: task 'silence' has no title"), Some("Open".into())),
            (crate::Tx::raw("mod.ltx: the version is not a version"), None),
        ],
    );
    ui.add_space(metrics::S16);
    ui.add_space(metrics::S8);
    ui.horizontal(|ui| {
        caption(ui, "Busy line");
        ui.vertical(|ui| busy_line(ui, "Reading the library..."));
    });
    ui.horizontal(|ui| {
        caption(ui, "Card");
        ui.vertical(|ui| {
            ui.set_width(em(24.0));
            card(ui, "kitsheet.card", "Author Key", Tone::Required, true, |ui| {
                ui.label(
                    egui::RichText::new("A key is needed to sign a release.")
                        .font(Font::Body.id())
                        .color(p.text_secondary),
                );
            });
            // while the part it holds is worked out
            card(ui, "kitsheet.card_busy", "Requirements", Tone::Info, false, |ui| {
                card_busy(ui, "Checking the release...");
            });
        });
    });
    ui.horizontal(|ui| {
        caption(ui, "Toasts");
        for (label, tone, text_) in [
            ("Info", Tone::Info, "Scene saved."),
            ("Success", Tone::Success, "The module was built."),
            ("Warning", Tone::Warning, "2 files were left out."),
            ("Error", Tone::Error, "The build stopped."),
        ] {
            if Button::new(label).show(ui).clicked() {
                toast(&ctx, tone, text_);
            }
        }
    });
    ui.horizontal(|ui| {
        caption(ui, "Dialogs");
        if Button::new("Dialog").show(ui).clicked() {
            s.dialog = true;
        }
        if Button::new("Dialog With a Job").show(ui).clicked() {
            s.dialog_busy = true;
        }
    });
    let (mut open, mut closed) = (s.dialog, false);
    let _ = Dialog::new("Rename Quest", "kitsheet", DialogSize::S).show(
        ui,
        &mut open,
        |ui| {
            ui.label("The quest keeps its tasks and its variables.");
        },
        |f| {
            f.status("The name is used by one other quest.", Tone::Warning);
            closed |= f.button("Cancel", Kind::Secondary, false, true, "");
            closed |= f.button("Rename", Kind::Primary, true, true, "");
        },
    );
    s.dialog = open && !closed;
    let (mut open, mut closed) = (s.dialog_busy, false);
    let _ = Dialog::new("Build to Folder", "kitsheet_busy", DialogSize::S).show(
        ui,
        &mut open,
        |ui| {
            ui.label("The module is built beside the editor.");
        },
        |f| {
            f.busy("Copying the files...");
            closed |= f.button("Stop Building", Kind::Secondary, false, true, "");
        },
    );
    s.dialog_busy = open && !closed;
    ui.scope(|ui| {
        ui.set_height(210.0);
        let _ = error_state(
            ui,
            "This model cannot be shown",
            "The file is not a model the viewer reads.",
            Some("Open Another".into()),
        );
    });
    ui.scope(|ui| {
        ui.set_height(210.0);
        let _ = empty_state(
            ui,
            Icon::Search,
            "No matches",
            "No command, panel or tab has these words in its name.",
            Some("Clear Search".into()),
        );
    });

    heading(ui, "Tokens");
    for row in [
        &[
            (p.chrome, "chrome"),
            (p.base, "base"),
            (p.inset, "inset"),
            (p.canvas, "canvas"),
            (p.raised, "raised"),
            (p.overlay, "overlay"),
            (p.input, "input"),
            (p.input_hover, "input.hover"),
        ][..],
        &[
            (p.hover, "hover"),
            (p.pressed, "pressed"),
            (p.button, "button"),
            (p.button_hover, "button.hover"),
            (p.border_subtle, "border.subtle"),
            (p.border, "border"),
            (p.border_strong, "border.strong"),
            (p.dim, "dim"),
        ][..],
        &[
            (p.text, "text"),
            (p.text_secondary, "text.secondary"),
            (p.text_muted, "text.muted"),
            (p.text_disabled, "text.disabled"),
            (p.accent, "accent"),
            (p.accent_bright, "accent.bright"),
            (p.selection, "selection"),
            (p.selection_focused, "selection.focused"),
        ][..],
        &[
            (p.primary, "primary"),
            (p.primary_hover, "primary.hover"),
            (p.primary_pressed, "primary.pressed"),
            (p.danger, "danger"),
            (p.danger_hover, "danger.hover"),
            (p.danger_pressed, "danger.pressed"),
            (p.on_fill, "on_fill"),
        ][..],
        &[
            (p.error, "error"),
            (p.warning, "warning"),
            (p.success, "success"),
            (p.required, "required"),
            (p.axis_x, "axis.x"),
            (p.axis_y, "axis.y"),
            (p.axis_z, "axis.z"),
        ][..],
        &[
            (p.grid_line, "grid.line"),
            (p.scene_grid, "scene.grid"),
            (p.scene_grid_major, "scene.grid.major"),
            (p.scene_wire, "scene.wire"),
            (p.scene_label_shadow, "scene.label.shadow"),
            (p.scroll_handle, "scroll.handle"),
            (p.scroll_handle_hover, "scroll.handle.hover"),
        ][..],
    ] {
        ui.horizontal_wrapped(|ui| {
            for (c, n) in row {
                swatch(ui, *c, n);
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        for tone in [Tone::Muted, Tone::Info, Tone::Success, Tone::Warning, Tone::Error, Tone::Required] {
            badge(ui, crate::Tx::raw(format!("{tone:?}")), tone);
        }
    });
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
