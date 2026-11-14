//! The World Outliner panel (`UIWorldOutliner.cpp`): the search, Filters with a badge, More,
//! the eye column and the names, class rows with their eye and count, By Type or By
//! Hierarchy, the empty states, the footer, Rename (`dlg.rename_object`) and Delete
//! (`dlg.delete_object`, the kit's confirmation naming what goes).
//!
//! The rows come from a `Query` keyed by the scene's revision and the filter
//! ([`crate::outliner_model::compute`]); only the rows that show are laid out.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Rect, ScrollArea, Sense, Ui, Vec2, pos2};
use gz_base::SmolStr;
use gz_editor::{Args, Editor, Value};
use gz_query::Query;
use gz_scene::apply::SelectMode;
use gz_scene::{ObjClass, ObjId, SceneCmd, SceneService, SceneState};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Answer, Asker, ConfirmSpec, Dialog, DialogSize, Font, Icon, Kind, MenuItem, SearchField, Tone, audit,
    icons, menu_item, metrics, palette, state,
};

use crate::classes;
use crate::local::{local_badge_button, local_eye_toggle};
use crate::outliner_model::{Filter, Folds, Listing, Row, Sort, Visibility, compute};

const ROW_H: f32 = metrics::ROW;
const ICON: f32 = 14.0;
const INDENT: f32 = 14.0;
const WINDOW: &str = "World Outliner";

/// The rename dialog's state.
#[derive(Clone, Debug)]
struct Rename {
    id: ObjId,
    original: SmolStr,
    buf: String,
    error: String,
    focus: bool,
}

/// The panel's view state; the scene is the store's.
pub struct Outliner {
    pub filter: Filter,
    pub folds: Folds,
    query: Query<(Filter, Folds), Listing>,
    pub focus_search: bool,
    /// The row a Shift+click extends from.
    anchor: Option<ObjId>,
    /// The panel's own clicks do not scroll: their row is under the pointer.
    skip_scroll: bool,
    sel_sig: u64,
    scroll_target: Option<ObjId>,
    rename: Option<Rename>,
    show_type: bool,
    show_owner: bool,
    /// Last frame's scroll and view height, pt.
    view: (f32, f32),
}

impl Outliner {
    #[must_use]
    pub fn new(editor: &Editor) -> Self {
        Self {
            filter: Filter::default(),
            folds: Folds::default(),
            query: Query::new(editor.exec.clone()),
            focus_search: false,
            anchor: None,
            skip_scroll: false,
            sel_sig: 0,
            scroll_target: None,
            rename: None,
            show_type: false,
            show_owner: false,
            view: (0.0, 0.0),
        }
    }

    /// `ClearFilters`.
    pub fn clear_filters(&mut self) {
        self.filter.text.clear();
        self.filter.selected_only = false;
        self.filter.visibility = Visibility::All;
        self.filter.hidden.clear();
    }

    /// The listing for the snapshot there is, fresh or the last one while a new one comes.
    fn listing(&self, snap: &Arc<gz_store::Snap<SceneState>>) -> Option<Arc<Listing>> {
        let key = (self.filter.clone(), self.folds.clone());
        let (s, f, d) = (snap.clone(), self.filter.clone(), self.folds.clone());
        self.query.get(&key, snap.rev, move || compute(&s, &f, &d)).value().cloned()
    }

    /// The panel's body.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>) {
        let svc = editor.project_opt().and_then(|s| s.services.get::<SceneService>());
        let Some(svc) = svc.filter(|s| s.store.snapshot().valid) else {
            state::empty_state(
                ui,
                Icon::Level,
                "No scene",
                "Open or create a scene to see its objects here.",
                None,
            );
            return;
        };
        let snap = svc.store.snapshot();
        let Some(listing) = self.listing(&snap) else {
            self.toolbar(ui, &svc, None);
            state::busy_line(ui, "Listing the scene...");
            return;
        };

        // a selection made elsewhere (the viewport, MCP) brings its last row into view
        if listing.selection_sig != self.sel_sig {
            self.sel_sig = listing.selection_sig;
            if !self.skip_scroll
                && let Some(last) = listing.last_selected
            {
                self.reveal(&snap, last);
                self.scroll_target = Some(last);
            }
            self.skip_scroll = false;
        }

