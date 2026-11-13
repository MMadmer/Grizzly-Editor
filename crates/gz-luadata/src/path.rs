//! Paths into a value or a document: `nodes[id=meet].params.text`, `objects[3]`.

use std::fmt;

use smol_str::SmolStr;

use crate::value::Value;

/// One step of a path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Seg {
    /// A string key of a table.
    Key(SmolStr),
    /// The n-th item of the array part, from 0.
    Index(usize),
    /// The item of the array part whose `field` (`id`, `name`) is `value`.
    Item { field: SmolStr, value: SmolStr },
}

impl Seg {
    #[must_use]
    pub fn key(k: &str) -> Self {
        Seg::Key(SmolStr::new(k))
    }

    #[must_use]
    pub fn item(field: &str, value: &str) -> Self {
        Seg::Item { field: SmolStr::new(field), value: SmolStr::new(value) }
    }
}

/// A path as text, the form [`path`] reads.
pub struct Shown<'a>(pub &'a [Seg]);

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, s) in self.0.iter().enumerate() {
            match s {
                Seg::Key(k) => {
                    if i > 0 {
                        f.write_str(".")?;
                    }
                    f.write_str(k)?;
                }
                Seg::Index(n) => write!(f, "[{n}]")?,
                Seg::Item { field, value } => write!(f, "[{field}={value}]")?,
            }
        }
        Ok(())
    }
}

/// A path as text: `a.b`, `[2]` for an index, `[id=x]` for a keyed item.
#[must_use]
pub fn show(p: &[Seg]) -> String {
    Shown(p).to_string()
}

/// Reads `nodes[id=meet].params.text`; `None` for a malformed path. The empty text is the
/// root.
#[must_use]
pub fn path(text: &str) -> Option<Vec<Seg>> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b.get(i) {
            Some(b'.') if !out.is_empty() => i += 1,
            Some(b'[') => {
                let close = text.get(i..)?.find(']')? + i;
                let inner = text.get(i + 1..close)?;
                if let Some((f, v)) = inner.split_once('=') {
                    out.push(Seg::item(f.trim(), v.trim()));
                } else {
                    out.push(Seg::Index(inner.trim().parse().ok()?));
                }
                i = close + 1;
                continue;
            }
            _ => {}
        }
        let rest = text.get(i..)?;
        let end = rest.find(['.', '[']).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        out.push(Seg::key(rest.get(..end)?));
        i += end;
    }
    Some(out)
}

fn item_matches(v: &Value, field: &str, value: &str) -> bool {
    match v.get(field) {
        Some(Value::String(s)) => s == value,
        Some(x @ Value::Number(_)) => x.to_text("") == value,
        _ => false,
    }
}

/// The value a path names.
#[must_use]
pub fn get_at<'a>(v: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    let mut cur = v;
    for s in path {
        cur = match s {
            Seg::Key(k) => cur.get(k)?,
            Seg::Index(n) => cur.arr().get(*n)?,
            Seg::Item { field, value } => cur.arr().iter().find(|x| item_matches(x, field, value))?,
        };
    }
    Some(cur)
}

/// The value a path names, to change.
pub fn get_at_mut<'a>(v: &'a mut Value, path: &[Seg]) -> Option<&'a mut Value> {
    let mut cur = v;
    for s in path {
        let Value::Table(t) = cur else { return None };
        cur = match s {
            Seg::Key(k) => t.get_mut(k)?,
            Seg::Index(n) => t.arr.get_mut(*n)?,
            Seg::Item { field, value } => t.arr.iter_mut().find(|x| item_matches(x, field, value))?,
        };
    }
    Some(cur)
}

/// Sets the value a path names: a missing key is added, a nil value removes the key. False
/// when the parent does not exist or is no table.
pub fn set_at(v: &mut Value, path: &[Seg], new: Value) -> bool {
    let Some((last, parent)) = path.split_last() else {
        *v = new;
        return true;
    };
    let Some(Value::Table(t)) = get_at_mut(v, parent) else { return false };
    match last {
        Seg::Key(k) => {
            if new.is_nil() {
                t.erase(k);
            } else if let Some(slot) = t.get_mut(k) {
                *slot = new;
            } else {
                t.put(k, new);
            }
            true
        }
        Seg::Index(n) => {
            if new.is_nil() {
                if *n < t.arr.len() {
                    t.arr.remove(*n);
                }
                return true;
            }
            if *n < t.arr.len() {
                t.arr[*n] = new;
            } else if *n == t.arr.len() {
                t.arr.push(new);
            } else {
                return false;
            }
            true
        }
        Seg::Item { field, value } => {
            let at = t.arr.iter().position(|x| item_matches(x, field, value));
            match (at, new.is_nil()) {
                (Some(i), true) => {
                    t.arr.remove(i);
                }
                (Some(i), false) => t.arr[i] = new,
                (None, true) => {}
                (None, false) => t.arr.push(new),
            }
            true
        }
    }
}

/// Values equal as data: numbers by value, tables by their keys whatever their order.
#[must_use]
pub fn data_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Table(x), Value::Table(y)) => {
            x.arr.len() == y.arr.len()
                && x.fields.len() == y.fields.len()
                && x.arr.iter().zip(&y.arr).all(|(p, q)| data_eq(p, q))
                && x.fields.iter().enumerate().all(|(i, (k, v))| {
                    // keys mostly stand in the same order on both sides
                    let w = match y.fields.get(i) {
                        Some((yk, w)) if yk == k => Some(w),
                        _ => y.get(k),
                    };
                    w.is_some_and(|w| data_eq(v, w))
                })
        }
        (Value::Number(x), Value::Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

/// Whether `doc` holds everything `model` names, equal as data; keys only `doc` has do not
/// count (they are the author's, unknown to the model).
#[must_use]
pub fn covers(doc: &Value, model: &Value) -> bool {
    match (doc, model) {
        (Value::Table(d), Value::Table(m)) => {
            d.arr.len() == m.arr.len()
                && d.arr.iter().zip(&m.arr).all(|(p, q)| covers(p, q))
                && m.fields.iter().all(|(k, v)| d.get(k).is_some_and(|w| covers(w, v)))
        }
        _ => data_eq(doc, model),
    }
}
