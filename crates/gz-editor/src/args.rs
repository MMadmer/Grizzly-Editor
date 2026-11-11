use gz_store::CmdError;
use serde_json::{Map, Value};

/// The arguments of a command: the request object without its `cmd`.
pub type Args = Map<String, Value>;

/// Reads arguments the forgiving way the wire has always been read: a number may come as a
/// string, a flag as `1` or `"true"`.
pub trait ArgsExt {
    fn str(&self, key: &str) -> Option<&str>;
    /// A non-empty string.
    fn text(&self, key: &str) -> Option<&str>;
    fn bool(&self, key: &str) -> Option<bool>;
    fn f64(&self, key: &str) -> Option<f64>;
    fn i64(&self, key: &str) -> Option<i64>;
    fn has(&self, key: &str) -> bool;

    fn f32(&self, key: &str) -> Option<f32> {
        self.f64(key).map(|v| v as f32)
    }
    fn u32(&self, key: &str) -> Option<u32> {
        self.i64(key).and_then(|v| u32::try_from(v).ok())
    }
    fn usize(&self, key: &str) -> Option<usize> {
        self.i64(key).and_then(|v| usize::try_from(v).ok())
    }
    fn flag(&self, key: &str) -> bool {
        self.bool(key).unwrap_or(false)
    }
    fn str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.text(key).unwrap_or(default)
    }
    /// A string the command cannot work without.
    fn need(&self, key: &str) -> Result<&str, CmdError> {
        self.text(key).ok_or_else(|| CmdError::invalid(format!("missing '{key}' argument")))
    }
    /// Three numbers: `[x, y, z]`, or `"x,y,z"`, or the keys `x`, `y`, `z` under `key`.
    fn vec3(&self, key: &str) -> Option<[f32; 3]>;
    /// A list of strings: an array, or one comma separated string.
    fn list(&self, key: &str) -> Option<Vec<String>>;
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

impl ArgsExt for Args {
    fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.str(key).filter(|s| !s.is_empty())
    }

    fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Bool(b) => Some(*b),
            Value::Number(n) => n.as_f64().map(|v| v != 0.0),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(as_f64)
    }

    fn i64(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => s.trim().parse().ok(),
            Value::Bool(b) => Some(i64::from(*b)),
            _ => None,
        }
    }

    fn has(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| !v.is_null())
    }

    fn vec3(&self, key: &str) -> Option<[f32; 3]> {
        let take = |a: Option<f64>, b: Option<f64>, c: Option<f64>| Some([a? as f32, b? as f32, c? as f32]);
        match self.get(key)? {
            Value::Array(a) if a.len() == 3 => take(as_f64(&a[0]), as_f64(&a[1]), as_f64(&a[2])),
            Value::String(s) => {
                let mut it =
                    s.split([',', ' ']).filter(|p| !p.is_empty()).map(|p| p.trim().parse::<f64>().ok());
                let out = take(it.next()?, it.next()?, it.next()?);
                if it.next().is_some() { None } else { out }
            }
            Value::Object(o) => {
                take(o.get("x").and_then(as_f64), o.get("y").and_then(as_f64), o.get("z").and_then(as_f64))
            }
            _ => None,
        }
    }

    fn list(&self, key: &str) -> Option<Vec<String>> {
        match self.get(key)? {
            Value::Array(a) => Some(a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()),
            Value::String(s) => {
                Some(s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect())
            }
            _ => None,
        }
    }
}
