//! The value a declarative Lua file holds.

use std::cmp::Ordering;

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use smol_str::SmolStr;

use crate::write::number_to_text;

/// The well-known keys, in the order every generic table is written in; any other key follows
/// them in byte order. [`key_rank_bytes`] is this list as a match.
#[cfg(test)]
const KEY_ORDER: &[&str] = &[
    "kind",
    "name",
    "npc",
    "who",
    "task",
    "quest",
    "target",
    "character",
    "section",
    "story",
    "profile",
    "community",
    "level",
    "place",
    "into",
    "smart",
    "spawn",
    "restrictor",
    "ref",
    "hold",
    "pos",
    "radius",
    "x",
    "y",
    "z",
    "text",
    "title",
    "descr",
    "initiator",
    "sender",
    "icon",
    "spot",
    "op",
    "value",
    "delta",
    "is",
    "by_actor",
    "count",
    "amount",
    "info",
    "spot_task",
    "spot_objective",
    "follow_actor",
    "allow_break",
    "status",
    "restart",
    "repeat",
    "cooldown",
    "timeout",
    "since",
    "node",
    "duration",
    "seconds",
    "game_hours",
    "game_minutes",
    "from",
    "to",
    "percent",
    "theme",
    "code",
    "of",
    "cond",
    "weight",
    "params",
    "not",
    "rus",
    "eng",
    "ukr",
    "pol",
    "fra",
    "ger",
    "ita",
    "spa",
];

/// The rank of a key in the canonical order; 100000 for a key that is not well known.
#[must_use]
pub fn key_rank(key: &str) -> u32 {
    key_rank_bytes(key.as_bytes())
}

/// [`key_rank`] over bytes. Like a C string compare, it looks at the key up to its first NUL.
#[must_use]
pub fn key_rank_bytes(key: &[u8]) -> u32 {
    let key = key.split(|&b| b == 0).next().unwrap_or(&[]);
    // by length first: this runs for every key of every table a file holds
    match key.len() {
        1 => match key {
            b"x" => 22,
            b"y" => 23,
            b"z" => 24,
            _ => 100_000,
        },
        2 => match key {
            b"op" => 32,
            b"is" => 35,
            b"to" => 56,
            b"of" => 60,
            _ => 100_000,
        },
        3 => match key {
            b"npc" => 2,
            b"who" => 3,
            b"ref" => 18,
            b"pos" => 20,
            b"not" => 64,
            b"rus" => 65,
            b"eng" => 66,
            b"ukr" => 67,
            b"pol" => 68,
            b"fra" => 69,
            b"ger" => 70,
            b"ita" => 71,
            b"spa" => 72,
            _ => 100_000,
        },
        4 => match key {
            b"kind" => 0,
            b"name" => 1,
            b"task" => 4,
            b"into" => 14,
            b"hold" => 19,
            b"text" => 25,
            b"icon" => 30,
            b"spot" => 31,
            b"info" => 39,
            b"node" => 50,
            b"from" => 55,
            b"code" => 59,
            b"cond" => 61,
            _ => 100_000,
        },
        5 => match key {
            b"quest" => 5,
            b"story" => 9,
            b"level" => 12,
            b"place" => 13,
            b"smart" => 15,
            b"spawn" => 16,
            b"title" => 26,
            b"descr" => 27,
            b"value" => 33,
            b"delta" => 34,
            b"count" => 37,
            b"since" => 49,
            b"theme" => 58,
            _ => 100_000,
        },
        6 => match key {
            b"target" => 6,
            b"radius" => 21,
            b"sender" => 29,
            b"amount" => 38,
            b"status" => 44,
            b"repeat" => 46,
            b"weight" => 62,
            b"params" => 63,
            _ => 100_000,
        },
        7 => match key {
            b"section" => 8,
            b"profile" => 10,
            b"restart" => 45,
            b"timeout" => 48,
            b"seconds" => 52,
            b"percent" => 57,
            _ => 100_000,
        },
        8 => match key {
            b"by_actor" => 36,
            b"cooldown" => 47,
            b"duration" => 51,
            _ => 100_000,
        },
        9 => match key {
            b"character" => 7,
            b"community" => 11,
            b"initiator" => 28,
            b"spot_task" => 40,
            _ => 100_000,
        },
        10 => match key {
            b"restrictor" => 17,
            b"game_hours" => 53,
            _ => 100_000,
        },
        11 => match key {
            b"allow_break" => 43,
            _ => 100_000,
        },
        12 => match key {
            b"follow_actor" => 42,
            b"game_minutes" => 54,
            _ => 100_000,
        },
        14 => match key {
            b"spot_objective" => 41,
            _ => 100_000,
        },
        _ => 100_000,
    }
}

