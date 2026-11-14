//! The kit's widgets driven through egui frames: what a person does to them and what they answer.

use egui::{Id, Key, Pos2, Rect, Vec2, pos2};

use crate::menu::{MenuItem, menu_item, popup_body};
use crate::pick::{PickItem, PickList};
use crate::rows::{Col, ItemRows, RowsEdit};
use crate::section::{self, Section};
use crate::tabs::{PanelTab, TabResponse, TabStyle, flat_widths, tab_strip, tab_width};
use crate::testkit::{click_at, context, frame, key, secondary_click_at, typed};
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

#[test]
fn a_tab_is_as_wide_as_its_label_and_its_padding() {
    let ctx = context();
    let mut widths = (0.0, 0.0, 0.0);
    // the faces exist once a frame has run
    frame(&ctx, 0.0, Vec::new(), |_| {
        widths = (
            tab_width(&ctx, "Outliner", true),
            tab_width(&ctx, "Outliner", false),
            tab_width(&ctx, "Outliner Details", false),
        );
    });
    let (closable, plain, longer) = widths;
    // the cross's slot costs ten points more than the flat tab's padding
    assert!((closable - plain - 10.0).abs() < 0.01, "{closable} {plain}");
    assert!(longer > plain);
}

#[test]
fn the_tabs_of_a_flat_strip_give_way_together() {
    // they fit: nothing changes
    assert_eq!(flat_widths(&[60.0, 80.0], 200.0), vec![60.0, 80.0]);
    // they do not: every one keeps its share of what the strip has
    let narrow = flat_widths(&[60.0, 120.0], 90.0);
    assert!((narrow.iter().sum::<f32>() - 90.0).abs() < 0.01);
    assert!((narrow[1] / narrow[0] - 2.0).abs() < 0.01);
    assert!(flat_widths(&[], 90.0).is_empty());
}

#[test]
fn a_docked_tab_answers_a_click_a_cross_and_a_menu() {
    let ctx = context();
    let rect = Rect::from_min_size(pos2(10.0, 10.0), Vec2::new(110.0, 24.0));
    let id = Id::unique("test.tab");
    let run = |t: f64, events: Vec<egui::Event>, front: bool| {
        let mut out = TabResponse::default();
        frame(&ctx, t, events, |ui| {
            out = PanelTab::new(id, rect, "Outliner").front(front).closable(true).show(ui);
        });
        out
    };
    let ask = |events: Vec<Vec<egui::Event>>, front: bool, base: f64| {
        let mut answers = Vec::new();
        for (i, e) in events.into_iter().enumerate() {
            answers.push(run(base + i as f64 * 0.05, e, front));
        }
        answers
    };
    // on the label
    let on_label = ask(click_at(pos2(40.0, 22.0)), false, 0.0);
    assert!(on_label.iter().any(|a| a.clicked && !a.close), "{on_label:?}");
    // on the cross: it closes and is no click of the tab
    let on_cross = ask(click_at(pos2(rect.max.x - 12.0, rect.center().y)), true, 1.0);
    assert!(on_cross.iter().any(|a| a.close && !a.clicked), "{on_cross:?}");
    // a right click asks for the tab's menu
    let menu = ask(secondary_click_at(pos2(40.0, 22.0)), false, 2.0);
    assert!(menu.iter().any(|a| a.menu), "{menu:?}");
    // a tab that cannot close has no cross to hit
    let mut closed = false;
    for (i, events) in click_at(pos2(rect.max.x - 12.0, rect.center().y)).into_iter().enumerate() {
        frame(&ctx, 3.0 + i as f64 * 0.05, events, |ui| {
            closed |= PanelTab::new(id, rect, "Outliner").style(TabStyle::Flat).show(ui).close;
        });
    }
    assert!(!closed);
}

#[test]
fn a_tab_strip_changes_its_choice_by_a_click() {
    let ctx = context();
    let labels: Vec<crate::Tx<'static>> = ["Problems", "Pages", "Vars", "Log"].map(crate::Tx::from).to_vec();
    let mut current = 0usize;
    let mut origin = Pos2::ZERO;
    let mut width = 0.0;
    let mut changed = false;
    frame(&ctx, 0.0, Vec::new(), |ui| {
        origin = ui.cursor().min;
        width = ui.available_width();
        tab_strip(ui, "test.strip", &labels, &mut current);
    });
    // the second tab: after the first one's label and padding
    let mut first = 0.0;
    frame(&ctx, 0.05, Vec::new(), |_| first = tab_width(&ctx, &labels[0], false));
    let at = pos2(origin.x + first + 10.0, origin.y + 13.0);
    assert!(width > first);
    for (i, events) in click_at(at).into_iter().enumerate() {
        frame(&ctx, 0.1 + i as f64 * 0.05, events, |ui| {
            changed |= tab_strip(ui, "test.strip", &labels, &mut current);
        });
    }
    assert!(changed);
    assert_eq!(current, 1);
}

