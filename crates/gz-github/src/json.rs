//! Reading GitHub's and VirusTotal's answers the forgiving way the C++ `SJson` did: a missing key
//! or one of another type reads as empty, `0` or `false`.

use serde_json::Value;

pub(crate) fn parse(body: &[u8]) -> Option<Value> {
    serde_json::from_slice(body).ok()
}

pub(crate) fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A plain non-negative integer; ids are one.
pub(crate) fn u64_of(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

pub(crate) fn bool_of(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) fn array_of<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}
