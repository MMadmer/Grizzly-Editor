//! The World Outliner panel: the search (Ctrl+F focuses it), Filters with a badge (Selected
//! Only, visibility, sort, the kinds shown), More (By Type / By Hierarchy, Show All, Show or
//! Hide Selected), an eye and a lock per kind and per object, the names, the empty states, the
//! footer, the row menu with what the selection can do, Rename (`dlg.rename_object`) and
//! Delete (`dlg.delete_object`, the kit's confirmation naming what goes).
//!
//! The rows come from a `Query` keyed by the scene's revision and the filter
//! ([`crate::outliner_model::compute`]); only the rows that show are laid out. A selection made
//! elsewhere scrolls its row into view on the first listing of the scene as it is then
//! ([`crate::outliner_model::scroll_decision`]).

use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Rect, ScrollArea, Sense, Ui, Vec2, pos2};
use gz_base::SmolStr;
use gz_editor::{Editor, Value, json};
use gz_query::Query;
use gz_scene::apply::SelectMode;
use gz_scene::{ObjClass, ObjData, ObjId, Object, SceneCmd, SceneService, SceneState};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Check, ConfirmSpec, Dialog, DialogSize, Font, Icon, Kind, MenuItem, SearchField, TipExt, Tone, Tx, audit,
    icons, menu_item, metrics, palette, state, tr,
};

use crate::classes;
use crate::local::{self, args, local_badge_button, local_eye_toggle, local_lock_toggle, run};
use crate::outliner_model::{Filter, Folds, Listing, Row, ScrollDecision, Sort, Visibility, compute};

const ROW_H: f32 = metrics::ROW;
const ICON: f32 = 14.0;
const INDENT: f32 = 14.0;
/// The window the contract names, and `ui.scroll`'s id of the rows.
pub const WINDOW: &str = "World Outliner";

/// The rename dialog's state.
#[derive(Clone, Debug)]
struct Rename {
    id: ObjId,
    original: SmolStr,
    buf: String,
    /// Why the name is refused: the editor's own sentence, empty for none.
    error: &'static str,
    focus: bool,
}

