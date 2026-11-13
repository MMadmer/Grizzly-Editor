//! The kit's widgets driven through egui frames: what a person does to them and what they answer.

use egui::{Id, Key, Pos2, Rect, Vec2, pos2};

use crate::menu::{MenuItem, menu_item, popup_body};
use crate::pick::{PickItem, PickList};
use crate::rows::{Col, ItemRows, RowsEdit};
use crate::section::{self, Section};
use crate::testkit::{click_at, context, frame, key, typed};
use crate::{Icon, Kind};

fn items() -> Vec<PickItem> {
    ["Wait", "Spawn NPC", "Give Item", "App Message", "Spawn Squad"]
        .iter()
        .enumerate()
        .map(|(i, l)| PickItem::new(format!("k{i}"), *l).icon(Icon::Flow))
        .collect()
}

#[test]
fn a_pick_list_ranks_what_is_typed_and_the_keyboard_picks() {
    let ctx = context();
    let items = items();
    let mut picked = None;
    let mut t = 0.0;
    let mut run = |events: Vec<egui::Event>, picked: &mut Option<usize>| {
        t += 0.1;
        frame(&ctx, t, events, |ui| {
            let out = PickList::new("test.pick", &items).remember("test").show(ui);
            if out.item.is_some() {
                *picked = out.item;
            }
        });
    };
    // the first frame gives the search the keyboard; then a word, Down, Enter
    run(Vec::new(), &mut picked);
    run(vec![typed("spa")], &mut picked);
    run(vec![key(Key::ArrowDown)], &mut picked);
    assert_eq!(picked, None);
    run(vec![key(Key::Enter)], &mut picked);
    // "spa": Spawn NPC first (the shorter), Down to Spawn Squad
    assert_eq!(picked, Some(4));
    // what was picked is the list's recent pick now: it leads the next time it opens
    assert_eq!(crate::layout::list(&ctx, "pick.test.recent"), vec!["k4".to_owned()]);
    // Esc closes without a pick
    let mut closed = false;
    frame(&ctx, t + 1.0, vec![key(Key::Escape)], |ui| {
        closed = PickList::new("test.pick", &items).show(ui).close;
    });
    assert!(closed);
}

#[test]
fn a_section_keeps_what_the_author_left_it_as() {
    let ctx = context();
    let mut header = Rect::NOTHING;
    let mut open = None;
    frame(&ctx, 0.0, Vec::new(), |ui| {
        let r = Section::new("test.vars", "Variables").count("3").show(ui);
        header = r.rect;
        open = Some(r.open);
    });
    assert_eq!(open, Some(true), "open until folded");
    for (i, events) in click_at(pos2(header.min.x + 40.0, header.center().y)).into_iter().enumerate() {
        frame(&ctx, 0.1 + i as f64 * 0.05, events, |ui| {
            open = Some(Section::new("test.vars", "Variables").show(ui).open);
        });
    }
    assert_eq!(open, Some(false));
    // kept in the layout store, so the next surface (and the next run) finds it folded
    assert!(!section::is_open(&ctx, "test.vars", true));
    assert!(crate::layout::all(&ctx).iter().any(|(k, v)| k == "section.test.vars" && *v == 0.0));
}