/// The canonical order of two keys: by rank, then by bytes.
#[must_use]
pub fn key_cmp(a: &str, b: &str) -> Ordering {
    key_cmp_bytes(a.as_bytes(), b.as_bytes())
}

/// [`key_cmp`] over bytes.
#[must_use]
pub fn key_cmp_bytes(a: &[u8], b: &[u8]) -> Ordering {
    key_rank_bytes(a).cmp(&key_rank_bytes(b)).then_with(|| a.cmp(b))
}

/// A Lua table as data: the array part in order, then the string keys in the order they were
/// put. The reader and [`Table::set`] keep the keys canonical ([`key_cmp`]); a typed writer
/// appends them in its own order with [`Table::put`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub arr: Vec<Value>,
    pub fields: Vec<(SmolStr, Value)>,
}

impl Table {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.arr.is_empty() && self.fields.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The slot of a key: the existing one, or a new nil at its canonical place.
    pub fn slot(&mut self, key: &str) -> &mut Value {
        let at = match self.fields.iter().position(|(k, _)| k == key) {
            Some(at) => at,
            None => {
                let at = self
                    .fields
                    .iter()
                    .position(|(k, _)| key_cmp(k, key) != Ordering::Less)
                    .unwrap_or(self.fields.len());
                self.fields.insert(at, (SmolStr::new(key), Value::Nil));
                at
            }
        };
        &mut self.fields[at].1
    }

    /// Sets a key, keeping the canonical order on insert.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        *self.slot(key) = value.into();
    }

    /// Appends a key as it is: a typed writer carries its own field order.
    pub fn put(&mut self, key: &str, value: impl Into<Value>) {
        self.fields.push((SmolStr::new(key), value.into()));
    }

    pub fn erase(&mut self, key: &str) {
        if let Some(at) = self.fields.iter().position(|(k, _)| k == key) {
            self.fields.remove(at);
        }
    }

    pub fn push(&mut self, value: impl Into<Value>) {
        self.arr.push(value.into());
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(|(k, _)| k.as_str())
    }
}

/// What a declarative Lua file holds: nil, a boolean, a number, a string or a table.
///
/// Equality is structural and compares numbers exactly; two tables are equal when their array
/// parts and their keys, in order, are.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Number(f64),
    String(SmolStr),
    Table(Table),
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Number(f64::from(v))
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Number(f64::from(v))
    }
}
impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::Number(f64::from(v))
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(SmolStr::new(v))
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(SmolStr::new(v))
    }
}
impl From<SmolStr> for Value {
    fn from(v: SmolStr) -> Self {
        Value::String(v)
    }
}
impl From<Table> for Value {
    fn from(v: Table) -> Self {
        Value::Table(v)
    }
}
impl From<Vec<Value>> for Value {
    fn from(arr: Vec<Value>) -> Self {
        Value::Table(Table { arr, fields: Vec::new() })
    }
}

impl Value {
    /// An empty table.
    #[must_use]
    pub fn table() -> Self {
        Value::Table(Table::new())
    }

