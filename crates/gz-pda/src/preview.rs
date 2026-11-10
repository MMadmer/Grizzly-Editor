//! The preview's running interface as data (`EditorPdaPreview.cpp`, `SPreview`): which page
//! shows, what actions changed, the log of what ran - and how a click, a key or an action
//! changes it, the way the game's `CUIPdaLayout` plays them. Everything here is in canvas
//! units; the tab only scales it and draws, `pda_preview` only forwards.
//!
//! What cannot run outside the game is said instead: `signal:` and `lua:` are a toast and a
//! line of the log, `close` a toast.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::layout::{Item, Layout, Page};

/// Lines the log keeps.
const LOG_LINES: usize = 60;

/// The device's shape: the canvas is laid over the screen of `pda.xml` or `pda_16.xml`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aspect {
    #[default]
    Standard,
    Wide,
}

impl Aspect {
    /// `16:9` / `wide` is wide, anything else 4:3.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        if text == "16:9" || text == "wide" { Self::Wide } else { Self::Standard }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "4:3",
            Self::Wide => "16:9",
        }
    }
}

/// The local time the preview fills `{time}` and `{date}` from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    pub hour: u32,
    pub minute: u32,
    pub day: u32,
    pub month: u32,
}

/// Where an item is, in canvas units, and whether it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ItemBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub shown: bool,
}

impl ItemBox {
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// What lies under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A clickable item (its index).
    Item(usize),
    /// A coloured panel that takes the click.
    Panel,
    Map,
    Nothing,
}

/// An item on the screen as the state answer lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleItem {
    /// 1-based index of the item.
    pub n: usize,
    pub kind: String,
    pub id: String,
    /// A row's number and whether it is the list's selection.
    pub row: Option<(u32, bool)>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The text with its placeholders filled in.
    pub text: Option<String>,
    pub action: Vec<String>,
    /// A tab button's selection.
    pub selected: Option<bool>,
}

/// The preview's state over one layout.
#[derive(Clone, Debug)]
pub struct Preview {
    pub layout: Arc<Layout>,
    pub page: String,
    pub prev_page: String,
    /// The stock section on show, empty when none.
    pub stock: String,
    pub closed: bool,
    /// Item index -> shown, once an action changed it.
    pub vis: BTreeMap<usize, bool>,
    /// Vars set by actions.
    pub vars: BTreeMap<String, String>,
    /// List index -> selected row.
    pub row_sel: BTreeMap<usize, u32>,
    /// List index -> scroll in canvas units.
    pub scroll: BTreeMap<usize, f32>,
    pub map_zoom: f32,
    /// The map was dragged off its centre.
    pub map_center_set: bool,
    /// What ran, oldest first.
    pub log: Vec<String>,
    /// The last toast and a counter that moves with every toast (the tab times them).
    pub toast: String,
    pub toast_serial: u64,
    pub aspect: Aspect,
    pub lang: String,
}

/// A builtin placeholder and the preview's stand-in for it.
const BUILTINS: [(&str, &str); 8] = [
    ("time", ""),
    ("date", ""),
    ("actor", "Stalker"),
    ("money", "12500"),
    ("rank", "Experienced"),
    ("community", "Loners"),
    ("level", ""),
    ("task", "The current task"),
];

impl Preview {
    #[must_use]
    pub fn new(layout: Arc<Layout>) -> Self {
        let mut p = Self {
            layout,
            page: String::new(),
            prev_page: String::new(),
            stock: String::new(),
            closed: false,
            vis: BTreeMap::new(),
            vars: BTreeMap::new(),
            row_sel: BTreeMap::new(),
            scroll: BTreeMap::new(),
            map_zoom: 1.0,
            map_center_set: false,
            log: Vec::new(),
            toast: String::new(),
            toast_serial: 0,
            aspect: Aspect::Standard,
            lang: "rus".to_owned(),
        };
        p.reset();
        p
    }

    fn item(&self, i: usize) -> Option<&Item> {
        self.layout.items.get(i)
    }