/// The panel's view state; the scene is the store's.
pub struct Outliner {
    pub filter: Filter,
    pub folds: Folds,
    query: Query<(Filter, Folds), Listing>,
    /// The last listing drawn: what shows while the next one is computed.
    last: Option<Arc<Listing>>,
    pub focus_search: bool,
    /// The row a Shift+click extends from.
    anchor: Option<ObjId>,
    /// The panel's own clicks do not scroll: their row is under the pointer.
    skip_scroll: bool,
    sel_sig: u64,
    /// The selected object to bring into view, kept until a listing of the scene as it is places
    /// it.
    scroll_target: Option<ObjId>,
    /// The scene revision the last frame's listing was of, when that listing was current.
    drawn_rev: Option<gz_base::Rev>,
    /// A frame drew the panel since [`Self::settled`] last asked.
    drew: bool,
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
            last: None,
            focus_search: false,
            anchor: None,
            skip_scroll: false,
            sel_sig: 0,
            scroll_target: None,
            drawn_rev: None,
            drew: false,
            rename: None,
            show_type: false,
            show_owner: false,
            view: (0.0, 0.0),
        }
    }

    /// Clear Filters.
    pub fn clear_filters(&mut self) {
        self.filter.text.clear();
        self.filter.selected_only = false;
        self.filter.visibility = Visibility::All;
        self.filter.hidden.clear();
    }

    /// The listing to draw and whether it is of the snapshot there is: fresh, the last one of
    /// this filter while a new one comes, or the last one drawn while the filter's first comes.
    fn listing(&mut self, snap: &Arc<gz_store::Snap<SceneState>>) -> Option<(Arc<Listing>, bool)> {
        let key = (self.filter.clone(), self.folds.clone());
        let (s, f, d, rev) = (snap.clone(), self.filter.clone(), self.folds.clone(), snap.rev);
        let answer = self.query.get(&key, snap.rev, move || {
            let mut l = compute(&s, &f, &d);
            l.rev = rev;
            l
        });
        let fresh = answer.is_fresh();
        match answer.value() {
            Some(v) => {
                self.last = Some(v.clone());
                Some((v.clone(), fresh))
            }
            None => self.last.clone().map(|l| (l, false)),
        }
    }

    /// Whether the panel shows the scene of revision `rev` with nothing left to scroll to: what
    /// `level.view` waits for before it answers the panel as drawn. `None` when no frame drew the
    /// panel since the last ask - it is not on the screen, and nothing is to be waited for.
    pub fn settled(&mut self, rev: gz_base::Rev) -> Option<bool> {
        if !std::mem::take(&mut self.drew) {
            return None;
        }
        Some(self.drawn_rev == Some(rev) && self.scroll_target.is_none())
    }

    /// The last listing drawn, for `level.view`.
    #[must_use]
    pub fn last_listing(&self) -> Option<Arc<Listing>> {
        self.last.clone()
    }

    /// The panel's body.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>) {
        // the kinds stand in the order of the names the window shows
        let order = classes::listed_in(ui.ctx());
        if self.filter.order != order {
            self.filter.order = order;
        }
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
        self.drew = true;
        let Some((listing, fresh)) = self.listing(&snap) else {
            self.drawn_rev = None;
            self.toolbar(ui, editor, &svc, None);
            state::busy_line(ui, "Listing the scene...");
            return;
        };
        self.drawn_rev = fresh.then_some(snap.rev);

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

        self.toolbar(ui, editor, &svc, Some(&listing));
        // the footer, and the item spacing between the header, the rows and it
        let footer_h = Font::Small.line() + 16.0;
        let body_h = (ui.available_height() - footer_h).max(ROW_H * 2.0);
        if listing.total == 0 || listing.shown == 0 {
            ui.allocate_ui(Vec2::new(ui.available_width(), body_h), |ui| {
                if listing.total == 0 {
                    state::empty_state(
                        ui,
                        Icon::SceneObject,
                        "The scene is empty",
                        "Drag something in from the Place panel or the Content Browser.",
                        None,
                    );
                } else if state::empty_state(
                    ui,
                    Icon::Search,
                    "Nothing matches",
                    "No object passes the search and the filters.",
                    Some("Clear Filters".into()),
                ) {
                    self.clear_filters();
                }
            });
        } else {
            self.table(ui, editor, &svc, &snap, &listing, fresh, body_h);
        }
        self.footer(ui, &listing);
        self.draw_rename(ui, editor, &svc);
    }

    /// Opens what keeps an object out of view: its folded kind, its closed owners.
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

    fn toolbar(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        listing: Option<&Listing>,
    ) {
        let ctx = ui.ctx().clone();
        let any_hidden = listing.is_some_and(|l| l.groups.iter().any(|g| self.filter.class_hidden(g.class)));
        let active = self.filter.active_filters(any_hidden);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let filters_w = 92.0;
            let more_w = metrics::TOOLBAR_BUTTON;
            let w = (ui.available_width() - filters_w - more_w - 12.0).max(gz_ui::units::em(6.0));
            let resp = SearchField::new(&mut self.filter.text)
                .hint("Search")
                .width(w)
                .id_salt("outliner_search")
                .show(ui);
            if self.focus_search {
                resp.request_focus();
                self.focus_search = false;
            }
            // Filters: its menu opens under it, its badge counts what is on
            let filters = local_badge_button(
                ui,
                Icon::Filter,
                "Filters",
                "Selected only, visibility, sort and the kinds shown",
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
                    // Russian runs longer: the segments of the menu take more room
                    let em = if gz_ui::i18n::lang(&ctx) == gz_ui::Lang::En { 16.0 } else { 19.0 };
                    ui.set_width(gz_ui::units::em(em));
                    // the filters' words (All, Visible, Hidden, Scene) are those of a filter
                    gz_ui::i18n::scope(&ctx, "outliner.filters", || self.filters_menu(ui, &groups));
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
                if menu_item(
                    ui,
                    "By Hierarchy",
                    MenuItem { selected: self.filter.hierarchy, ..MenuItem::default() },
                ) {
                    self.filter.hierarchy = true;
                }
                ui.separator();
                if menu_item(ui, "Show All", MenuItem { icon: Icon::Eye, ..MenuItem::default() }) {
                    act = Some((false, true));
                }
                if menu_item(ui, "Show Selected", MenuItem::default()) {
                    act = Some((true, true));
                }
                if menu_item(ui, "Hide Selected", MenuItem { icon: Icon::EyeOff, ..MenuItem::default() }) {
                    act = Some((true, false));
                }
            });
            if let Some((selected_only, visible)) = act {
                show_objects(svc, selected_only, visible, editor.ui_origin());
            }
        });
        ui.add_space(4.0);
    }

    fn filters_menu(&mut self, ui: &mut Ui, groups: &[(ObjClass, usize)]) {
        let p = palette(ui.ctx());
        Check::new(&mut self.filter.selected_only, "Selected Only")
            .tip("Show only the selected objects")
            .show(ui);
        ui.add_space(4.0);
        text::label(ui, "Visibility", Font::Small, p.text_secondary, Clip::End);
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
        ui.add_space(4.0);
        text::label(ui, "Sort", Font::Small, p.text_secondary, Clip::End);
        let mut cur = match self.filter.sort {
            Sort::Scene => 0,
            Sort::Az => 1,
            Sort::Za => 2,
        };
        if gz_ui::segmented(ui, &mut cur, &["Scene", "A-Z", "Z-A"], None) {
            self.filter.sort = [Sort::Scene, Sort::Az, Sort::Za].get(cur).copied().unwrap_or_default();
        }
        ui.separator();
        ui.horizontal(|ui| {
            text::label(ui, "Kinds", Font::Small, p.text_secondary, Clip::End);
            if gz_ui::Button::new("All").kind(Kind::Ghost).show(ui).clicked() {
                self.filter.hidden.clear();
            }
            if gz_ui::Button::new("None").kind(Kind::Ghost).show(ui).clicked() {
                self.filter.hidden = groups.iter().map(|(c, _)| *c).collect();
            }
        });
        for (c, n) in groups {
            let mut shown = !self.filter.class_hidden(*c);
            let label = tr!("{kinds} ({n})", kinds = Tx::from(classes::plural(*c)), n = *n);
            if Check::new(&mut shown, label).show(ui).changed() {
                if shown {
                    self.filter.hidden.retain(|h| h != c);
                } else {
                    self.filter.hidden.push(*c);
                }
            }
        }
        ui.separator();
        text::status(
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
        fresh: bool,
        body_h: f32,
    ) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        // the header: the eye, the lock, Name, and Type / Owner when the header's menu shows them
        let width = ui.available_width();
        let (head, head_resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
        // the Type column: as wide as the longest kind's name where the window speaks another
        // language than the code
        let type_w = if !self.show_type {
            0.0
        } else if gz_ui::i18n::lang(&ctx) == gz_ui::Lang::En {
            gz_ui::units::em(7.0)
        } else {
            classes::listed()
                .into_iter()
                .map(|c| text::width(&ctx, &Font::Body.id(), classes::title(c)) + 8.0)
                .fold(gz_ui::units::em(7.0), f32::max)
                .min(width * 0.4)
        };
        let owner_w = if self.show_owner { (width * 0.25).max(gz_ui::units::em(5.0)) } else { 0.0 };
        if ui.is_rect_visible(head) {
            let painter = ui.painter();
            painter.line_segment(
                [pos2(head.min.x, head.max.y - 0.5), pos2(head.max.x, head.max.y - 0.5)],
                egui::Stroke::new(1.0, p.border_subtle),
            );
            for (k, icon) in [Icon::Eye, Icon::Lock].into_iter().enumerate() {
                let c = pos2(head.min.x + ROW_H * (k as f32 + 0.5), head.center().y);
                icons::draw(painter, icon, Rect::from_center_size(c, Vec2::splat(ICON)), p.text_muted);
            }
            let font = Font::Small.id();
            let [head_name, head_type, head_owner] =
                ["Name", "Type", "Owner"].map(|w| gz_ui::i18n::t(&ctx, w));
            painter.text(
                pos2(head.min.x + ROW_H * 2.0 + 4.0, head.center().y),
                Align2::LEFT_CENTER,
                head_name,
                font.clone(),
                p.text_secondary,
            );
            if self.show_type {
                painter.text(
                    pos2(head.max.x - owner_w - type_w + 4.0, head.center().y),
                    Align2::LEFT_CENTER,
                    head_type,
                    font.clone(),
                    p.text_secondary,
                );
            }
            if self.show_owner {
                painter.text(
                    pos2(head.max.x - owner_w + 4.0, head.center().y),
                    Align2::LEFT_CENTER,
                    head_owner,
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
        // a selection made elsewhere: placed only by a listing of the scene as it is now
        let mut target_row = None;
        if let Some(id) = self.scroll_target {
            match crate::outliner_model::scroll_decision(id, listing, snap.rev, fresh) {
                ScrollDecision::Wait => {}
                ScrollDecision::Scroll(r) => {
                    target_row = Some(r);
                    self.scroll_target = None;
                }
                ScrollDecision::Drop => self.scroll_target = None,
            }
        }
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
        let mut clicks: Vec<Click> = Vec::new();
        let mut class_clicks: Vec<ObjClass> = Vec::new();
        let mut expand: Vec<(ObjId, bool)> = Vec::new();
        let mut menu: Option<RowMenu> = None;
        let origin = editor.ui_origin();
        let out = scroll.show_rows(ui, ROW_H, n, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let Some(row) = rows.get(i) else { continue };
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
                match row {
                    Row::Class { class, shown, total, open } => {
                        if self.class_row(ui, svc, snap, rect, *class, *shown, *total, *open, origin) {
                            class_clicks.push(*class);
                        }
                    }
                    Row::Point { way, index, depth } => {
                        if let Some(c) = point_row(ui, snap, rect, *way, *index, *depth) {
                            clicks.push(c);
                        }
                    }
                    Row::Object { id, class, depth, direct, children, expanded } => {
                        let Some(o) = snap.get(*id) else { continue };
                        let rid = Id::unique(("gz-ui-level.outliner_row", id.0));
                        let resp = ui.interact(rect, rid, Sense::click());
                        let selected = snap.is_selected(*id);
                        let hovered = resp.hovered() || ui.rect_contains_pointer(rect);
                        let painter = ui.painter();
                        if selected {
                            painter.rect_filled(rect, CornerRadius::same(3), p.selection_focused);
                        } else if hovered {
                            painter.rect_filled(rect, CornerRadius::same(3), p.hover);
                        }
                        // the eye and the lock columns
                        let eye = Rect::from_min_size(rect.min, Vec2::splat(ROW_H));
                        let vis = o.flags.visible;
                        if local_eye_toggle(
                            ui,
                            rid.with("eye"),
                            eye,
                            vis,
                            if vis { "Hide in the viewport" } else { "Show in the viewport" },
                        ) {
                            svc.store.post(SceneCmd::Show { ids: vec![*id], visible: !vis }, origin);
                        }
                        let lock = Rect::from_min_size(pos2(eye.max.x, rect.min.y), Vec2::splat(ROW_H));
                        let locked = o.flags.locked;
                        if local_lock_toggle(
                            ui,
                            rid.with("lock"),
                            lock,
                            locked,
                            hovered,
                            if locked {
                                "Unlock: it can be picked and changed in the viewport again"
                            } else {
                                "Lock: it stays drawn, but cannot be picked or changed in the viewport"
                            },
                        ) {
                            svc.store.post(SceneCmd::Lock { ids: vec![*id], locked: !locked }, origin);
                        }
                        let kind_visible = snap.tools.part(*class).visible;
                        let mut x = lock.max.x + 4.0 + f32::from((*depth).min(12)) * INDENT;
                        if *children {
                            let er = Rect::from_min_size(pos2(x, rect.min.y), Vec2::new(16.0, ROW_H));
                            let ways = matches!(o.data, ObjData::Way(_));
                            let filtering =
                                !ways && (self.filter.filtering() || !self.filter.hidden.is_empty());
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
                        let dim = !kind_visible || !*direct || !vis;
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
                        let shown_name = leaf(&o.name);
                        let cut = text::paint(
                            ui.painter(),
                            &Font::Body.id(),
                            pos2(x, rect.center().y),
                            Align2::LEFT_CENTER,
                            if dim { p.text_secondary } else { p.text },
                            Tx::raw(shown_name),
                            Some((right - x).max(0.0)),
                            Clip::Middle,
                        );
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
                                Tx::raw(leaf(&owner.name)),
                                Some(owner_w - 8.0),
                                Clip::Middle,
                            );
                        }
                        let hidden_tip = !vis && self.filter.hierarchy;
                        let resp = if hidden_tip {
                            resp.tip("Hidden objects cannot be selected until shown.")
                        } else if !*direct {
                            resp.tip("Shown because an object under it matches.")
                        } else if cut || shown_name.len() != o.name.len() {
                            resp.tip(Tx::raw(o.name.as_str()))
                        } else {
                            resp
                        };
                        if resp.clicked() && !hidden_tip {
                            clicks.push(Click::Object(i, *id, ctx.input(|inp| inp.modifiers), false));
                        }
                        if resp.double_clicked() {
                            clicks.push(Click::Object(i, *id, egui::Modifiers::NONE, true));
                        }
                        if resp.secondary_clicked() && !selected {
                            // the row's menu acts on the selection: the row becomes it first
                            svc.store
                                .post(SceneCmd::Select { ids: vec![*id], mode: SelectMode::Set }, origin);
                            self.skip_scroll = true;
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
        for c in clicks {
            self.click(editor, svc, snap, listing, c);
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
        let hovered = resp.hovered() || ui.rect_contains_pointer(rect);
        if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(3), p.hover);
        }
        let part = snap.tools.part(class);
        let (visible, locked) = (part.visible, part.locked);
        let title = classes::plural(class);
        let eye = Rect::from_min_size(rect.min, Vec2::splat(ROW_H));
        if local_eye_toggle(
            ui,
            id.with("eye"),
            eye,
            visible,
            if visible {
                tr!("Hide every one of the {title}", title = Tx::from(title))
            } else {
                tr!("Show the {title} again", title = Tx::from(title))
            },
        ) {
            svc.store.post(SceneCmd::ClassView { class, visible: Some(!visible), locked: None }, origin);
        }
        let lock = Rect::from_min_size(pos2(eye.max.x, rect.min.y), Vec2::splat(ROW_H));
        if local_lock_toggle(
            ui,
            id.with("lock"),
            lock,
            locked,
            hovered,
            if locked {
                tr!("Unlock the {title}: they can be picked and changed again", title = Tx::from(title))
            } else {
                tr!(
                    "Lock the {title}: drawn, but never picked or changed in the viewport",
                    title = Tx::from(title)
                )
            },
        ) {
            svc.store.post(SceneCmd::ClassView { class, visible: None, locked: Some(!locked) }, origin);
        }
        let cy = rect.center().y;
        let x0 = lock.max.x + 4.0;
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
        let count = if self.filter.filtering() {
            tr!("{a} of {b}", a = shown, b = total).text(&ctx)
        } else {
            total.to_string()
        };
        let small = Font::Body.id();
        let count_w = text::width(&ctx, &small, Tx::raw(&count));
        let room = (rect.max.x - tx - count_w - 10.0).max(0.0);
        text::paint(
            ui.painter(),
            &Font::Strong.id(),
            pos2(tx, cy),
            Align2::LEFT_CENTER,
            if visible { p.text } else { p.text_secondary },
            title,
            Some(room),
            Clip::End,
        );
        let title_w = text::width(&ctx, &Font::Strong.id(), title).min(room);
        ui.painter().text(pos2(tx + title_w + 6.0, cy), Align2::LEFT_CENTER, count, small, p.text_muted);
        audit::control(&ctx, title, true, None, None, rect);
        resp.clicked()
    }

    fn click(
        &mut self,
        editor: &Arc<Editor>,
        svc: &Arc<SceneService>,
        snap: &SceneState,
        listing: &Listing,
        c: Click,
    ) {
        let origin = editor.ui_origin();
        self.skip_scroll = true;
        let (row, id, mods, double) = match c {
            Click::Point { way, index, toggle } => {
                let Some(w) = snap.get(way) else { return };
                if !snap.is_selected(way) {
                    svc.store.post(SceneCmd::Select { ids: vec![way], mode: SelectMode::Set }, origin);
                }
                run(
                    editor,
                    "scene.way",
                    args(&[
                        ("action", json!("select_points")),
                        ("way", json!(w.name.as_str())),
                        ("points", json!([index.to_string()])),
                        ("mode", json!(if toggle { "toggle" } else { "set" })),
                    ]),
                );
                return;
            }
            Click::Object(row, id, mods, double) => (row, id, mods, double),
        };
        let Some(o) = snap.get(id) else { return };
        if double {
            focus(editor, &o.name);
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
                    _ => None,
                })
                .filter(|i| snap.get(*i).is_some_and(|x| x.flags.visible))
                .collect();
            svc.store.post(SceneCmd::Select { ids, mode: SelectMode::Set }, origin);
            return;
        }
        if mods.ctrl || mods.command {
            svc.store.post(SceneCmd::Select { ids: vec![id], mode: SelectMode::Toggle }, origin);
            self.anchor = Some(id);
            return;
        }
        svc.store.post(SceneCmd::Select { ids: vec![id], mode: SelectMode::Set }, origin);
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
                    focus(editor, &o.name);
                }
            }
            RowMenu::SelectKind(c) => {
                self.skip_scroll = true;
                svc.store.post(
                    SceneCmd::SelectAll { classes: Some(vec![c]), select: true, invert: false },
                    origin,
                );
            }
            RowMenu::SelectSame(id) => {
                self.skip_scroll = true;
                let ids = same_reference(snap, id);
                svc.store.post(SceneCmd::Select { ids, mode: SelectMode::Set }, origin);
            }
            RowMenu::Show(ids, visible) => svc.store.post(SceneCmd::Show { ids, visible }, origin),
            RowMenu::Lock(ids, locked) => svc.store.post(SceneCmd::Lock { ids, locked }, origin),
            RowMenu::Rename(id) => {
                if let Some(o) = snap.get(id) {
                    self.rename = Some(Rename {
                        id,
                        original: o.name.clone(),
                        buf: o.name.to_string(),
                        error: "",
                        focus: true,
                    });
                }
            }
            RowMenu::Delete(ids) => ask_delete(ui.ctx(), editor, svc, snap, &ids),
            RowMenu::Command(cmd, pairs) => run(editor, cmd, args(&pairs)),
            RowMenu::Ask(spec, cmd, pairs) => {
                let ed = editor.clone();
                local::confirm(ui.ctx(), editor, *spec, move || run(&ed, cmd, args(&pairs)));
            }
            RowMenu::PivotChooser => svc.view.lock().tool_chooser = Some("group_pivot".into()),
            RowMenu::SaveGroupAs => crate::tools::save_group_as(editor, ui.ctx(), svc, snap),
        }
    }

    fn footer(&self, ui: &mut Ui, listing: &Listing) {
        let p = palette(ui.ctx());
        let line = if listing.shown != listing.total {
            tr!(
                "{shown} of {total} {total:object|objects} \u{b7} {selected} selected",
                shown = listing.shown,
                total = listing.total,
                selected = listing.selected
            )
        } else {
            tr!(
                "{n} {n:object|objects} \u{b7} {selected} selected",
                n = listing.total,
                selected = listing.selected
            )
        };
        ui.add_space(2.0);
        text::label(ui, line, Font::Small, p.text_muted, Clip::End);
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
                    text::label(ui, "A name is unique in the scene.", Font::Small, p.text_muted, Clip::End);
                } else {
                    text::status(ui, Tone::Error, r.error);
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
        let name = r.buf.trim().to_owned();
        let Some(o) = snap.get(r.id).filter(|o| o.name == r.original) else {
            r.error = "The object no longer exists.";
            return;
        };
        if !snap.tools.part(o.class).editable() || !o.editable() {
            r.error = "The object is locked or part of a group.";
            return;
        }
        if name.is_empty() {
            r.error = "Enter a name.";
            return;
        }
        if snap.find_any(&name).is_some_and(|x| x.id != o.id) {
            r.error = "Another object has this name.";
            return;
        }
        let key_before = (o.class, o.name.clone());
        if let Some(k) = self.folds.expanded.iter_mut().find(|k| **k == key_before) {
            k.1 = SmolStr::new(&name);
        }
        svc.store.post(SceneCmd::Rename { id: o.id, name: SmolStr::new(&name) }, editor.ui_origin());
        self.rename = None;
    }

    /// Unfolds (or folds) a kind's row By Type, or an object's - a group's, a way path's - By
    /// Hierarchy, by the kind's key or the object's name. Answers what it did not know.
    pub fn set_unfolded(&mut self, snap: &SceneState, names: &[String], open: bool) -> Vec<String> {
        let mut unknown = Vec::new();
        for n in names {
            if let Some(c) = ObjClass::by_key(n) {
                self.folds.collapsed.retain(|x| *x != c);
                if !open {
                    self.folds.collapsed.push(c);
                }
            } else if let Some(o) = snap.find_relaxed(n) {
                let key = (o.class, o.name.clone());
                self.folds.expanded.retain(|k| *k != key);
                if open {
                    self.folds.expanded.push(key);
                }
            } else {
                unknown.push(n.clone());
            }
        }
        unknown
    }

    /// What `level.view` answers of the Outliner.
    #[must_use]
    pub fn view_state(&self) -> Value {
        let l = self.last.as_deref();
        let hidden: Vec<&str> = self.filter.hidden.iter().map(|c| c.key()).collect();
        json!({
            "search": self.filter.text,
            "selected_only": self.filter.selected_only,
            "visibility": self.filter.visibility.key(),
            "sort": self.filter.sort.key(),
            "layout": if self.filter.hierarchy { "hierarchy" } else { "type" },
            "hidden_kinds": hidden,
            "kinds": l.map_or_else(Vec::new, |l| l.groups.iter().map(|g| json!({
                "kind": g.class.key(),
                "name": classes::plural(g.class),
                "total": g.total,
                "shown": g.shown,
            })).collect()),
            "rows": l.map_or(0, |l| l.rows.len()),
            "shown": l.map_or(0, |l| l.shown),
            "total": l.map_or(0, |l| l.total),
            "selected": l.map_or(0, |l| l.selected),
            "scroll_pending": self.scroll_target.is_some(),
            "scroll_y": self.view.0,
            "view_h": self.view.1,
            "row_h": ROW_H,
        })
    }

    /// The first rows of a listing as they are drawn (`level.view`): kind rows by their name,
    /// objects by theirs, points as `<way>/<point>`.
    #[must_use]
    pub fn rows_of(&self, snap: &SceneState, l: &Listing, limit: usize) -> Vec<Value> {
        l.rows
            .iter()
            .take(limit)
            .map(|r| match r {
                Row::Class { class, total, open, .. } => json!({
                    "kind": class.key(),
                    "name": classes::plural(*class),
                    "count": total,
                    "open": open,
                    "visible": snap.tools.part(*class).visible,
                    "locked": snap.tools.part(*class).locked,
                }),
                Row::Object { id, depth, .. } => {
                    let o = snap.get(*id);
                    json!({
                        "object": o.map_or("", |o| o.name.as_str()),
                        "depth": depth,
                        "visible": o.is_some_and(|o| o.flags.visible),
                        "locked": o.is_some_and(|o| o.flags.locked),
                        "selected": snap.is_selected(*id),
                    })
                }
                Row::Point { way, index, depth } => {
                    let name = snap.get(*way).map_or(String::new(), |w| match &w.data {
                        ObjData::Way(d) => d
                            .points
                            .get(*index as usize)
                            .map_or(String::new(), |p| format!("{}/{}", w.name, p.name)),
                        _ => String::new(),
                    });
                    json!({ "point": name, "depth": depth })
                }
            })
            .collect()
    }
}