        self.toolbar(ui, &svc, Some(&listing));
        // the footer, and the item spacing between the header, the rows and it
        let footer_h = Font::Small.line() + 16.0;
        let body_h = (ui.available_height() - footer_h).max(ROW_H * 2.0);
        if listing.total == 0 || listing.shown == 0 {
            ui.allocate_ui(Vec2::new(ui.available_width(), body_h), |ui| {
                if listing.total == 0 {
                    state::empty_state(ui, Icon::SceneObject, "The scene is empty", "Pick a tool in Tools and click in the viewport, or drag an asset in from the Content Browser.", None);
                } else if state::empty_state(ui, Icon::Search, "Nothing matches", "No object passes the search and the filters.", Some("Clear Filters")) {
                    self.clear_filters();
                }
            });
        } else {
            self.table(ui, editor, &svc, &snap, &listing, body_h);
        }
        self.footer(ui, &listing);
        self.draw_rename(ui, editor, &svc);
    }

    /// Opens what keeps an object out of view: its folded class, its closed owners.
    fn reveal(&mut self, snap: &SceneState, id: ObjId) {
        let Some(o) = snap.get(id) else { return };
        if self.filter.hierarchy {
            let mut owner = o.owner;
            let mut guard = 0;
            while let Some(oid) = owner {
                guard += 1;
                if guard > 64 {
                    break;
                }
                let Some(g) = snap.get(oid) else { break };
                let key = (g.class, g.name.clone());
                if !self.folds.expanded.contains(&key) {
                    self.folds.expanded.push(key);
                }
                owner = g.owner;
            }
        } else {
            self.folds.collapsed.retain(|c| *c != o.class);
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, svc: &Arc<SceneService>, listing: Option<&Listing>) {
        let ctx = ui.ctx().clone();
        let any_hidden = listing.is_some_and(|l| l.groups.iter().any(|g| self.filter.class_hidden(g.class)));
        let active = self.filter.active_filters(any_hidden);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let filters_w = 92.0;
            let more_w = metrics::TOOLBAR_BUTTON;
            let w = (ui.available_width() - filters_w - more_w - 12.0).max(gz_ui::units::em(6.0));
            let search_id = "outliner_search";
            let resp =
                SearchField::new(&mut self.filter.text).hint("Search").width(w).id_salt(search_id).show(ui);
            if self.focus_search {
                resp.request_focus();
                self.focus_search = false;
            }
            // Filters: its menu opens under it, its badge counts what is on
            let filters = local_badge_button(
                ui,
                Icon::Filter,
                "Filters",
                "Selected only, visibility, sort and object types",
                active,
                active > 0,
            );
            let fid = Id::unique("gz-ui-level.outliner_filters");
            if filters.clicked() {
                egui::Popup::toggle_id(&ctx, fid);
            }
            if gz_ui::menu::take_popup_request(&ctx, "outliner_filters") {
                egui::Popup::open_id(&ctx, fid);
            }
            let layer = ui.layer_id();
            let groups: Vec<(ObjClass, usize)> =
                listing.map(|l| l.groups.iter().map(|g| (g.class, g.total)).collect()).unwrap_or_default();
            egui::Popup::new(fid, ctx.clone(), egui::PopupAnchor::ParentRect(filters.rect), layer)
                .kind(egui::PopupKind::Popup)
                .open_memory(None)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .gap(2.0)
                .frame(gz_ui::menu::popup_frame(&ctx))
                .show(|ui| {
                    ui.set_width(gz_ui::units::em(24.0));
                    self.filters_menu(ui, &groups);
                });
            let more =
                gz_ui::icon_button(ui, Icon::More, "Layout, and showing or hiding objects", false, true);
            let mid = Id::unique("gz-ui-level.outliner_more");
            if more.clicked() {
                egui::Popup::toggle_id(&ctx, mid);
            }
            if gz_ui::menu::take_popup_request(&ctx, "outliner_more") {
                egui::Popup::open_id(&ctx, mid);
            }
            let mut act: Option<(bool, bool)> = None;
            gz_ui::menu::menu_popup(&ctx, mid, more.rect, layer, |ui| {
                if menu_item(
                    ui,
                    "By Type",
                    MenuItem { selected: !self.filter.hierarchy, ..MenuItem::default() },
                ) {
                    self.filter.hierarchy = false;
                }
                let r = menu_item(
                    ui,
                    "By Hierarchy",
                    MenuItem { selected: self.filter.hierarchy, ..MenuItem::default() },
                );
                if r {
                    self.filter.hierarchy = true;
                }
                ui.separator();
                if menu_item(ui, "Show All", MenuItem::default()) {
                    act = Some((false, true));
                }
                if menu_item(ui, "Show Selected", MenuItem::default()) {
                    act = Some((true, true));
                }
                if menu_item(ui, "Hide Selected", MenuItem::default()) {
                    act = Some((true, false));
                }
            });
            if let Some((selected_only, visible)) = act {
                show_objects(svc, selected_only, visible, gz_base::Origin::Ui);
            }
        });
        ui.add_space(4.0);
    }

    fn filters_menu(&mut self, ui: &mut Ui, groups: &[(ObjClass, usize)]) {
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            gz_ui::grid::switch(ui, &mut self.filter.selected_only)
                .on_hover_text("Show only the objects selected in the viewport");
            text::label(ui, "Selected Only", Font::Body, p.text, Clip::End);
        });
        ui.add_space(4.0);
        gz_ui::PropertyGrid::new("outliner_filters").label_frac(0.3).show(ui, |g| {
            g.row(
                "Visibility",
                "Every object, only the visible ones or only the hidden ones",
                gz_ui::PropFlags::NONE,
                0,
                |ui| {
                    let mut cur = match self.filter.visibility {
                        Visibility::All => 0,
                        Visibility::Visible => 1,
                        Visibility::Hidden => 2,
                    };
                    if gz_ui::segmented(ui, &mut cur, &["All", "Visible", "Hidden"], None) {
                        self.filter.visibility = [Visibility::All, Visibility::Visible, Visibility::Hidden]
                            .get(cur)
                            .copied()
                            .unwrap_or_default();
                    }
                },
            );
            g.row(
                "Sort",
                "The order of the objects in each class: as in the scene, or by name",
                gz_ui::PropFlags::NONE,
                0,
                |ui| {
                    let mut cur = match self.filter.sort {
                        Sort::Scene => 0,
                        Sort::Az => 1,
                        Sort::Za => 2,
                    };
                    if gz_ui::segmented(ui, &mut cur, &["Scene", "A-Z", "Z-A"], None) {
                        self.filter.sort =
                            [Sort::Scene, Sort::Az, Sort::Za].get(cur).copied().unwrap_or_default();
                    }
                },
            );
        });
        ui.separator();
        ui.horizontal(|ui| {
            text::label(ui, "Object Types", Font::Body, p.text_secondary, Clip::End);
            if gz_ui::Button::new("All").show(ui).clicked() {
                self.filter.hidden.clear();
            }
            if gz_ui::Button::new("None").show(ui).clicked() {
                self.filter.hidden = groups.iter().map(|(c, _)| *c).collect();
            }
        });
        for (c, n) in groups {
            ui.horizontal(|ui| {
                let mut shown = !self.filter.class_hidden(*c);
                if gz_ui::grid::switch(ui, &mut shown).changed() {
                    if shown {
                        self.filter.hidden.retain(|h| h != c);
                    } else {
                        self.filter.hidden.push(*c);
                    }
                }
                text::label(ui, classes::title(*c), Font::Body, p.text, Clip::End);
                text::label(ui, &n.to_string(), Font::Body, p.text_muted, Clip::End);
            });
        }
        ui.separator();
        gz_ui::text::status(
            ui,
            Tone::Muted,
            "Search: every word has to match, -word leaves those out, \"two words\" match together.",
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn table(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &Arc<gz_store::Snap<SceneState>>,
        listing: &Arc<Listing>,
        body_h: f32,
    ) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        // the header: the eye, Name, and Type / Owner when the header's menu shows them
        let width = ui.available_width();
        let (head, head_resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
        let type_w = if self.show_type { gz_ui::units::em(7.0) } else { 0.0 };
        let owner_w = if self.show_owner { (width * 0.25).max(gz_ui::units::em(5.0)) } else { 0.0 };
        if ui.is_rect_visible(head) {
            let painter = ui.painter();
            painter.line_segment(
                [pos2(head.min.x, head.max.y - 0.5), pos2(head.max.x, head.max.y - 0.5)],
                egui::Stroke::new(1.0, p.border_subtle),
            );
            icons::draw(
                painter,
                Icon::Eye,
                Rect::from_center_size(pos2(head.min.x + ROW_H * 0.5, head.center().y), Vec2::splat(ICON)),
                p.text_muted,
            );
            let font = Font::Small.id();
            painter.text(
                pos2(head.min.x + ROW_H + 4.0, head.center().y),
                Align2::LEFT_CENTER,
                "Name",
                font.clone(),
                p.text_secondary,
            );
            if self.show_type {
                painter.text(
                    pos2(head.max.x - owner_w - type_w + 4.0, head.center().y),
                    Align2::LEFT_CENTER,
                    "Type",
                    font.clone(),
                    p.text_secondary,
                );
            }
            if self.show_owner {
                painter.text(
                    pos2(head.max.x - owner_w + 4.0, head.center().y),
                    Align2::LEFT_CENTER,
                    "Owner",
                    font,
                    p.text_secondary,
                );
            }
        }
        head_resp.context_menu(|ui| {
            gz_ui::menu::popup_body(ui, |ui| {
                if menu_item(ui, "Type", MenuItem { selected: self.show_type, ..MenuItem::default() }) {
                    self.show_type = !self.show_type;
                }
                if menu_item(ui, "Owner", MenuItem { selected: self.show_owner, ..MenuItem::default() }) {
                    self.show_owner = !self.show_owner;
                }
            });
        });

        let rows = &listing.rows;
        let n = rows.len();
        let view_h = (body_h - ROW_H).max(ROW_H);
        let mut scroll = ScrollArea::vertical()
            .id_salt("gz-ui-level.outliner_rows")
            .max_height(view_h)
            .auto_shrink([false, false]);
        // ui_scroll and a selection made elsewhere
        let mut target_row = self.scroll_target.take().and_then(|id| listing.row_of(id));
        match gz_app::take_scroll(&ctx, WINDOW) {
            Some(gz_app::ScrollTo::Top) => scroll = scroll.vertical_scroll_offset(0.0),
            Some(gz_app::ScrollTo::Bottom) => scroll = scroll.vertical_scroll_offset(n as f32 * ROW_H),
            Some(gz_app::ScrollTo::Offset(y)) => scroll = scroll.vertical_scroll_offset(y),
            Some(gz_app::ScrollTo::Item(name)) => {
                target_row = snap.find_relaxed(&name).and_then(|o| listing.row_of(o.id)).or(target_row)
            }
            None => {}
        }
        if let Some(r) = target_row {
            let top = r as f32 * ROW_H;
            let (y, h) = self.view;
            if top < y || top + ROW_H > y + h {
                scroll = scroll.vertical_scroll_offset((top + ROW_H * 0.5 - h * 0.5).max(0.0));
            }
        }
        let mut clicks: Vec<(usize, ObjId, egui::Modifiers, bool)> = Vec::new();
        let mut class_clicks: Vec<ObjClass> = Vec::new();
        let mut expand: Vec<(ObjId, bool)> = Vec::new();
        let mut menu: Option<RowMenu> = None;
        let out = scroll.show_rows(ui, ROW_H, n, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let Some(row) = rows.get(i) else { continue };
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
                match row {
                    Row::Class { class, shown, total, open } => {
                        if self.class_row(
                            ui,
                            svc,
                            snap,
                            rect,
                            *class,
                            *shown,
                            *total,
                            *open,
                            editor.ui_origin(),
                        ) {
                            class_clicks.push(*class);
                        }
                    }
                    Row::Object { id, class, depth, direct, children, expanded } => {
                        let Some(o) = snap.get(*id) else { continue };
                        let rid = Id::unique(("gz-ui-level.outliner_row", id.0));
                        let resp = ui.interact(rect, rid, Sense::click());
                        let selected = snap.is_selected(*id);
                        let painter = ui.painter();
                        if selected {
                            painter.rect_filled(rect, CornerRadius::same(3), p.selection_focused);
                        } else if resp.hovered() {
                            painter.rect_filled(rect, CornerRadius::same(3), p.hover);
                        }
                        // the eye column
                        let eye = Rect::from_min_size(rect.min, Vec2::splat(ROW_H));
                        let vis = o.flags.visible;
                        if local_eye_toggle(
                            ui,
                            rid.with("eye"),
                            eye,
                            vis,
                            if vis { "Hide in the viewport" } else { "Show in the viewport" },
                        ) {
                            svc.store
                                .post(SceneCmd::Show { ids: vec![*id], visible: !vis }, editor.ui_origin());
                        }
                        let tool_visible = snap.tools.part(*class).visible;
                        let mut x = eye.max.x + 4.0 + f32::from((*depth).min(12)) * INDENT;
                        if *children {
                            let er = Rect::from_min_size(pos2(x, rect.min.y), Vec2::new(16.0, ROW_H));
                            let filtering = self.filter.filtering() || !self.filter.hidden.is_empty();
                            let eresp = ui.interact(
                                er,
                                rid.with("expand"),
                                if filtering { Sense::hover() } else { Sense::click() },
                            );
                            icons::draw(
                                ui.painter(),
                                if *expanded { Icon::ChevronDown } else { Icon::ChevronRight },
                                Rect::from_center_size(er.center(), Vec2::splat(12.0)),
                                if filtering { p.text_disabled } else { p.text_secondary },
                            );
                            if eresp.clicked() {
                                expand.push((*id, !*expanded));
                            }
                        }
                        x += 16.0;
                        let dim = !tool_visible || !*direct;
                        icons::draw(
                            ui.painter(),
                            classes::icon(*class),
                            Rect::from_min_size(
                                pos2(x.round(), (rect.center().y - ICON * 0.5).round()),
                                Vec2::splat(ICON),
                            ),
                            if dim { p.text_disabled } else { p.text_secondary },
                        );
                        x += ICON + 6.0;
                        let right = rect.max.x - type_w - owner_w - 4.0;
                        let cut = text::paint(
                            ui.painter(),
                            &Font::Body.id(),
                            pos2(x, rect.center().y),
                            Align2::LEFT_CENTER,
                            if dim { p.text_secondary } else { p.text },
                            &o.name,
                            Some((right - x).max(0.0)),
                            Clip::End,
                        );
                        if cut {
                            audit::truncated(&ctx, &o.name, &o.name, rect);
                        }
                        if self.show_type {
                            text::paint(
                                ui.painter(),
                                &Font::Body.id(),
                                pos2(rect.max.x - owner_w - type_w + 4.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                p.text_secondary,
                                classes::title(*class),
                                Some(type_w - 8.0),
                                Clip::End,
                            );
                        }
                        if self.show_owner
                            && let Some(owner) = o.owner.and_then(|oid| snap.get(oid))
                        {
                            text::paint(
                                ui.painter(),
                                &Font::Body.id(),
                                pos2(rect.max.x - owner_w + 4.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                p.text_secondary,
                                &owner.name,
                                Some(owner_w - 8.0),
                                Clip::End,
                            );
                        }
                        let hidden_tip = !vis && self.filter.hierarchy;
                        let resp = if hidden_tip {
                            resp.on_hover_text("Hidden objects cannot be selected until shown.")
                        } else if !*direct {
                            resp.on_hover_text("Shown because an object under it matches.")
                        } else if cut {
                            resp.on_hover_text(o.name.as_str())
                        } else {
                            resp
                        };
                        if resp.clicked() && !hidden_tip {
                            clicks.push((i, *id, ctx.input(|inp| inp.modifiers), false));
                        }
                        if resp.double_clicked() {
                            clicks.push((i, *id, egui::Modifiers::NONE, true));
                        }
                        if !hidden_tip {
                            resp.context_menu(|ui| {
                                if let Some(m) = row_menu(ui, snap, o) {
                                    menu = Some(m);
                                }
                            });
                        }
                    }
                }
            }
        });
        self.view = (out.state.clamped_offset().y, out.inner_rect.height());
        let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
        gz_app::report_scroll(&ctx, WINDOW, out.state.clamped_offset().y, max);

        for c in class_clicks {
            if !self.filter.filtering() {
                if let Some(i) = self.folds.collapsed.iter().position(|x| *x == c) {
                    self.folds.collapsed.remove(i);
                } else {
                    self.folds.collapsed.push(c);
                }
            }
        }
        for (id, open) in expand {
            if let Some(o) = snap.get(id) {
                let key = (o.class, o.name.clone());
                self.folds.expanded.retain(|k| *k != key);
                if open {
                    self.folds.expanded.push(key);
                }
            }
        }
        for (row, id, mods, double) in clicks {
            self.click(editor, svc, snap, listing, row, id, mods, double);
        }
        if let Some(m) = menu {
            self.run_menu(ui, editor, svc, snap, m);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn class_row(
        &mut self,
        ui: &mut Ui,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        rect: Rect,
        class: ObjClass,
        shown: usize,
        total: usize,
        open: bool,
        origin: gz_base::Origin,
    ) -> bool {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let id = Id::unique(("gz-ui-level.outliner_class", class.id()));
        let resp = ui.interact(rect, id, Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(3), p.hover);
        }
        let visible = snap.tools.part(class).visible;
        let eye = Rect::from_min_size(rect.min, Vec2::splat(ROW_H));
        if local_eye_toggle(
            ui,
            id.with("eye"),
            eye,
            visible,
            if visible { "Hide this class in the viewport" } else { "Show this class in the viewport" },
        ) {
            svc.store.post(SceneCmd::ClassView { class, visible: Some(!visible), locked: None }, origin);
        }
        let cy = rect.center().y;
        let x0 = eye.max.x + 4.0;
        icons::draw(
            ui.painter(),
            if open { Icon::ChevronDown } else { Icon::ChevronRight },
            Rect::from_min_size(pos2(x0, cy - 6.0), Vec2::splat(12.0)),
            p.text_secondary,
        );
        let x = x0 + 16.0;
        icons::draw(
            ui.painter(),
            classes::icon(class),
            Rect::from_min_size(pos2(x, cy - ICON * 0.5), Vec2::splat(ICON)),
            if visible { p.text_secondary } else { p.text_disabled },
        );
        let tx = x + ICON + 6.0;
        let count = if self.filter.filtering() { format!("{shown} of {total}") } else { total.to_string() };
        let small = Font::Body.id();
        let count_w = text::width(&ctx, &small, &count);
        let title = classes::title(class);
        text::paint(
            ui.painter(),
            &Font::Strong.id(),
            pos2(tx, cy),
            Align2::LEFT_CENTER,
            if visible { p.text } else { p.text_secondary },
            title,
            Some((rect.max.x - tx - count_w - 10.0).max(0.0)),
            Clip::End,
        );
        let title_w =
            text::width(&ctx, &Font::Strong.id(), title).min((rect.max.x - tx - count_w - 10.0).max(0.0));
        ui.painter().text(pos2(tx + title_w + 6.0, cy), Align2::LEFT_CENTER, count, small, p.text_muted);
        audit::control(&ctx, title, true, None, None, rect);
        resp.clicked()
    }

    #[allow(clippy::too_many_arguments)]
    fn click(
        &mut self,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        listing: &Listing,
        row: usize,
        id: ObjId,
        mods: egui::Modifiers,
        double: bool,
    ) {
        let Some(o) = snap.get(id) else { return };
        let origin = editor.ui_origin();
        self.skip_scroll = true;
        if double {
            let mut args = Args::new();
            args.insert("name".into(), Value::String(o.name.to_string()));
            args.insert("select".into(), Value::Bool(true));
            editor.post("focus_object", args);
            self.anchor = Some(id);
            return;
        }
        if mods.shift
            && let Some(anchor) = self.anchor
            && let Some(a) = listing.row_of(anchor)
        {
            let (lo, hi) = (a.min(row), a.max(row));
            let ids: Vec<ObjId> = listing
                .rows
                .get(lo..=hi)
                .unwrap_or(&[])
                .iter()
                .filter_map(|r| match r {
                    Row::Object { id, .. } => Some(*id),
                    Row::Class { .. } => None,
                })
                .filter(|i| snap.get(*i).is_some_and(|x| x.flags.visible))
                .collect();
            let mode = if self.filter.hierarchy { SelectMode::Set } else { SelectMode::Add };
            svc.store.post(SceneCmd::Select { ids, mode }, origin);
            return;
        }
        if mods.ctrl || mods.command {
            svc.store.post(SceneCmd::Select { ids: vec![id], mode: SelectMode::Toggle }, origin);
            self.anchor = Some(id);
            return;
        }
        pick(svc, editor, snap, id);
        self.anchor = Some(id);
    }

    fn run_menu(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        m: RowMenu,
    ) {
        let origin = editor.ui_origin();
        match m {
            RowMenu::Focus(id) => {
                if let Some(o) = snap.get(id) {
                    let mut args = Args::new();
                    args.insert("name".into(), Value::String(o.name.to_string()));
                    args.insert("select".into(), Value::Bool(true));
                    editor.post("focus_object", args);
                }
            }
            RowMenu::SelectClass(c) => {
                self.skip_scroll = true;
                svc.store.post(SceneCmd::SelectAll { classes: None, select: false, invert: false }, origin);
                svc.store.post(
                    SceneCmd::SelectAll { classes: Some(vec![c]), select: true, invert: false },
                    origin,
                );
            }
            RowMenu::Toggle(id, visible) => svc.store.post(SceneCmd::Show { ids: vec![id], visible }, origin),
            RowMenu::Rename(id) => {
                if let Some(o) = snap.get(id) {
                    self.rename = Some(Rename {
                        id,
                        original: o.name.clone(),
                        buf: o.name.to_string(),
                        error: String::new(),
                        focus: true,
                    });
                }
            }
            RowMenu::Delete(id) => ask_delete(ui.ctx(), editor, svc, snap, id),
        }
    }

    fn footer(&self, ui: &mut Ui, listing: &Listing) {
        let p = palette(ui.ctx());
        let text_ = if listing.shown != listing.total {
            format!("{} of {} objects \u{b7} {} selected", listing.shown, listing.total, listing.selected)
        } else {
            format!(
                "{} object{} \u{b7} {} selected",
                listing.total,
                if listing.total == 1 { "" } else { "s" },
                listing.selected
            )
        };
        ui.add_space(2.0);
        text::label(ui, &text_, Font::Small, p.text_muted, Clip::End);
    }

    fn draw_rename(&mut self, ui: &mut Ui, editor: &Arc<Editor>, svc: &Arc<SceneService>) {
        let Some(r) = self.rename.as_mut() else { return };
        let mut open = true;
        let mut apply = false;
        let mut pressed = false;
        let mut cancel = false;
        let p = palette(ui.ctx());
        Dialog::new("Rename Object", "dlg.rename_object", DialogSize::S).show(
            ui,
            &mut open,
            |ui| {
                let w = ui.available_width();
                let id = Id::unique("gz-ui-level.rename_field");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut r.buf)
                        .id(id)
                        .font(Font::Body.id())
                        .margin(egui::Margin::symmetric(8, 4))
                        .desired_width(w)
                        .min_size(Vec2::new(w, metrics::FRAME)),
                );
                if r.focus {
                    resp.request_focus();
                    if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
                        let all = egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(r.buf.chars().count()),
                        );
                        st.cursor.set_char_range(Some(all));
                        st.store(ui.ctx(), id);
                    }
                    r.focus = false;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply = true;
                }
                ui.add_space(4.0);
                if r.error.is_empty() {
                    text::label(
                        ui,
                        "Names are lower case and unique in the scene.",
                        Font::Small,
                        p.text_muted,
                        Clip::End,
                    );
                } else {
                    gz_ui::text::status(ui, Tone::Error, &r.error);
                }
            },
            |f| {
                pressed = f.button("Rename", Kind::Primary, true, true, "");
                if f.button("Cancel", Kind::Secondary, false, true, "") {
                    cancel = true;
                }
            },
        );
        apply |= pressed;
        if cancel || !open {
            self.rename = None;
            return;
        }
        if !apply {
            return;
        }
        let snap = svc.store.snapshot();
        let low = r.buf.trim().to_lowercase();
        let Some(o) = snap.get(r.id).filter(|o| o.name == r.original) else {
            r.error = "The object no longer exists.".into();
            return;
        };
        if !snap.tools.part(o.class).editable() || !o.editable() {
            r.error = "The object is read only.".into();
            return;
        }
        if low.is_empty() {
            r.error = "Enter a name.".into();
            return;
        }
        if snap.find_any(&low).is_some_and(|x| x.id != o.id) {
            r.error = "Another object has this name.".into();
            return;
        }
        let key_before = (o.class, o.name.clone());
        if let Some(k) = self.folds.expanded.iter_mut().find(|k| **k == key_before) {
            k.1 = SmolStr::new(&low);
        }
        svc.store.post(SceneCmd::Rename { id: o.id, name: SmolStr::new(&low) }, editor.ui_origin());
        self.rename = None;
    }
}