    /// Back to the start: the start page, nothing changed, the map close up on the player.
    pub fn reset(&mut self) {
        self.page.clone_from(&self.layout.start);
        self.prev_page.clear();
        self.stock.clear();
        self.closed = false;
        self.vis.clear();
        self.vars.clear();
        self.row_sel.clear();
        self.scroll.clear();
        // the game centres on the player close up (ViewActor); zoom = "global" shows it all
        self.map_zoom = if self.layout.items.iter().any(|it| it.kind == "map" && it.zoom == "global") { 1.0 } else { 3.0 };
        self.map_center_set = false;
    }

    /// A new reading of the file. With `keep_state` the page stays while the layout still has
    /// it (an edit should not throw the author back), and the state kept by item index goes.
    pub fn set_layout(&mut self, layout: Arc<Layout>, keep_state: bool) {
        let page = self.page.clone();
        self.layout = layout;
        if !keep_state || self.layout.find_page(&page).is_none() {
            self.reset();
        } else {
            // the item list may have changed shape: index-keyed state goes
            self.vis.clear();
            self.row_sel.clear();
            self.scroll.clear();
        }
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > LOG_LINES {
            self.log.remove(0);
        }
    }

    fn toast(&mut self, text: String) {
        self.toast.clone_from(&text);
        self.toast_serial += 1;
        self.push_log(text);
    }

    // ---- placeholders --------------------------------------------------------------------

    /// A var: set by an action, from `preview.vars`, or declared by the layout.
    #[must_use]
    pub fn var_of(&self, name: &str) -> Option<String> {
        if let Some(v) = self.vars.get(name) {
            return Some(v.clone());
        }
        let l = &self.layout;
        l.preview_vars.iter().find(|(k, _)| k == name).or_else(|| l.vars.iter().find(|(k, _)| k == name)).map(|(_, v)| v.clone())
    }

    /// A text with its `{placeholders}` filled in: the game's own from `preview.vars` or a
    /// stand-in, the rest from the vars; an unknown one stays as written.
    #[must_use]
    pub fn format(&self, raw: &str, now: &Clock) -> String {
        if !raw.contains('{') {
            return raw.to_owned();
        }
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(open) = rest.find('{') {
            let (before, from) = rest.split_at(open);
            out.push_str(before);
            let Some(close) = from.find('}') else {
                out.push_str(from);
                rest = "";
                break;
            };
            let name = from.get(1..close).unwrap_or("");
            match self.placeholder(name, now) {
                Some(v) => out.push_str(&v),
                None => out.push_str(from.get(..=close).unwrap_or(from)),
            }
            rest = from.get(close + 1..).unwrap_or("");
        }
        out.push_str(rest);
        out
    }

    fn placeholder(&self, name: &str, now: &Clock) -> Option<String> {
        let l = &self.layout;
        if let Some((_, stand_in)) = BUILTINS.iter().find(|(b, _)| *b == name) {
            // the last preview var of that name wins, as the C++ loop let it
            if let Some((_, v)) = l.preview_vars.iter().rev().find(|(k, _)| k == name) {
                return Some(v.clone());
            }
            return Some(match name {
                "time" => format!("{:02}:{:02}", now.hour, now.minute),
                "date" => format!("{:02}.{:02}.2012", now.day, now.month),
                "level" => {
                    if l.preview_level.is_empty() {
                        "Cordon".to_owned()
                    } else {
                        l.preview_level.clone()
                    }
                }
                _ => (*stand_in).to_owned(),
            });
        }
        match name {
            "title" => Some(l.title.clone()),
            "page" => Some(l.find_page(&self.page).map(|p| p.title.clone()).unwrap_or_default()),
            _ => self.var_of(name),
        }
    }

    // ---- geometry ------------------------------------------------------------------------

    #[must_use]
    pub fn item_visible(&self, i: usize) -> bool {
        self.vis.get(&i).copied().unwrap_or_else(|| self.item(i).is_some_and(|it| it.visible))
    }

