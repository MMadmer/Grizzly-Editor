//! A value as JSON and back, in the shape the `Serialize` impl of [`Value`] gives: nil is
//! `null`, a whole number below 9e15 an integer, a table without keys an array, a table with
//! keys an object whose array part comes first under `"1"`, `"2"`, ...

use serde_json::{Map, Number};

use crate::value::{Table, Value};

/// The value as a `serde_json::Value`. A number that is not finite becomes `null`, as JSON has
/// no form for it.
#[must_use]
pub fn to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Nil => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                serde_json::Value::Number(Number::from(*n as i64))
            } else {
                Number::from_f64(*n).map_or(serde_json::Value::Null, serde_json::Value::Number)
            }
        }
        Value::String(s) => serde_json::Value::String(s.to_string()),
        Value::Table(t) => {
            if t.fields.is_empty() {
                return serde_json::Value::Array(t.arr.iter().map(to_json).collect());
            }
            let mut m = Map::with_capacity(t.arr.len() + t.fields.len());
            for (i, x) in t.arr.iter().enumerate() {
                m.insert((i + 1).to_string(), to_json(x));
            }
            for (k, x) in &t.fields {
                m.insert(k.to_string(), to_json(x));
            }
            serde_json::Value::Object(m)
        }
    }
}

/// A JSON value as a Lua value. An object's leading keys `"1"`, `"2"`, ... in that order are
/// its array part; every other key is set in the canonical order.
#[must_use]
pub fn from_json(j: &serde_json::Value) -> Value {
    match j {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::from(s.as_str()),
        serde_json::Value::Array(a) => {
            Value::Table(Table { arr: a.iter().map(from_json).collect(), fields: Vec::new() })
        }
        serde_json::Value::Object(m) => {
            let mut t = Table::new();
            let mut next = 1usize;
            let mut in_array = true;
            for (k, x) in m {
                if in_array && *k == next.to_string() {
                    t.arr.push(from_json(x));
                    next += 1;
                    continue;
                }
                in_array = false;
                t.set(k, from_json(x));
            }
            Value::Table(t)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut t = Table::new();
        t.push(1);
        t.push("x");
        t.set("kind", "k");
        t.set("w", 0.5);
        let v = Value::Table(t);
        let j = to_json(&v);
        assert_eq!(j.to_string(), r#"{"1":1,"2":"x","kind":"k","w":0.5}"#);
        assert_eq!(from_json(&j), v);
        assert_eq!(serde_json::to_value(&v).ok(), Some(j));
        let list: Value = vec![Value::Nil, Value::Bool(false)].into();
        assert_eq!(to_json(&list).to_string(), "[null,false]");
        assert_eq!(to_json(&Value::table()).to_string(), "[]");
    }
}