/// A click the table saw this frame.
enum Click {
    /// Its row, the object, the keys held, a double click.
    Object(usize, ObjId, egui::Modifiers, bool),
    Point {
        way: ObjId,
        index: u32,
        toggle: bool,
    },
}

/// A point of an open way path: its glyph and name, selected while the way and the point are.
fn point_row(
    ui: &mut Ui,
    snap: &SceneState,
    rect: Rect,
    way: ObjId,
    index: u32,
    depth: u16,
) -> Option<Click> {
    let w = snap.get(way)?;
    let ObjData::Way(d) = &w.data else { return None };
    let pt = d.points.get(index as usize)?;
    let p = palette(ui.ctx());
    let rid = Id::unique(("gz-ui-level.outliner_point", way.0, index));
    let resp = ui.interact(rect, rid, Sense::click());
    let selected = snap.is_selected(way) && pt.selected;
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(3), p.selection_focused);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), p.hover);
    }
    let x = rect.min.x + ROW_H * 2.0 + 4.0 + f32::from(depth.min(12)) * INDENT + 16.0;
    icons::draw(
        ui.painter(),
        Icon::Dot,
        Rect::from_min_size(pos2(x, rect.center().y - ICON * 0.5), Vec2::splat(ICON)),
        p.text_muted,
    );
    let links = pt.links.len();
    let label = if links == 0 { pt.name.to_string() } else { format!("{}  \u{2192} {links}", pt.name) };
    text::paint(
        ui.painter(),
        &Font::Body.id(),
        pos2(x + ICON + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        p.text_secondary,
        Tx::raw(&label),
        Some((rect.max.x - x - ICON - 10.0).max(0.0)),
        Clip::Middle,
    );
    let resp = resp.tip(tr!("A point of {way}: {n} {n:link|links}", way = w.name.as_str(), n = links));
    resp.clicked().then(|| Click::Point { way, index, toggle: ui.input(|i| i.modifiers.command) })
}