    /// The rows of the list at `list`.
    #[must_use]
    pub fn rows_of(&self, list: usize) -> usize {
        self.layout.items.iter().filter(|it| it.parent == list + 1 && it.kind == "row").count()
    }

    /// Every item's box in canvas units and whether it shows (`ComputeLayout`).
    #[must_use]
    pub fn boxes(&self) -> Vec<ItemBox> {
        let l = &self.layout;
        let stock = !self.stock.is_empty();
        let mut out: Vec<ItemBox> = Vec::with_capacity(l.items.len());
        for (i, it) in l.items.iter().enumerate() {
            let mut shown = self.item_visible(i) && !self.closed;
            let (mut bx, mut by) = (0.0, 0.0);
            let parent = it.parent.checked_sub(1).and_then(|p| out.get(p).copied());
            if let Some(p) = parent {
                (bx, by) = (p.x0, p.y0);
                shown = shown && p.shown;
            } else if it.layer == "page" {
                shown = shown && it.page == self.page && !stock;
            } else if it.layer == "chrome" {
                shown = shown && !stock;
            }
            let list = it.parent.checked_sub(1).and_then(|p| l.items.get(p));
            let b = match list {
                Some(list) if it.kind == "row" => {
                    // rows stack in their list, scrolled
                    let off = self.scroll.get(&(it.parent - 1)).copied().unwrap_or(0.0);
                    let y = (it.row as f32 - 1.0) * (list.row_h + list.gap) - off;
                    ItemBox { x0: bx, y0: by + y, x1: bx + list.w, y1: by + y + it.h, shown }
                }
                _ => ItemBox { x0: bx + it.x, y0: by + it.y, x1: bx + it.x + it.w, y1: by + it.y + it.h, shown },
            };
            out.push(b);
        }
        out
    }

    /// The list a row (or anything under one) sits in.
    fn list_of(&self, i: usize) -> Option<usize> {
        let mut p = self.item(i)?.parent;
        // a parent always comes before its child: the walk ends
        while p > 0 {
            let it = self.item(p - 1)?;
            if it.kind == "list" {
                return Some(p - 1);
            }
            if it.parent >= p {
                return None;
            }
            p = it.parent;
        }
        None
    }

    #[must_use]
    pub fn clickable(it: &Item) -> bool {
        it.kind == "row" || (it.kind == "button" && !it.action.is_empty())
    }

    /// The topmost clickable item under a canvas point; a coloured panel above it stops the
    /// search (`HitTest`).
    #[must_use]
    pub fn hit_test(&self, boxes: &[ItemBox], x: f32, y: f32) -> Hit {
        for (i, a) in boxes.iter().enumerate().rev() {
            if !a.shown || !a.contains(x, y) {
                continue;
            }
            if let Some(list) = self.list_of(i)
                && !boxes.get(list).is_some_and(|b| b.contains(x, y))
            {
                continue;
            }
            let Some(it) = self.item(i) else { continue };
            if it.kind == "row" && y > a.y0 + it.h {
                continue;
            }
            if Self::clickable(it) {
                return Hit::Item(i);
            }
            if (it.kind == "rect" || it.kind == "group") && (it.color >> 24) > 0 {
                return Hit::Panel;
            }
            if it.kind == "map" {
                return Hit::Map;
            }
        }
        Hit::Nothing
    }

    /// A row is the list's selection; a button is selected while one of its pages shows.
    #[must_use]
    pub fn selected(&self, i: usize) -> bool {
        let Some(it) = self.item(i) else { return false };
        if it.kind == "row" {
            return it.parent.checked_sub(1).and_then(|l| self.row_sel.get(&l)).is_some_and(|r| *r == it.row);
        }
        it.selected_on.contains(&self.page)
    }

    // ---- actions -------------------------------------------------------------------------

    fn find_page(&self, id: &str) -> Option<&Page> {
        self.layout.find_page(id)
    }

