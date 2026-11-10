//! Values across the border: `gz_luadata::Value` into Lua and back.

use gz_luadata::{Table, Value};
use mlua::Lua;
use smol_str::SmolStr;

use crate::host::Limits;

/// A value as a Lua value: the array part first, then the string keys.
pub(crate) fn to_lua(lua: &Lua, v: &Value, depth: usize) -> mlua::Result<mlua::Value> {
    Ok(match v {
        Value::Nil => mlua::Value::Nil,
        Value::Bool(b) => mlua::Value::Boolean(*b),
        Value::Number(n) => mlua::Value::Number(*n),
        Value::String(s) => mlua::Value::String(lua.create_string(s.as_bytes())?),
        Value::Table(t) => {
            if depth == 0 {
                return Err(mlua::Error::runtime("the value nests too deep"));
            }
            let out = lua.create_table_with_capacity(t.arr.len(), t.fields.len())?;
            for (i, item) in t.arr.iter().enumerate() {
                out.raw_set(i + 1, to_lua(lua, item, depth - 1)?)?;
            }
            for (k, item) in &t.fields {
                if !item.is_nil() {
                    out.raw_set(k.as_str(), to_lua(lua, item, depth - 1)?)?;
                }
            }
            mlua::Value::Table(out)
        }
    })
}

struct Budget<'a> {
    limits: &'a Limits,
    values: usize,
}

/// A Lua value as data. A table's `1..n` (up to the first nil) is its array part, its string
/// keys its fields in the order `next` walks them; other keys, functions and userdata are no
/// data and are left out. `omit` names keys of the top table to leave out.
pub(crate) fn from_lua(v: &mlua::Value, omit: &[&str], limits: &Limits) -> Result<Value, String> {
    let mut budget = Budget { limits, values: 0 };
    Ok(convert(v, omit, 0, &mut budget)?.unwrap_or(Value::Nil))
}

fn convert(v: &mlua::Value, omit: &[&str], depth: usize, budget: &mut Budget<'_>) -> Result<Option<Value>, String> {
    budget.values += 1;
    if budget.values > budget.limits.values {
        return Err(format!("the result holds more than {} values", budget.limits.values));
    }
    Ok(Some(match v {
        mlua::Value::Nil => Value::Nil,
        mlua::Value::Boolean(b) => Value::Bool(*b),
        mlua::Value::Integer(i) => Value::Number(*i as f64),
        mlua::Value::Number(n) => Value::Number(*n),
        mlua::Value::String(s) => Value::String(SmolStr::new(s.to_string_lossy())),
        mlua::Value::Table(t) => {
            if depth >= budget.limits.depth {
                return Err(format!("the result nests deeper than {} tables", budget.limits.depth));
            }
            let mut out = Table::new();
            for i in 1usize.. {
                let item: mlua::Value = t.raw_get(i).map_err(|e| crate::host::message(&e))?;
                if item.is_nil() {
                    break;
                }
                out.arr.push(convert(&item, &[], depth + 1, budget)?.unwrap_or(Value::Nil));
            }
            // integer keys are the array part's or no field at all
            for pair in t.pairs::<mlua::Value, mlua::Value>() {
                let (k, item) = pair.map_err(|e| crate::host::message(&e))?;
                let key = match &k {
                    mlua::Value::String(s) => s.to_string_lossy(),
                    _ => continue,
                };
                if omit.contains(&key.as_str()) {
                    continue;
                }
                if let Some(value) = convert(&item, &[], depth + 1, budget)? {
                    out.fields.push((SmolStr::new(key), value));
                }
            }
            Value::Table(out)
        }
        _ => return Ok(None),
    }))
}