/// The part of a name after its last folder, as the row shows it.
#[must_use]
pub fn leaf(name: &str) -> &str {
    name.rsplit(['\\', '/']).next().filter(|l| !l.is_empty()).unwrap_or(name)
}

/// Frames an object in the viewport and selects it.
fn focus(editor: &Arc<Editor>, name: &str) {
    run(
        editor,
        "viewport.camera",
        args(&[("action", json!("focus")), ("object", json!(name)), ("select", json!(true))]),
    );
}

/// The objects made from the same thing as `id` (the library object, the section, the effect,
/// the group asset), of its kind.
fn same_reference(snap: &SceneState, id: ObjId) -> Vec<ObjId> {
    let Some(o) = snap.get(id) else { return Vec::new() };
    let Some(r) = o.ref_name().map(str::to_lowercase) else { return vec![id] };
    snap.of_class(o.class)
        .filter(|x| snap.shown(x) && x.ref_name().is_some_and(|n| n.to_lowercase() == r))
        .map(|x| x.id)
        .collect()
}

/// What a row's menu asked for.
enum RowMenu {
    Focus(ObjId),
    SelectKind(ObjClass),
    SelectSame(ObjId),
    Show(Vec<ObjId>, bool),
    Lock(Vec<ObjId>, bool),
    Rename(ObjId),
    Delete(Vec<ObjId>),
    /// A command over the selection, with these arguments.
    Command(&'static str, Vec<(&'static str, Value)>),
    /// The same, after the kit's question.
    Ask(Box<ConfirmSpec>, &'static str, Vec<(&'static str, Value)>),
    PivotChooser,
    SaveGroupAs,
}