/// What a row's menu asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowMenu {
    Focus(ObjId),
    SelectClass(ObjClass),
    Toggle(ObjId, bool),
    Rename(ObjId),
    Delete(ObjId),
}

fn row_menu(ui: &mut Ui, snap: &SceneState, o: &gz_scene::Object) -> Option<RowMenu> {
    let mut out = None;
    gz_ui::menu::popup_body(ui, |ui| {
        if menu_item(ui, "Focus", MenuItem { icon: Icon::FrameSelection, ..MenuItem::default() }) {
            out = Some(RowMenu::Focus(o.id));
        }
        if menu_item(ui, "Select All of This Class", MenuItem::default()) {
            out = Some(RowMenu::SelectClass(o.class));
        }
        let vis = o.flags.visible;
        if menu_item(
            ui,
            if vis { "Hide" } else { "Show" },
            MenuItem { icon: if vis { Icon::EyeOff } else { Icon::Eye }, ..MenuItem::default() },
        ) {
            out = Some(RowMenu::Toggle(o.id, !vis));
        }
        ui.separator();
        let can_edit = snap.tools.part(o.class).editable() && o.editable();
        if menu_item(
            ui,
            "Rename...",
            MenuItem { icon: Icon::Rename, enabled: can_edit, ..MenuItem::default() },
        ) {
            out = Some(RowMenu::Rename(o.id));
        }
        if menu_item(
            ui,
            "Delete...",
            MenuItem { icon: Icon::Delete, enabled: can_edit, danger: true, ..MenuItem::default() },
        ) {
            out = Some(RowMenu::Delete(o.id));
        }
    });
    out
}