    /// Opens a page; nothing for a page the layout does not have.
    pub fn set_page(&mut self, id: &str) {
        let Some(stock) = self.find_page(id).map(|p| p.stock.clone()) else { return };
        if self.page != id {
            self.prev_page = std::mem::replace(&mut self.page, id.to_owned());
        }
        self.stock = stock;
    }

    fn stock_action(&mut self, section: &str) {
        if let Some(id) = self.layout.pages.iter().find(|p| p.stock == section).map(|p| p.id.clone()) {
            self.set_page(&id);
            return;
        }
        if !self.page.is_empty() {
            self.prev_page.clone_from(&self.page);
        }
        self.stock = section.to_owned();
    }

    /// Runs one action the way the game does; what only the game can do is said instead.
    pub fn run_action(&mut self, action: &str, now: &Clock) {
        self.push_log(action);
        let (verb, arg) = action.split_once(':').unwrap_or((action, ""));
        match verb {
            "page" => self.set_page(arg),
            "back" => {
                if !self.stock.is_empty() && (self.prev_page.is_empty() || self.prev_page == self.page) {
                    if self.find_page(&self.page).is_none_or(|p| p.stock.is_empty()) {
                        self.stock.clear();
                    }
                } else if !self.prev_page.is_empty() {
                    let back = self.prev_page.clone();
                    self.set_page(&back);
                }
            }
            "close" => {
                self.closed = true;
                self.toast("The PDA closes".to_owned());
            }
            "show" | "hide" | "toggle" => {
                if let Some(i) = self.layout.find_id(arg) {
                    let on = match verb {
                        "show" => true,
                        "hide" => false,
                        _ => !self.item_visible(i),
                    };
                    self.vis.insert(i, on);
                }
            }
            "stock" => self.stock_action(arg),
            "map" => {
                let map_page = self.layout.items.iter().find(|it| it.kind == "map" && it.page != self.page && !it.page.is_empty()).map(|it| it.page.clone());
                if let Some(p) = map_page {
                    self.set_page(&p);
                }
                match arg {
                    "zoom_in" => self.map_zoom = (self.map_zoom * 1.5).min(12.0),
                    "zoom_out" => self.map_zoom = (self.map_zoom / 1.5).max(1.0),
                    "global" => {
                        self.map_zoom = 1.0;
                        self.map_center_set = false;
                    }
                    "actor" => {
                        self.map_zoom = self.map_zoom.max(3.0);
                        self.map_center_set = false;
                    }
                    _ => {}
                }
            }
            "var" => {
                if let Some((name, value)) = arg.split_once('=') {
                    let v = self.format(value, now);
                    self.vars.insert(name.to_owned(), v);
                }
            }
            "signal" => self.toast(format!("Quests receive signal \"{arg}\"")),
            "lua" => self.toast(format!("The game calls {arg}()")),
            _ => {}
        }
    }

    /// A click on item `i`: a row becomes its list's selection, then the actions run.
    pub fn click(&mut self, i: usize, now: &Clock) {
        let Some(it) = self.item(i) else { return };
        let actions = it.action.clone();
        if it.kind == "row"
            && let Some(list) = it.parent.checked_sub(1)
        {
            let row = it.row;
            self.row_sel.insert(list, row);
        }
        for a in &actions {
            self.run_action(a, now);
        }
    }

    /// The game's map, tasks or contacts key while the layout is up.
    pub fn open_key(&mut self, key: &str, now: &Clock) {
        let l = &self.layout;
        let action = match key {
            "map" => l.key_map.clone(),
            "contacts" => l.key_contacts.clone(),
            _ => l.key_tasks.clone(),
        };
        self.closed = false;
        if !action.is_empty() {
            self.run_action(&action, now);
        }
    }

    /// Opens a page by its id (`pda_preview action=page`).
    pub fn go_to_page(&mut self, id: &str) -> Result<(), String> {
        if self.find_page(id).is_none() {
            return Err(format!("there is no page '{id}'"));
        }
        self.closed = false;
        self.set_page(id);
        Ok(())
    }