/// The objects a row's menu acts on: the selection when the row is in it, else the row.
fn targets(snap: &SceneState, o: &Object) -> Vec<ObjId> {
    if snap.is_selected(o.id) { snap.selected_in_order() } else { vec![o.id] }
}

/// The row's menu: the object, then what the selection's kinds can do.
fn row_menu(ui: &mut Ui, snap: &SceneState, o: &Object) -> Option<RowMenu> {
    let mut out = None;
    let ids = targets(snap, o);
    let objects: Vec<&Arc<Object>> = ids.iter().filter_map(|id| snap.get(*id)).collect();
    let count = |c: ObjClass| objects.iter().filter(|x| x.class == c).count();
    let n = objects.len();
    let can_edit = snap.tools.part(o.class).editable() && o.editable() && !o.flags.locked;
    gz_ui::menu::popup_body(ui, |ui| {
        let mut item = |ui: &mut Ui, label: Tx<'static>, m: MenuItem, act: RowMenu| {
            if menu_item(ui, label, m) {
                out = Some(act);
            }
        };
        item(
            ui,
            "Focus".into(),
            MenuItem { icon: Icon::FrameSelection, ..MenuItem::default() },
            RowMenu::Focus(o.id),
        );
        if o.ref_name().is_some() {
            item(ui, "Select All Made From the Same".into(), MenuItem::default(), RowMenu::SelectSame(o.id));
        }
        item(
            ui,
            tr!("Select All {kinds}", kinds = Tx::from(classes::plural(o.class))),
            MenuItem::default(),
            RowMenu::SelectKind(o.class),
        );
        let all_shown = objects.iter().all(|x| x.flags.visible);
        let all_locked = objects.iter().all(|x| x.flags.locked);
        item(
            ui,
            if all_shown { "Hide" } else { "Show" }.into(),
            MenuItem { icon: if all_shown { Icon::EyeOff } else { Icon::Eye }, ..MenuItem::default() },
            RowMenu::Show(ids.clone(), !all_shown),
        );
        item(
            ui,
            if all_locked { "Unlock" } else { "Lock" }.into(),
            MenuItem { icon: if all_locked { Icon::Unlock } else { Icon::Lock }, ..MenuItem::default() },
            RowMenu::Lock(ids.clone(), !all_locked),
        );

        // what the selection's kinds can do
        let mut kind_rows: Vec<(Tx<'static>, RowMenu)> = Vec::new();
        if n >= 2 {
            kind_rows.push((
                Tx::ctx("action", "Group"),
                RowMenu::Command("scene.group", vec![("action", json!("group"))]),
            ));
        }
        if count(ObjClass::Group) > 0 {
            kind_rows.push(("Ungroup".into(), RowMenu::Command("scene.ungroup", Vec::new())));
            kind_rows.push(("Save Group As...".into(), RowMenu::SaveGroupAs));
            kind_rows.push((
                "Center the Pivot".into(),
                RowMenu::Command("scene.group", vec![("action", json!("center_pivot"))]),
            ));
            kind_rows.push(("Pivot to an Object...".into(), RowMenu::PivotChooser));
            kind_rows.push((
                "Reload From Its Asset".into(),
                RowMenu::Command("scene.group", vec![("action", json!("reload"))]),
            ));
        }
        if count(ObjClass::Way) == 1 && n == 1 {
            for (label, action) in [
                ("Link Selected Points One Way", "link_one_way"),
                ("Link Selected Points Both Ways", "link_both_ways"),
                ("Turn the Links Round", "invert_links"),
                ("Remove the Selected Points' Links", "unlink"),
            ] {
                kind_rows
                    .push((label.into(), RowMenu::Command("scene.way", vec![("action", json!(action))])));
            }
        }
        if count(ObjClass::Way) > 0 {
            kind_rows.push((
                "Make Every Link One Way".into(),
                RowMenu::Command("scene.way", vec![("action", json!("make_one_way"))]),
            ));
            kind_rows.push((
                "Make Every Link Both Ways".into(),
                RowMenu::Command("scene.way", vec![("action", json!("make_both_ways"))]),
            ));
        }
        let shapes = count(ObjClass::Shape);
        if shapes >= 2 {
            kind_rows.push((
                "Join Shapes".into(),
                RowMenu::Command("scene.shape", vec![("action", json!("join"))]),
            ));
        }
        if shapes > 0 {
            let names: Vec<String> =
                objects.iter().filter(|x| x.class == ObjClass::Shape).map(|x| x.name.to_string()).collect();
            kind_rows.push((
                "Split Into Parts...".into(),
                RowMenu::Ask(
                    Box::new(ConfirmSpec {
                        id: "split_shapes".into(),
                        title: "Split Shapes".into(),
                        headline: "Split these shapes into their parts?".into(),
                        total: names.len(),
                        items: names.into_iter().take(10).map(Tx::raw).collect(),
                        consequence:
                            "Each part becomes a shape of its own. Edit > Undo puts them back together."
                                .into(),
                        action: "Split".into(),
                        destructive: false,
                    }),
                    "scene.shape",
                    vec![("action", json!("split"))],
                ),
            ));
        }
        if count(ObjClass::FogVolume) >= 2 {
            kind_rows.push(("Group Fog Volumes".into(), RowMenu::Command("scene.fog_group", Vec::new())));
        }
        if count(ObjClass::FogVolume) > 0 {
            kind_rows.push((
                "Ungroup Fog Volumes".into(),
                RowMenu::Command("scene.fog_group", vec![("ungroup", json!(true))]),
            ));
        }
        let spawns: Vec<&&Arc<Object>> = objects.iter().filter(|x| x.class == ObjClass::SpawnPoint).collect();
        if let ([spawn], 1) = (spawns.as_slice(), shapes)
            && n == 2
            && let Some(shape) = objects.iter().find(|x| x.class == ObjClass::Shape)
        {
            kind_rows.push((
                "Give the Shape to the Spawn Point".into(),
                RowMenu::Command(
                    "scene.attach",
                    vec![("spawn", json!(spawn.name.as_str())), ("object", json!(shape.name.as_str()))],
                ),
            ));
        }
        let holders: Vec<String> = spawns
            .iter()
            .filter(|x| matches!(&x.data, ObjData::Spawn(s) if s.attached.is_some()))
            .map(|x| x.name.to_string())
            .collect();
        if !holders.is_empty() {
            kind_rows.push((
                "Detach Their Shapes...".into(),
                RowMenu::Ask(
                    Box::new(ConfirmSpec {
                        id: "detach_spawn_objects".into(),
                        title: "Detach Shapes".into(),
                        headline: "Give the shapes of these spawn points back to the scene?".into(),
                        total: holders.len(),
                        items: holders.into_iter().take(10).map(Tx::raw).collect(),
                        consequence: "Edit > Undo attaches them again.".into(),
                        action: "Detach".into(),
                        destructive: false,
                    }),
                    "scene.attach",
                    vec![("detach", json!(true))],
                ),
            ));
        }
        if count(ObjClass::SceneObject) > 0 {
            kind_rows.push((
                "Reset Surfaces".into(),
                RowMenu::Command("scene.set", vec![("set", json!({ "surfaces.reset": "Reset" }))]),
            ));
        }
        if !kind_rows.is_empty() {
            ui.separator();
            for (label, act) in kind_rows {
                item(ui, label, MenuItem::default(), act);
            }
        }

        ui.separator();
        item(
            ui,
            "Rename...".into(),
            MenuItem { icon: Icon::Rename, enabled: can_edit && n == 1, ..MenuItem::default() },
            RowMenu::Rename(o.id),
        );
        item(
            ui,
            "Delete...".into(),
            MenuItem { icon: Icon::Delete, enabled: can_edit, danger: true, ..MenuItem::default() },
            RowMenu::Delete(ids.clone()),
        );
    });
    out
}