/// `PickObject`: the object alone selected.
pub fn pick(svc: &SceneService, editor: &Editor, _snap: &SceneState, id: ObjId) {
    svc.store.post(SceneCmd::Select { ids: vec![id], mode: SelectMode::Set }, editor.ui_origin());
}

/// `AskDelete`: the kit's confirmation names the object; Yes deletes it (Undo brings it back).
fn ask_delete(
    ctx: &egui::Context,
    editor: &Arc<Editor>,
    svc: &Arc<SceneService>,
    snap: &SceneState,
    id: ObjId,
) {
    let Some(o) = snap.get(id) else { return };
    let spec = ConfirmSpec {
        id: "delete_object".into(),
        title: "Delete Object".into(),
        headline: "Delete this object from the scene?".into(),
        items: vec![o.name.to_string()],
        total: 1,
        consequence: "Edit > Undo brings it back.".into(),
        action: "Delete".into(),
        destructive: true,
    };
    let store = svc.store.clone();
    let origin = editor.ui_origin();
    let name = o.name.clone();
    let asker = if editor.flags.unattended { Asker::Unattended } else { Asker::Ui };
    let line = gz_ui::dialog::confirm(ctx, asker, spec, move |answer| {
        if answer != Answer::Yes {
            return;
        }
        // the object may have gone while the question was up
        let snap = store.snapshot();
        if let Some(o) = snap.find_any(&name)
            && snap.tools.part(o.class).editable()
            && o.editable()
        {
            store.post(SceneCmd::Delete(vec![o.id]), origin);
        }
    });
    if let Some(line) = line {
        editor.log.plain(line);
    }
}

