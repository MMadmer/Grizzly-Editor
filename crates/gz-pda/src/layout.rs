//! The flat layout the game's reader produces, read off its answer.

use gz_luadata::{Table, Value};

/// A coded problem of the reader: `E3nn` stops a build, `W3nn` does not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdaProblem {
    pub code: String,
    /// Where in the file: `pages[2].widgets[1]` (empty: the layout).
    pub path: String,
    pub message: String,
}

impl PdaProblem {
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.code.starts_with('E')
    }
}

/// One item of the flat list: the fields the game's layout drawing reads, in canvas units,
/// colours as ARGB.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// `rect image text button list group map row`
    pub kind: String,
    /// `chrome page overlay`
    pub layer: String,
    pub page: String,
    /// 1-based index of the parent item; 0 = the layer.
    pub parent: usize,
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub visible: bool,

    pub color: u32,
    pub hover: u32,
    pub press: u32,
    pub selected: u32,
    pub border: u32,
    pub tint: u32,
    pub has_hover: bool,
    pub has_press: bool,
    pub has_selected: bool,
    pub has_border: bool,
    pub radius: f32,
    pub border_w: f32,

    pub texture: String,
    pub stretch: bool,

    pub text: String,
    pub font: String,
    pub hint: String,
    pub has_text: bool,
    pub text_color: u32,
    pub text_hover: u32,
    pub text_selected: u32,
    pub has_text_hover: bool,
    pub has_text_selected: bool,
    /// `l c r`
    pub align: char,
    /// `t c b`
    pub valign: char,
    pub wrap: bool,

    pub icon: String,
    pub icon_size: f32,
    pub icon_color: u32,
    pub icon_selected: u32,
    pub has_icon_selected: bool,

    pub action: Vec<String>,
    pub selected_on: Vec<String>,

    pub row_h: f32,
    pub gap: f32,
    pub empty: String,
    /// A list row's number, from 1.
    pub row: u32,
    pub zoom: String,
}

impl Default for Item {
    fn default() -> Self {
        Self {
            kind: String::new(),
            layer: String::new(),
            page: String::new(),
            parent: 0,
            id: String::new(),
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            visible: true,
            color: 0,
            hover: 0,
            press: 0,
            selected: 0,
            border: 0,
            tint: 0xFFFF_FFFF,
            has_hover: false,
            has_press: false,
            has_selected: false,
            has_border: false,
            radius: 0.0,
            border_w: 2.0,
            texture: String::new(),
            stretch: true,
            text: String::new(),
            font: String::new(),
            hint: String::new(),
            has_text: false,
            text_color: 0xFFFF_FFFF,
            text_hover: 0,
            text_selected: 0,
            has_text_hover: false,
            has_text_selected: false,
            align: 'l',
            valign: 'c',
            wrap: false,
            icon: String::new(),
            icon_size: 24.0,
            icon_color: 0xFFFF_FFFF,
            icon_selected: 0,
            has_icon_selected: false,
            action: Vec::new(),
            selected_on: Vec::new(),
            row_h: 0.0,
            gap: 0.0,
            empty: String::new(),
            row: 0,
            zoom: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub id: String,
    pub title: String,
    /// The stock PDA section the page shows, empty for a page of widgets.
    pub stock: String,
}

/// A spot of `preview.markers`.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub x: f32,
    pub z: f32,
    pub icon: String,
    pub hint: String,
    /// A `map_spots` node whose look it takes.
    pub spot: String,
    pub color: u32,
}

/// `map_spots = { ... }`: the look of one stock map spot node while the module is on.
#[derive(Clone, Debug, PartialEq)]
pub struct MapSpot {
    pub name: String,
    pub icon: String,
    pub arrows: String,
    pub color: u32,
    pub size: f32,
    pub heading: bool,
    pub level: i32,
    /// `icon` is a texture file and `rect` the piece of it, in its pixels.
    pub rect: Option<[f32; 4]>,
    pub inner: Option<InnerSpot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InnerSpot {
    pub icon: String,
    pub color: u32,
    pub size: f32,
    pub x: f32,
    pub y: f32,
}

/// What the reader made of a layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// No error.
    pub ok: bool,
    pub errors: u32,
    pub warnings: u32,
    pub id: String,
    pub title: String,
    pub start: String,
    pub canvas_w: f32,
    pub canvas_h: f32,
    pub noise: bool,
    pub close: bool,
    pub key_map: String,
    pub key_tasks: String,
    pub key_contacts: String,
    /// `vars`, sorted.
    pub vars: Vec<(String, String)>,
    pub pages: Vec<Page>,
    pub items: Vec<Item>,
    pub problems: Vec<PdaProblem>,
    pub map_spots: Vec<MapSpot>,