    #[must_use]
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }
    #[must_use]
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }
    #[must_use]
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }
    #[must_use]
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }
    #[must_use]
    pub fn is_table(&self) -> bool {
        matches!(self, Value::Table(_))
    }

    /// Nil, or a table with neither array nor keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Nil => true,
            Value::Table(t) => t.is_empty(),
            _ => false,
        }
    }

    #[must_use]
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }

    /// The table to edit. Nil becomes an empty table; any other value has none.
    pub fn table_mut(&mut self) -> Option<&mut Table> {
        if self.is_nil() {
            *self = Value::table();
        }
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The array part; empty when this is not a table.
    #[must_use]
    pub fn arr(&self) -> &[Value] {
        self.as_table().map_or(&[], |t| &t.arr)
    }

    /// The keyed part; empty when this is not a table.
    #[must_use]
    pub fn fields(&self) -> &[(SmolStr, Value)] {
        self.as_table().map_or(&[], |t| &t.fields)
    }

    /// A string key of a table; `None` when absent or when this is not a table.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_table()?.get(key)
    }

    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// [`Table::set`] on a table or on nil. False when this is another kind of value.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) -> bool {
        match self.table_mut() {
            Some(t) => {
                t.set(key, value);
                true
            }
            None => false,
        }
    }

    /// [`Table::put`] on a table or on nil. False when this is another kind of value.
    pub fn put(&mut self, key: &str, value: impl Into<Value>) -> bool {
        match self.table_mut() {
            Some(t) => {
                t.put(key, value);
                true
            }
            None => false,
        }
    }

    pub fn erase(&mut self, key: &str) {
        if let Value::Table(t) = self {
            t.erase(key);
        }
    }

    /// Appends to the array part of a table or of nil. False when this is another kind of value.
    pub fn push(&mut self, value: impl Into<Value>) -> bool {
        match self.table_mut() {
            Some(t) => {
                t.push(value);
                true
            }
            None => false,
        }
    }

    /// The value as text: a string as it is, a number in its shortest plain form, a boolean
    /// as `true` / `false`, anything else `default`.
    #[must_use]
    pub fn to_text(&self, default: &str) -> String {
        match self {
            Value::String(s) => s.to_string(),
            Value::Number(n) => number_to_text(*n),
            Value::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
            _ => default.to_owned(),
        }
    }

    /// The value as a number: a number, a string that is one entirely, a boolean as 1 / 0,
    /// anything else `default`.
    #[must_use]
    pub fn to_number(&self, default: f64) -> f64 {
        match self {
            Value::Number(n) => *n,
            Value::String(s) if !s.is_empty() => crate::num::strtod_whole(s).unwrap_or(default),
            Value::Bool(b) => f64::from(u8::from(*b)),
            _ => default,
        }
    }

    /// The value as a boolean: a boolean, a number that is not zero, the strings `true` and
    /// `1`, anything else `default`.
    #[must_use]
    pub fn to_bool(&self, default: bool) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0,
            Value::String(s) => s == "true" || s == "1",
            _ => default,
        }
    }

    #[must_use]
    pub fn get_string(&self, key: &str, default: &str) -> String {
        self.get(key).map_or_else(|| default.to_owned(), |v| v.to_text(default))
    }

    #[must_use]
    pub fn get_number(&self, key: &str, default: f64) -> f64 {
        self.get(key).map_or(default, |v| v.to_number(default))
    }

    #[must_use]
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.get(key).map_or(default, |v| v.to_bool(default))
    }

    /// Sorts the string keys of every table canonically, after bulk edits. Stable, so two
    /// equal keys keep their order.
    pub fn canonicalize(&mut self) {
        if let Value::Table(t) = self {
            t.fields.sort_by(|a, b| key_cmp(&a.0, &b.0));
            for v in &mut t.arr {
                v.canonicalize();
            }
            for (_, v) in &mut t.fields {
                v.canonicalize();
            }
        }
    }

    /// Merges the string keys of `patch` into this table: a nil erases, anything else sets. An
    /// array part in the patch replaces this one whole. Nil becomes a table first; any other
    /// value is left alone.
    pub fn patch(&mut self, patch: &Value) {
        let Value::Table(from) = patch else { return };
        let Some(t) = self.table_mut() else { return };
        for (k, v) in &from.fields {
            if v.is_nil() {
                t.erase(k);
            } else {
                t.set(k, v.clone());
            }
        }
        if !from.arr.is_empty() {
            t.arr = from.arr.clone();
        }
    }
}