/// The kit's confirmation names the objects; Yes deletes the ones that may be (Undo brings them
/// back).
fn ask_delete(
    ctx: &egui::Context,
    editor: &Arc<Editor>,
    svc: &Arc<SceneService>,
    snap: &SceneState,
    ids: &[ObjId],
) {
    let names: Vec<SmolStr> = ids.iter().filter_map(|id| snap.get(*id)).map(|o| o.name.clone()).collect();
    if names.is_empty() {
        return;
    }
    let one = names.len() == 1;
    let spec = ConfirmSpec {
        id: "delete_object".into(),
        title: if one { "Delete Object" } else { "Delete Objects" }.into(),
        headline: if one {
            "Delete this object from the scene?"
        } else {
            "Delete these objects from the scene?"
        }
        .into(),
        total: names.len(),
        items: names.iter().take(10).map(|n| Tx::raw(n.to_string())).collect(),
        consequence: "Edit > Undo brings them back.".into(),
        action: "Delete".into(),
        destructive: true,
    };
    let store = svc.store.clone();
    let origin = editor.ui_origin();
    local::confirm(ctx, editor, spec, move || {
        // the objects may have gone while the question was up
        let snap = store.snapshot();
        let ids: Vec<ObjId> = names
            .iter()
            .filter_map(|n| snap.find_any(n))
            .filter(|o| snap.tools.part(o.class).editable() && o.editable() && !o.flags.locked)
            .map(|o| o.id)
            .collect();
        if !ids.is_empty() {
            store.post(SceneCmd::Delete(ids), origin);
        }
    });
}