    /// `preview = { ... }`: the editor's fake world, never read by the game.
    pub preview_level: String,
    pub actor: Option<(f32, f32)>,
    pub preview_vars: Vec<(String, String)>,
    pub markers: Vec<Marker>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            ok: false,
            errors: 0,
            warnings: 0,
            id: String::new(),
            title: String::new(),
            start: String::new(),
            canvas_w: 960.0,
            canvas_h: 680.0,
            noise: true,
            close: true,
            key_map: String::new(),
            key_tasks: String::new(),
            key_contacts: String::new(),
            vars: Vec::new(),
            pages: Vec::new(),
            items: Vec::new(),
            problems: Vec::new(),
            map_spots: Vec::new(),
            preview_level: String::new(),
            actor: None,
            preview_vars: Vec::new(),
            markers: Vec::new(),
        }
    }
}

impl Layout {
    #[must_use]
    pub fn find_page(&self, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    #[must_use]
    pub fn find_spot(&self, name: &str) -> Option<&MapSpot> {
        self.map_spots.iter().find(|s| s.name == name)
    }

    /// The index of the item with this id.
    #[must_use]
    pub fn find_id(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|it| it.id == id)
    }
}

// ---- reading the reader's answer from the Lua stack --------------------------

fn field<'a>(t: &'a Table, k: &str) -> Option<&'a Value> {
    t.fields.iter().find(|(name, _)| name == k).map(|(_, v)| v)
}