/// As JSON: nil is `null`, a number that is whole an integer, a table with keys an object and
/// one without an array. A table with both writes its array part under `"1"`, `"2"`, ...
/// before its keys.
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Nil => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 9.0e15 {
                    s.serialize_i64(*n as i64)
                } else {
                    s.serialize_f64(*n)
                }
            }
            Value::String(v) => s.serialize_str(v),
            Value::Table(t) => t.serialize(s),
        }
    }
}

impl Serialize for Table {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.fields.is_empty() {
            let mut seq = s.serialize_seq(Some(self.arr.len()))?;
            for v in &self.arr {
                seq.serialize_element(v)?;
            }
            return seq.end();
        }
        let mut map = s.serialize_map(Some(self.arr.len() + self.fields.len()))?;
        for (i, v) in self.arr.iter().enumerate() {
            map.serialize_entry(&(i + 1).to_string(), v)?;
        }
        for (k, v) in &self.fields {
            map.serialize_entry(k.as_str(), v)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_keep_the_canonical_order() {
        let mut v = Value::Nil;
        assert!(v.set("zeta", 1));
        assert!(v.set("params", 2));
        assert!(v.set("alpha", 3));
        assert!(v.set("kind", "x"));
        assert!(v.set("name", "n"));
        let keys: Vec<&str> = v.fields().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["kind", "name", "params", "alpha", "zeta"]);
        assert!(v.set("alpha", 4));
        assert_eq!(v.get_number("alpha", 0.0), 4.0);
        assert_eq!(v.fields().len(), 5);
        assert!(!Value::Number(1.0).set("a", 1));
        assert_eq!(key_rank("kind"), 0);
        assert_eq!(key_rank("spa"), 72);
        assert_eq!(key_rank("Kind"), 100_000);
        for (i, k) in KEY_ORDER.iter().enumerate() {
            assert_eq!(key_rank(k), i as u32, "{k}");
            assert_eq!(key_rank_bytes(format!("{k}\0x").as_bytes()), i as u32);
        }
        assert_eq!(key_rank("kinds"), 100_000);
    }

    #[test]
    fn put_keeps_the_writer_order_and_canonicalize_restores_it() {
        let mut t = Table::new();
        t.put("id", "a");
        t.put("kind", "k");
        t.put("once", true);
        assert_eq!(t.keys().collect::<Vec<_>>(), ["id", "kind", "once"]);
        let mut v = Value::Table(t);
        v.canonicalize();
        assert_eq!(v.fields().iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["kind", "id", "once"]);
    }

    #[test]
    fn typed_reads() {
        assert_eq!(Value::Number(3.0).to_text(""), "3");
        assert_eq!(Value::Number(-0.5).to_text(""), "-0.5");
        assert_eq!(Value::Number(1e20).to_text(""), "1e+20");
        assert_eq!(Value::Bool(true).to_text(""), "true");
        assert_eq!(Value::Nil.to_text("d"), "d");
        assert_eq!(Value::from(" 12.5").to_number(0.0), 12.5);
        assert_eq!(Value::from("12.5 ").to_number(-1.0), -1.0);
        assert_eq!(Value::from("0x10").to_number(0.0), 16.0);
        assert_eq!(Value::from("abc").to_number(7.0), 7.0);
        assert_eq!(Value::Bool(true).to_number(0.0), 1.0);
        assert!(Value::from("1").to_bool(false));
        assert!(!Value::from("yes").to_bool(true));
        assert!(Value::Nil.to_bool(true));
    }

    #[test]
    fn patch_sets_erases_and_replaces_the_list() {
        let mut v = Value::table();
        v.set("a", 1);
        v.set("b", 2);
        v.push(1);
        let mut p = Value::table();
        p.put("a", Value::Nil);
        p.put("c", 3);
        p.push(9);
        p.push(8);
        v.patch(&p);
        assert!(!v.has("a"));
        assert_eq!(v.get_number("c", 0.0), 3.0);
        assert_eq!(v.arr().len(), 2);
        let mut n = Value::Number(1.0);
        n.patch(&p);
        assert_eq!(n, Value::Number(1.0));
    }
}