/// Show All / Show Selected / Hide Selected: the objects whose eye changes, and the kinds whose
/// eye opens with Show All.
#[must_use]
pub fn plan_show(snap: &SceneState, selected_only: bool, visible: bool) -> (Vec<ObjId>, Vec<ObjClass>) {
    let mut ids = Vec::new();
    let mut kinds = Vec::new();
    for c in ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool()) {
        if !selected_only && visible && !snap.tools.part(c).visible {
            kinds.push(c);
        }
        for o in snap.of_class(c) {
            if (selected_only && !snap.is_selected(o.id)) || o.flags.visible == visible {
                continue;
            }
            ids.push(o.id);
        }
    }
    (ids, kinds)
}

/// Sends what [`plan_show`] planned; answers how many objects changed.
pub fn show_objects(
    svc: &SceneService,
    selected_only: bool,
    visible: bool,
    origin: gz_base::Origin,
) -> usize {
    let snap = svc.store.snapshot();
    let (ids, kinds) = plan_show(&snap, selected_only, visible);
    let n = ids.len();
    for c in kinds {
        svc.store.post(SceneCmd::ClassView { class: c, visible: Some(true), locked: None }, origin);
    }
    if !ids.is_empty() {
        svc.store.post(SceneCmd::Show { ids, visible }, origin);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_shows_the_leaf_of_a_path_name() {
        assert_eq!(leaf("statics\\shkaf\\cabinet_0001"), "cabinet_0001");
        assert_eq!(leaf("light_01"), "light_01");
        assert_eq!(leaf("a\\"), "a\\");
    }

    #[test]
    fn show_all_opens_the_kinds_too() {
        let mut s = SceneState::default();
        s.insert(Object::new(ObjClass::Light, "l1", ObjData::Light(gz_scene::objects::LightData::default())));
        s.tools.part_mut(ObjClass::Light).visible = false;
        let (ids, kinds) = plan_show(&s, false, true);
        assert!(ids.is_empty());
        assert_eq!(kinds, vec![ObjClass::Light]);
    }
}
