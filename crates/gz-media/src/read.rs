//! Typed reads of a media asset's values against its key tables: a value of the wrong type, out
//! of its range or not one of its words is an issue at its key (`M109`), and the key reads as
//! left out.

use gz_base::Severity;
use gz_luadata::{Seg, Value};

use crate::fields::{Field, Ty, value_of};

/// A problem of a file found while it is read, at the key it is about.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub code: &'static str,
    pub severity: Severity,
    /// Where in the file: a path of keys and list items.
    pub path: Vec<Seg>,
    pub message: String,
}

impl Issue {
    #[must_use]
    pub fn new(code: &'static str, severity: Severity, path: Vec<Seg>, message: impl Into<String>) -> Self {
        Self { code, severity, path, message: message.into() }
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// A value outside what its key takes.
pub const BAD_VALUE: &str = "M109";

/// What a read gathers: where it is in the file and the issues so far.
#[derive(Default)]
pub struct Cx {
    pub path: Vec<Seg>,
    pub issues: Vec<Issue>,
}

impl Cx {
    /// The path of a key at the current place.
    #[must_use]
    pub fn at(&self, key: &str) -> Vec<Seg> {
        let mut p = self.path.clone();
        p.push(Seg::key(key));
        p
    }

    pub fn bad(&mut self, key: &str, message: impl Into<String>) {
        let path = self.at(key);
        self.issues.push(Issue::new(BAD_VALUE, Severity::Error, path, message));
    }

    /// Runs `f` one level down: a key, then an item of its list.
    pub fn within<R>(&mut self, segs: &[Seg], f: impl FnOnce(&mut Self) -> R) -> R {
        let depth = self.path.len();
        self.path.extend_from_slice(segs);
        let r = f(self);
        self.path.truncate(depth);
        r
    }
}

fn field<'a>(fields: &'a [Field], key: &str) -> Option<&'a Field> {
    fields.iter().find(|f| f.key == key)
}

fn range(fields: &[Field], key: &str) -> (f64, f64) {
    match field(fields, key).map(|f| f.ty) {
        Some(Ty::Number { min, max }) => (min, max),
        Some(Ty::Integer { min, max }) => (min as f64, max as f64),
        _ => (f64::MIN, f64::MAX),
    }
}

/// A number in its key's range.
pub fn number(cx: &mut Cx, v: &Value, fields: &[Field], key: &str) -> Option<f64> {
    let raw = v.get(key)?;
    let Some(n) = raw.as_number() else {
        cx.bad(key, format!("'{key}' takes a number"));
        return None;
    };
    let (min, max) = range(fields, key);
    if !n.is_finite() || n < min || n > max {
        cx.bad(key, format!("'{key}' is {n}; it takes {min} to {max}"));
        return None;
    }
    Some(n)
}

/// A whole number in its key's range.
pub fn integer(cx: &mut Cx, v: &Value, fields: &[Field], key: &str) -> Option<i64> {
    let n = number(cx, v, fields, key)?;
    if n.fract() != 0.0 {
        cx.bad(key, format!("'{key}' takes a whole number"));
        return None;
    }
    Some(n as i64)
}

pub fn boolean(cx: &mut Cx, v: &Value, key: &str) -> Option<bool> {
    let raw = v.get(key)?;
    let b = raw.as_bool();
    if b.is_none() {
        cx.bad(key, format!("'{key}' takes true or false"));
    }
    b
}

pub fn text(cx: &mut Cx, v: &Value, key: &str) -> Option<String> {
    let raw = v.get(key)?;
    match raw.as_str() {
        Some(s) => Some(s.to_owned()),
        None => {
            cx.bad(key, format!("'{key}' takes a string"));
            None
        }
    }
}

/// One of its key's words, as the number it stands for.
pub fn word(cx: &mut Cx, v: &Value, fields: &[Field], key: &str) -> Option<u32> {
    let s = text(cx, v, key)?;
    let Some(Ty::Words(words)) = field(fields, key).map(|f| f.ty) else { return None };
    match value_of(words, &s) {
        Some(n) => Some(n),
        None => {
            let mut list: Vec<&str> = Vec::new();
            for (w, _) in words {
                if !list.contains(w) {
                    list.push(w);
                }
            }
            cx.bad(key, format!("'{key}' is \"{s}\"; it takes {}", list.join(", ")));
            None
        }
    }
}

/// A list of `n` numbers.
pub fn numbers<const N: usize>(cx: &mut Cx, v: &Value, key: &str) -> Option<[f64; N]> {
    let raw = v.get(key)?;
    let items = raw.arr();
    if !raw.is_table()
        || items.len() != N
        || items.iter().any(|x| x.as_number().is_none_or(|n| !n.is_finite()))
    {
        cx.bad(key, format!("'{key}' takes a list of {N} numbers"));
        return None;
    }
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(items) {
        *o = x.as_number().unwrap_or(0.0);
    }
    Some(out)
}

/// `{ r, g, b, a }`, each 0 to 255, as `0xAARRGGBB`.
pub fn color(cx: &mut Cx, v: &Value, key: &str) -> Option<u32> {
    let c = numbers::<4>(cx, v, key)?;
    if c.iter().any(|x| *x < 0.0 || *x > 255.0 || x.fract() != 0.0) {
        cx.bad(key, format!("'{key}' takes four whole numbers from 0 to 255: {{ r, g, b, a }}"));
        return None;
    }
    let [r, g, b, a] = c.map(|x| x as u32);
    Some((a << 24) | (r << 16) | (g << 8) | b)
}

/// `0xAARRGGBB` as the file writes it.
#[must_use]
pub fn color_value(argb: u32) -> Value {
    let (a, r, g, b) = (argb >> 24, (argb >> 16) & 0xff, (argb >> 8) & 0xff, argb & 0xff);
    Value::from(vec![Value::from(r), Value::from(g), Value::from(b), Value::from(a)])
}

/// The items of a list key; a key that is no list is an issue and reads as empty.
pub fn list<'v>(cx: &mut Cx, v: &'v Value, key: &str) -> &'v [Value] {
    match v.get(key) {
        None => &[],
        Some(raw) if raw.is_table() && raw.fields().is_empty() => raw.arr(),
        Some(_) => {
            cx.bad(key, format!("'{key}' takes a list"));
            &[]
        }
    }
}

/// A list item's path: by its `name` when it has one, else by its place.
#[must_use]
pub fn item_seg(item: &Value, index: usize) -> Seg {
    match item.get("name").and_then(Value::as_str) {
        Some(n) if !n.is_empty() => Seg::item("name", n),
        _ => Seg::Index(index),
    }
}

/// A list of numbers for the writer.
#[must_use]
pub fn number_list(values: &[f64]) -> Value {
    Value::from(values.iter().map(|n| Value::from(*n)).collect::<Vec<_>>())
}

/// A game name as the game's lists key it: lower case, back slashes, no extension the editor
/// knows of.
#[must_use]
pub fn name_key(name: &str) -> String {
    let n = name.trim().replace('/', "\\").to_lowercase();
    let n = n.trim_start_matches('\\').to_owned();
    for ext in [".dds", ".ogg", ".ogf", ".tga", ".png"] {
        if let Some(stem) = n.strip_suffix(ext) {
            return stem.to_owned();
        }
    }
    n
}