/// A combo whose list is still being read takes no press and says why; one that is ready opens
/// its list on a press, from an empty search.
#[test]
fn a_busy_combo_takes_no_press_and_a_ready_one_opens_its_list() {
    use crate::pick::PickItem;
    use crate::{Combo, PickList};
    let ctx = context();
    let items = vec![PickItem::new("a", "Alpha"), PickItem::new("b", "Beta")];
    // one frame: where the field stood and the id of its popup
    let run = |t: f64, events: Vec<egui::Event>, busy: bool| {
        let mut out = (Rect::NOTHING, Id::NULL);
        frame(&ctx, t, events, |ui| {
            let combo = Combo::new("test.combo", "Alpha").width(120.0).busy(busy, "The list is being read.");
            let popup = combo.popup_id();
            let before = ui.cursor().min;
            let _ = combo.show(ui, PickList::new("test.combo", &items).current("a"));
            out = (Rect::from_min_size(before, Vec2::new(120.0, crate::metrics::FRAME)), popup);
        });
        out
    };
    let (field, popup) = run(0.0, Vec::new(), true);
    for (i, e) in click_at(field.center()).into_iter().enumerate() {
        let _ = run(0.05 + i as f64 * 0.05, e, true);
    }
    assert!(!egui::Popup::is_id_open(&ctx, popup), "a busy combo opened");
    for (i, e) in click_at(field.center()).into_iter().enumerate() {
        let _ = run(1.0 + i as f64 * 0.05, e, false);
    }
    assert!(egui::Popup::is_id_open(&ctx, popup), "a ready combo did not open");
}

/// What an audited frame reports as `loading`: the control `test_ui_shell` and the tour's wait
/// stand on. One entry per busy mark drawn, none for a frame without one, and none at all when
/// nobody asked for the audit.
#[test]
fn an_audited_frame_reports_every_busy_mark_and_only_those() {
    use crate::audit::{self, AuditEntry};
    use crate::dialog::{Dialog, DialogSize};
    let ctx = context();
    let loading = |entries: &[AuditEntry]| -> Vec<String> {
        entries
            .iter()
            .filter_map(|e| match e {
                AuditEntry::Loading { what, .. } => Some(what.clone()),
                _ => None,
            })
            .collect()
    };
    let mut t = 0.0;
    // one frame with the given marks drawn; `audited` says whether anybody asked
    let mut run = |audited: bool, line: bool, dialog: bool, card: bool, section: bool| {
        t += 0.1;
        if audited {
            audit::begin(&ctx);
        }
        let mut open = dialog;
        frame(&ctx, t, Vec::new(), |ui| {
            if line {
                crate::state::busy_line(ui, "Reading the list...");
            }
            if card {
                crate::card::card_busy(ui, "Working out the preview...");
            }
            if section {
                Section::new("test.busy", "Objects").busy("Objects are being read").show(ui);
            }
            let _ = Dialog::new("Build", "busy_test", DialogSize::S).show(
                ui,
                &mut open,
                |ui| {
                    ui.label("body");
                },
                |f| f.busy("Compiling the level"),
            );
        });
        loading(&audit::take(&ctx))
    };
    // nothing busy: nothing reported
    assert_eq!(run(true, false, false, false, false), Vec::<String>::new());
    // each mark alone gives exactly one entry, with the words it shows
    assert_eq!(run(true, true, false, false, false), ["Reading the list..."]);
    assert_eq!(run(true, false, false, true, false), ["Working out the preview..."]);
    assert_eq!(run(true, false, false, false, true), ["Objects are being read"]);
    // a dialog's footer: the first frame of a dialog draws it too
    assert_eq!(run(true, false, true, false, false), ["Compiling the level"]);
    // all together: one each
    assert_eq!(run(true, true, true, true, true).len(), 4);
    // an ordinary frame, nobody asked: nothing collected
    assert_eq!(run(false, true, true, true, true), Vec::<String>::new());
}