    /// The item a click names by its id - with `row`, a list's row by its number, like
    /// `xms.pda.click(id, row)` in the game.
    pub fn target_by_id(&self, boxes: &[ItemBox], id: &str, row: u32) -> Result<usize, String> {
        let Some(target) = self.layout.find_id(id) else { return Err(format!("no item has the id '{id}'")) };
        if !boxes.get(target).is_some_and(|b| b.shown) {
            return Err(format!("'{id}' is not on the screen"));
        }
        if row == 0 {
            return Ok(target);
        }
        self.layout
            .items
            .iter()
            .enumerate()
            .rev()
            .find(|(_, it)| it.kind == "row" && it.parent == target + 1 && it.row == row)
            .map(|(i, _)| i)
            .ok_or_else(|| format!("'{id}' has no row {row}"))
    }

    /// The item a click names by the `n` of the state's `visible` list.
    pub fn target_by_n(boxes: &[ItemBox], n: usize) -> Result<usize, String> {
        let i = n.checked_sub(1).ok_or_else(|| "that item is not on the screen".to_owned())?;
        if boxes.get(i).is_some_and(|b| b.shown) { Ok(i) } else { Err("that item is not on the screen".to_owned()) }
    }

    /// The item under a canvas point.
    pub fn target_at(&self, boxes: &[ItemBox], x: f32, y: f32) -> Result<usize, String> {
        match self.hit_test(boxes, x + 0.01, y + 0.01) {
            Hit::Item(i) => Ok(i),
            _ => Err(format!("nothing clickable at {},{}", x.round(), y.round())),
        }
    }

    /// The wheel over a point: a list scrolls, the map zooms. `false`: neither took it.
    pub fn wheel(&mut self, boxes: &[ItemBox], x: f32, y: f32, delta: f32) -> bool {
        if self.closed {
            return false;
        }
        for (i, a) in boxes.iter().enumerate().rev() {
            if !a.shown || !a.contains(x, y) {
                continue;
            }
            let Some(it) = self.item(i) else { continue };
            if it.kind == "list" {
                let content = self.rows_of(i) as f32 * (it.row_h + it.gap);
                let max = (content - it.h).max(0.0);
                let sc = self.scroll.entry(i).or_insert(0.0);
                *sc = (*sc - delta * 60.0).clamp(0.0, max);
                return true;
            }
            if it.kind == "map" {
                self.map_zoom = (self.map_zoom * if delta > 0.0 { 1.25 } else { 0.8 }).clamp(1.0, 12.0);
                return true;
            }
        }
        false
    }

    /// The vars as the state answer lists them: the layout's in its order with their current
    /// values, then those only actions set.
    #[must_use]
    pub fn vars_view(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.layout.vars.iter().map(|(k, _)| (k.clone(), self.var_of(k).unwrap_or_default())).collect();
        for (k, v) in &self.vars {
            if !self.layout.vars.iter().any(|(d, _)| d == k) {
                out.push((k.clone(), v.clone()));
            }
        }
        out
    }

    /// What is on the screen that an agent can name or click, in canvas units.
    #[must_use]
    pub fn visible(&self, boxes: &[ItemBox], now: &Clock) -> Vec<VisibleItem> {
        let mut out = Vec::new();
        for (i, a) in boxes.iter().enumerate() {
            let Some(it) = self.item(i) else { continue };
            if !a.shown {
                continue;
            }
            let interesting = !it.id.is_empty() || Self::clickable(it) || it.has_text || it.kind == "map" || it.kind == "list";
            if !interesting {
                continue;
            }
            out.push(VisibleItem {
                n: i + 1,
                kind: it.kind.clone(),
                id: it.id.clone(),
                row: (it.kind == "row").then(|| (it.row, self.selected(i))),
                x: a.x0,
                y: a.y0,
                w: a.x1 - a.x0,
                h: a.y1 - a.y0,
                text: it.has_text.then(|| self.format(&it.text, now)),
                action: it.action.clone(),
                selected: (it.kind == "button" && !it.selected_on.is_empty()).then(|| self.selected(i)),
            });
        }
        out
    }
}