#[test]
fn item_rows_add_remove_and_reorder() {
    let ctx = context();
    let cols = [Col::new("Item", 200.0), Col::new("Count", 80.0)];
    let mut edits: Vec<RowsEdit> = Vec::new();
    let draw = |t: f64, events: Vec<egui::Event>, edits: &mut Vec<RowsEdit>| -> (Pos2, f32) {
        let mut at = (Pos2::ZERO, 0.0);
        frame(&ctx, t, events, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            at = (ui.cursor().min, ui.available_width());
            let e = ItemRows::new("test.rows", &cols, 3).show(ui, |_ui, _r, _c| {});
            if e != RowsEdit::default() {
                edits.push(e);
            }
        });
        at
    };
    let (origin, width) = draw(0.0, Vec::new(), &mut edits);
    let row_h = crate::theme::metrics::PROPERTY_ROW + 4.0;
    let head = 24.0;
    let row_y = |k: f32| origin.y + head + k * row_h + row_h * 0.5;
    // the Add row under the three rows
    let mut t = 0.1;
    for events in click_at(pos2(origin.x + 60.0, row_y(3.0))) {
        t += 0.05;
        draw(t, events, &mut edits);
    }
    assert_eq!(edits.pop().map(|e| e.added), Some(true));
    // the cross of the second row
    let cross_x = origin.x + width - (crate::theme::metrics::ICON_BUTTON + 4.0) + 2.0 + 12.0;
    for events in click_at(pos2(cross_x, row_y(1.0))) {
        t += 0.05;
        draw(t, events, &mut edits);
    }
    assert_eq!(edits.pop().and_then(|e| e.removed), Some(1));
    // the first row's grip dragged two rows down
    let grip = pos2(origin.x + 9.0, row_y(0.0));
    let to = pos2(origin.x + 9.0, row_y(2.0));
    let press = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let steps = vec![
        vec![egui::Event::PointerMoved(grip)],
        vec![press(grip, true)],
        vec![egui::Event::PointerMoved(pos2(grip.x, grip.y + 12.0))],
        vec![egui::Event::PointerMoved(pos2(grip.x, grip.y + 40.0))],
        vec![egui::Event::PointerMoved(to)],
        vec![press(to, false)],
        vec![],
    ];
    for events in steps {
        t += 0.05;
        draw(t, events, &mut edits);
    }
    assert_eq!(edits.pop().and_then(|e| e.moved), Some((0, 2)));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "two Primary buttons")]
fn a_second_primary_in_one_footer_stops_a_debug_build() {
    let ctx = context();
    frame(&ctx, 0.0, Vec::new(), |ui| {
        let scope = Id::unique("test.footer");
        crate::note_primary(ui.ctx(), scope, "dialog footer", "Build");
        // the same button drawn again in a sizing pass counts once
        crate::note_primary(ui.ctx(), scope, "dialog footer", "Build");
        assert_eq!(crate::button::primaries(ui.ctx(), scope), 1);
        crate::note_primary(ui.ctx(), scope, "dialog footer", "Package");
    });
}

#[test]
fn a_toolbar_with_one_primary_is_fine() {
    let ctx = context();
    frame(&ctx, 0.0, Vec::new(), |ui| {
        let mut tb = crate::Toolbar::begin(ui, "test.tb", crate::ToolbarStyle::Row);
        let _ = tb.action("a", Icon::Save, "Save", "", true, false, Kind::Ghost);
        let _ = tb.action("b", Icon::BuildMod, "Build Mod", "", true, false, Kind::Primary);
        tb.end();
    });
}

#[test]
fn a_menus_labels_start_at_one_x() {
    let ctx = context();
    let label_x =
        |ctx: &egui::Context| ctx.data(|d| d.get_temp::<f32>(Id::unique("gz-ui.menu.test_label_x")));
    let mut xs: Vec<f32> = Vec::new();
    for t in 0..2 {
        xs.clear();
        frame(&ctx, f64::from(t), Vec::new(), |ui| {
            egui::Area::new(Id::unique("test.menu")).show(ui.ctx(), |ui| {
                popup_body(ui, |ui| {
                    // a row with an icon, one without, one with a check
                    let _ = menu_item(
                        ui,
                        "Save",
                        MenuItem { icon: Icon::Save, keys: "Ctrl+S", ..MenuItem::default() },
                    );
                    xs.push(label_x(ui.ctx()).unwrap_or(0.0));
                    let _ = menu_item(ui, "Reset Layout", MenuItem::default());
                    xs.push(label_x(ui.ctx()).unwrap_or(0.0));
                    let _ = menu_item(ui, "Graph Panel", MenuItem { selected: true, ..MenuItem::default() });
                    xs.push(label_x(ui.ctx()).unwrap_or(0.0));
                });
            });
        });
    }
    assert_eq!(xs.len(), 3);
    assert!(xs.iter().all(|x| (x - xs[0]).abs() < 0.01), "{xs:?}");
    // a popup without an icon or a check anywhere keeps no column
    let mut plain = Vec::new();
    for t in 2..4 {
        plain.clear();
        frame(&ctx, f64::from(t), Vec::new(), |ui| {
            egui::Area::new(Id::unique("test.menu.plain")).show(ui.ctx(), |ui| {
                popup_body(ui, |ui| {
                    let _ = menu_item(ui, "Copy", MenuItem::default());
                    plain.push(label_x(ui.ctx()).unwrap_or(0.0));
                });
            });
        });
    }
    assert!(plain[0] < xs[0], "{plain:?} {xs:?}");
}