fn get_str(t: &Table, k: &str) -> String {
    match field(t, k) {
        Some(Value::String(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn has_str(t: &Table, k: &str) -> bool {
    matches!(field(t, k), Some(Value::String(_)))
}

fn get_num(t: &Table, k: &str, def: f64) -> f64 {
    match field(t, k) {
        Some(Value::Number(n)) => *n,
        _ => def,
    }
}

fn get_f32(t: &Table, k: &str, def: f64) -> f32 {
    get_num(t, k, def) as f32
}

/// `(int)` of a double: toward zero, saturated.
fn get_int(t: &Table, k: &str, def: i64) -> i64 {
    get_num(t, k, def as f64) as i64
}

fn get_bool(t: &Table, k: &str, def: bool) -> bool {
    match field(t, k) {
        Some(Value::Bool(b)) => *b,
        _ => def,
    }
}

/// A colour number: saturated into `u32`, the fraction cut. `None` when it is no number.
fn get_color(t: &Table, k: &str) -> Option<u32> {
    match field(t, k) {
        Some(Value::Number(d)) => Some(if *d <= 0.0 {
            0
        } else if *d >= 4_294_967_295.0 {
            u32::MAX
        } else {
            *d as u32
        }),
        _ => None,
    }
}

fn get_list(t: &Table, k: &str) -> Vec<String> {
    match field(t, k) {
        Some(Value::Table(list)) => list.arr.iter().map_while(|v| v.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

fn sub<'a>(t: &'a Table, k: &str) -> Option<&'a Table> {
    match field(t, k) {
        Some(Value::Table(s)) => Some(s),
        _ => None,
    }
}

/// The tables of an array, up to the first entry that is no table.
fn tables<'a>(t: &'a Table, k: &str) -> impl Iterator<Item = &'a Table> {
    sub(t, k).into_iter().flat_map(|list| list.arr.iter().map_while(Value::as_table))
}

fn read_item(t: &Table) -> Item {
    let mut it = Item {
        kind: get_str(t, "kind"),
        layer: get_str(t, "layer"),
        page: get_str(t, "page"),
        parent: usize::try_from(get_int(t, "parent", 0)).unwrap_or(0),
        id: get_str(t, "id"),
        x: get_f32(t, "x", 0.0),
        y: get_f32(t, "y", 0.0),
        w: get_f32(t, "w", 0.0),
        h: get_f32(t, "h", 0.0),
        visible: get_bool(t, "visible", true),
        ..Item::default()
    };
    if let Some(c) = get_color(t, "color") {
        it.color = c;
    }
    (it.has_hover, it.hover) = get_color(t, "hover").map_or((false, 0), |c| (true, c));
    (it.has_press, it.press) = get_color(t, "press").map_or((false, 0), |c| (true, c));
    (it.has_selected, it.selected) = get_color(t, "selected").map_or((false, 0), |c| (true, c));
    (it.has_border, it.border) = get_color(t, "border").map_or((false, 0), |c| (true, c));
    if let Some(c) = get_color(t, "tint") {
        it.tint = c;
    }
    it.radius = get_f32(t, "radius", 0.0);
    it.border_w = get_f32(t, "border_w", 2.0);
    it.texture = get_str(t, "texture");
    it.stretch = get_bool(t, "stretch", true);
    it.has_text = has_str(t, "text");
    it.text = get_str(t, "text");
    it.font = get_str(t, "font");
    it.hint = get_str(t, "hint");
    if let Some(c) = get_color(t, "text_color") {
        it.text_color = c;
    }
    (it.has_text_hover, it.text_hover) = get_color(t, "text_hover").map_or((false, 0), |c| (true, c));
    (it.has_text_selected, it.text_selected) =
        get_color(t, "text_selected").map_or((false, 0), |c| (true, c));
    it.align = get_str(t, "align").chars().next().unwrap_or('l');
    it.valign = get_str(t, "valign").chars().next().unwrap_or('c');
    it.wrap = get_bool(t, "wrap", false);
    it.icon = get_str(t, "icon");
    it.icon_size = get_f32(t, "icon_size", 24.0);
    if let Some(c) = get_color(t, "icon_color") {
        it.icon_color = c;
    }
    (it.has_icon_selected, it.icon_selected) =
        get_color(t, "icon_selected").map_or((false, 0), |c| (true, c));
    it.action = get_list(t, "action");
    it.selected_on = get_list(t, "selected_on");
    it.row_h = get_f32(t, "row_h", 0.0);
    it.gap = get_f32(t, "gap", 0.0);
    it.empty = get_str(t, "empty");
    it.row = u32::try_from(get_int(t, "row", 0)).unwrap_or(0);
    it.zoom = get_str(t, "zoom");
    it
}

fn read_spot(t: &Table) -> MapSpot {
    let size = get_f32(t, "size", 24.0);
    let rect = sub(t, "rect")
        .map(|r| [get_f32(r, "x", 0.0), get_f32(r, "y", 0.0), get_f32(r, "w", 0.0), get_f32(r, "h", 0.0)]);
    let inner = sub(t, "inner").map(|i| InnerSpot {
        icon: get_str(i, "icon"),
        color: get_color(i, "color").unwrap_or(0xFFFF_FFFF),
        size: get_f32(i, "size", f64::from(size) * 0.5),
        x: get_f32(i, "x", 0.0),
        y: get_f32(i, "y", 0.0),
    });
    MapSpot {
        name: get_str(t, "name"),
        icon: get_str(t, "icon"),
        arrows: get_str(t, "arrows"),
        color: get_color(t, "color").unwrap_or(0xFFFF_FFFF),
        size,
        heading: get_bool(t, "heading", false),
        level: i32::try_from(get_int(t, "level", 5)).unwrap_or(5),
        rect,
        inner,
    }
}

/// The layout of the reader's answer `r` (`normalize`'s table).
#[must_use]
pub fn from_reader(r: &Table) -> Layout {
    let mut out = Layout {
        ok: get_bool(r, "ok", false),
        errors: u32::try_from(get_int(r, "errors", 0)).unwrap_or(0),
        warnings: u32::try_from(get_int(r, "warnings", 0)).unwrap_or(0),
        id: get_str(r, "id"),
        title: get_str(r, "title"),
        start: get_str(r, "start"),
        ..Layout::default()
    };
    if let Some(c) = sub(r, "canvas") {
        out.canvas_w = get_f32(c, "w", 960.0);
        out.canvas_h = get_f32(c, "h", 680.0);
    }
    if let Some(d) = sub(r, "device") {
        out.noise = get_bool(d, "noise", true);
        out.close = get_bool(d, "close", true);
    }
    if let Some(k) = sub(r, "keys") {
        out.key_map = get_str(k, "map");
        out.key_tasks = get_str(k, "tasks");
        out.key_contacts = get_str(k, "contacts");
    }
    if let Some(v) = sub(r, "vars") {
        out.vars = v
            .fields
            .iter()
            .filter_map(|(k, val)| val.as_str().map(|s| (k.to_string(), s.to_owned())))
            .collect();
        out.vars.sort();
    }
    out.pages = tables(r, "pages")
        .map(|p| Page { id: get_str(p, "id"), title: get_str(p, "title"), stock: get_str(p, "stock") })
        .collect();
    out.items = tables(r, "items").map(read_item).collect();
    out.map_spots = tables(r, "map_spots").map(read_spot).collect();
    out.problems = tables(r, "problems")
        .map(|p| PdaProblem {
            code: get_str(p, "code"),
            path: get_str(p, "path"),
            message: get_str(p, "message"),
        })
        .collect();
    out
}

/// `%g` of a number, as a preview var is formatted.
fn g(n: f64) -> String {
    gz_luadata::format_g(n, 6)
}

/// Reads `preview = { ... }` off the parsed asset (the reader ignores it).
pub fn read_preview(asset: &Value, out: &mut Layout) {
    let Some(pv) = asset.get("preview").filter(|v| v.is_table()) else { return };
    let Some(p) = pv.as_table() else { return };
    out.preview_level = pv.get_string("level", "");
    if let Some(a) = field(p, "actor").and_then(Value::as_table) {
        let mut x = field(a, "x");
        let mut z = field(a, "z");
        if x.is_none() && a.arr.len() >= 2 {
            x = a.arr.first();
            z = a.arr.get(if a.arr.len() >= 3 { 2 } else { 1 });
        }
        if let (Some(Value::Number(x)), Some(Value::Number(z))) = (x, z) {
            out.actor = Some((*x as f32, *z as f32));
        }
    }
    if let Some(vars) = field(p, "vars").and_then(Value::as_table) {
        for (k, v) in &vars.fields {
            match v {
                Value::String(s) => out.preview_vars.push((k.to_string(), s.to_string())),
                Value::Number(n) => out.preview_vars.push((k.to_string(), g(*n))),
                _ => {}
            }
        }
    }
    if let Some(ms) = field(p, "markers").and_then(Value::as_table) {
        for m in &ms.arr {
            if !m.is_table() {
                continue;
            }
            out.markers.push(Marker {
                x: m.get_number("x", 0.0) as f32,
                z: m.get_number("z", 0.0) as f32,
                icon: m.get_string("icon", "ui_xms_icon_pin"),
                hint: m.get_string("hint", ""),
                spot: m.get_string("spot", ""),
                color: 0xFFFF_D21E,
            });
        }
    }
}