/// `ShowObjects`: shows or hides every object (or the selected ones); showing everything shows
/// every class too. Answers how many objects changed.
pub fn plan_show(snap: &SceneState, selected_only: bool, visible: bool) -> (Vec<ObjId>, Vec<ObjClass>) {
    let mut ids = Vec::new();
    let mut classes = Vec::new();
    for c in ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool()) {
        if !selected_only && visible && !snap.tools.part(c).visible {
            classes.push(c);
        }
        for o in snap.of_class(c) {
            if (selected_only && !snap.is_selected(o.id)) || o.flags.visible == visible {
                continue;
            }
            ids.push(o.id);
        }
    }
    (ids, classes)
}

/// Sends what [`plan_show`] planned; answers how many objects changed.
pub fn show_objects(
    svc: &SceneService,
    selected_only: bool,
    visible: bool,
    origin: gz_base::Origin,
) -> usize {
    let snap = svc.store.snapshot();
    let (ids, classes) = plan_show(&snap, selected_only, visible);
    let n = ids.len();
    for c in classes {
        svc.store.post(SceneCmd::ClassView { class: c, visible: Some(true), locked: None }, origin);
    }
    if !ids.is_empty() {
        svc.store.post(SceneCmd::Show { ids, visible }, origin);
    }
    n
}
