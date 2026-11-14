//! What a command says about itself: its name, what it does to the world and its parameters.
//!
//! A command's parameters are a struct that derives `Deserialize` and `JsonSchema`, with a doc
//! comment on every field. The registry reads the request into that struct, so what `help`
//! prints and what the command takes cannot drift apart: both come from the one type.

use std::fmt::Write as _;

use gz_store::CmdError;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// What running a command can do. It decides through which door a caller may run it: an
/// agent's ordinary tool runs `Read`, `View` and `Write`; `Destroy` and `Publish` each have a
/// tool of their own, so a client can ask its user on exactly those calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Answers from what is there and changes nothing.
    Read,
    /// Changes what the editor shows - a tab, a panel, a scroll position, the camera - and
    /// nothing an author keeps.
    View,
    /// Changes the project or the editor's settings in a way that can be undone or repeated.
    Write,
    /// Loses data for good: a file deleted, unsaved work dropped.
    Destroy,
    /// Reaches other people: a release, the catalog.
    Publish,
}

impl Effect {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::View => "view",
            Self::Write => "write",
            Self::Destroy => "destroy",
            Self::Publish => "publish",
        }
    }
}

/// The door a request came through. A request without one (the QA socket, the command line)
/// may run anything: the caller is the author.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gate {
    #[default]
    Any,
    /// The ordinary tool: everything that loses nothing and reaches nobody.
    Run,
    /// The tool for calls that lose data.
    Destroy,
    /// The tool for calls that reach players.
    Publish,
}

impl Gate {
    /// The gate a request names (`"gate": "run" | "destroy" | "release"`).
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "run" => Some(Self::Run),
            "destroy" => Some(Self::Destroy),
            "release" | "publish" => Some(Self::Publish),
            "" | "any" => Some(Self::Any),
            _ => None,
        }
    }

    /// Why `effect` may not pass, as a sentence that names the right door.
    pub(crate) fn refuse(self, name: &str, effect: Effect) -> Option<String> {
        let wrong = match (self, effect) {
            (Self::Any, _) => return None,
            (Self::Run, Effect::Read | Effect::View | Effect::Write) => return None,
            (Self::Destroy, Effect::Destroy) | (Self::Publish, Effect::Publish) => return None,
            (_, e) => e,
        };
        Some(match wrong {
            Effect::Destroy => format!("{name} loses data for good here: run it with grizzly_destroy"),
            Effect::Publish => format!("{name} reaches players: run it with grizzly_release"),
            _ => format!("{name} is an ordinary command: run it with grizzly_run"),
        })
    }
}

/// A command's card: what `help` lists and what the gates read.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// `area.verb`, lower case.
    pub name: &'static str,
    /// One line: what it does, for the list of its area.
    pub summary: &'static str,
    pub effect: Effect,
    /// Parameters that turn the call into one that loses data when given and not false
    /// (`discard`): such a call counts as [`Effect::Destroy`].
    pub destroys_with: &'static [&'static str],
    /// The arguments of a typical call, as they are written on a command line
    /// (`kind=object ref=props/box pos=1,2,3`). Empty when the parameters say it all.
    pub example: &'static str,
}

impl Spec {
    const fn new(name: &'static str, summary: &'static str, effect: Effect) -> Self {
        Self { name, summary, effect, destroys_with: &[], example: "" }
    }

    /// Adds the arguments of a typical call (see [`Spec::example`]).
    #[must_use]
    pub const fn example(mut self, args: &'static str) -> Self {
        self.example = args;
        self
    }

    #[must_use]
    pub const fn read(name: &'static str, summary: &'static str) -> Self {
        Self::new(name, summary, Effect::Read)
    }

    #[must_use]
    pub const fn view(name: &'static str, summary: &'static str) -> Self {
        Self::new(name, summary, Effect::View)
    }

    #[must_use]
    pub const fn write(name: &'static str, summary: &'static str) -> Self {
        Self::new(name, summary, Effect::Write)
    }

    #[must_use]
    pub const fn destroy(name: &'static str, summary: &'static str) -> Self {
        Self::new(name, summary, Effect::Destroy)
    }

    #[must_use]
    pub const fn publish(name: &'static str, summary: &'static str) -> Self {
        Self::new(name, summary, Effect::Publish)
    }

    /// Names the parameters that make a call lose data (see [`Spec::destroys_with`]).
    #[must_use]
    pub const fn destroys_with(mut self, params: &'static [&'static str]) -> Self {
        self.destroys_with = params;
        self
    }

    /// The area: what stands before the first dot of the name.
    #[must_use]
    pub fn area(&self) -> &'static str {
        self.name.split_once('.').map_or(self.name, |(a, _)| a)
    }

    /// What this call does, given its arguments.
    #[must_use]
    pub fn effect_of(&self, args: &Map<String, Value>) -> Effect {
        let loses = self.destroys_with.iter().any(|p| args.get(*p).is_some_and(truthy));
        if loses && self.effect < Effect::Destroy { Effect::Destroy } else { self.effect }
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => {
            !matches!(s.trim().to_ascii_lowercase().as_str(), "" | "false" | "0" | "no" | "off")
        }
        _ => true,
    }
}

/// The parameters of a command: any struct that reads itself from JSON and describes itself.
pub trait Params: DeserializeOwned + JsonSchema + Send + 'static {}
impl<T: DeserializeOwned + JsonSchema + Send + 'static> Params for T {}

/// The parameters of a command that takes none.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// What kind of value a parameter takes, as far as reading a request needs to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Number,
    Integer,
    Bool,
    List(Box<Kind>),
    /// An object, or anything else that is read as it comes.
    Other,
}

/// One parameter as `help` shows it.
#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    /// How its value is written: `string`, `number`, `bool`, `[x,y,z]`, `a|b|c`, `[string]`.
    pub form: String,
    pub required: bool,
    /// The field's doc comment.
    pub doc: String,
    pub kind: Kind,
}

/// The parameters of a command, read off its type once.
#[derive(Clone, Debug, Default)]
pub struct Shape {
    pub params: Vec<Param>,
}

impl Shape {
    /// The shape of `P`, from the schema its derive wrote.
    #[must_use]
    pub fn of<P: Params>() -> Self {
        let mut settings = schemars::generate::SchemaSettings::default();
        settings.inline_subschemas = true;
        let schema = settings.into_generator().into_root_schema_for::<P>();
        let schema = serde_json::to_value(schema).unwrap_or(Value::Null);
        Self::from_schema(&schema)
    }

    fn from_schema(schema: &Value) -> Self {
        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let params = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| {
                props
                    .iter()
                    .map(|(name, s)| {
                        let (form, kind) = form_of(s);
                        Param {
                            name: name.clone(),
                            form,
                            required: required.contains(&name.as_str()),
                            doc: doc_of(s),
                            kind,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { params }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }

    /// The parameters as the lines `help` prints: `  name: form (required) - doc`.
    #[must_use]
    pub fn lines(&self) -> String {
        let mut out = String::new();
        if self.params.is_empty() {
            out.push_str("  (no parameters)\n");
        }
        for p in &self.params {
            let _ = write!(out, "  {}: {}", p.name, p.form);
            if p.required {
                out.push_str(" (required)");
            }
            if !p.doc.is_empty() {
                let _ = write!(out, " - {}", p.doc);
            }
            out.push('\n');
        }
        out
    }

    /// Reads a request's arguments into `P`: an unknown parameter is an error that names the
    /// nearest real one, and a value written as text where a number, a flag or a list belongs
    /// is taken for what it says.
    pub fn read<P: Params>(&self, command: &str, mut args: Map<String, Value>) -> Result<P, CmdError> {
        for key in args.keys() {
            if self.get(key).is_none() {
                return Err(self.error(command, self.unknown(command, key)));
            }
        }
        for (key, value) in &mut args {
            if let Some(p) = self.get(key) {
                coerce(value, &p.kind);
            }
        }
        // an explicit null is "not given"
        args.retain(|_, v| !v.is_null());
        serde_json::from_value::<P>(Value::Object(args)).map_err(|e| {
            let text = e.to_string();
            let text = text.split(" at line ").next().unwrap_or(&text).to_owned();
            self.error(command, format!("{command}: {text}"))
        })
    }

    fn unknown(&self, command: &str, key: &str) -> String {
        let near = self
            .params
            .iter()
            .map(|p| (distance(key, &p.name), p.name.as_str()))
            .filter(|(d, name)| *d <= name.len().max(key.len()) / 2)
            .min_by_key(|(d, _)| *d);
        match near {
            Some((_, name)) => format!("{command} has no parameter '{key}' - did you mean '{name}'?"),
            None => format!("{command} has no parameter '{key}'"),
        }
    }

    fn error(&self, command: &str, message: String) -> CmdError {
        CmdError::invalid(message).with_data(json!({ "usage": format!("{command}\n{}", self.lines()) }))
    }
}

fn doc_of(s: &Value) -> String {
    // a doc comment's first paragraph, on one line
    let own = s
        .get("description")
        .and_then(Value::as_str)
        .map(|d| {
            d.split(
                "

",
            )
            .next()
            .unwrap_or(d)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
        })
        .unwrap_or_default();
    if !own.is_empty() {
        return own;
    }
    // a choice whose every value says what it is documents itself: "a: ...; b: ..."
    for key in ["oneOf", "anyOf"] {
        let Some(alts) = s.get(key).and_then(Value::as_array) else { continue };
        let said: Vec<String> = alts
            .iter()
            .filter_map(|a| {
                let value = a.get("const").or_else(|| single_enum(a))?.as_str()?;
                let what = a.get("description").and_then(Value::as_str)?;
                let what = what
                    .split(
                        "

",
                    )
                    .next()
                    .unwrap_or(what)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                Some(format!("{value}: {}", what.trim_end_matches('.')))
            })
            .collect();
        if !said.is_empty()
            && said.len() == alts.iter().filter(|a| a.get("type") != Some(&json!("null"))).count()
        {
            return format!("{}.", said.join("; "));
        }
    }
    String::new()
}

/// How a value of this schema is written, and what reading it has to know.
fn form_of(s: &Value) -> (String, Kind) {
    if let Some(values) = s.get("enum").and_then(Value::as_array) {
        return (choices(values.iter()), Kind::Text);
    }
    if let Some(c) = s.get("const") {
        return (choices(std::iter::once(c)), Kind::Text);
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(alts) = s.get(key).and_then(Value::as_array) {
            let real: Vec<&Value> = alts.iter().filter(|a| a.get("type") != Some(&json!("null"))).collect();
            if real.len() == 1 {
                return form_of(real[0]);
            }
            // an enum whose variants carry their own docs: every alternative is a constant
            let consts: Vec<&Value> =
                real.iter().filter_map(|a| a.get("const").or_else(|| single_enum(a))).collect();
            if !real.is_empty() && consts.len() == real.len() {
                return (choices(consts.into_iter()), Kind::Text);
            }
            let forms: Vec<String> = real.iter().map(|a| form_of(a).0).collect();
            return (forms.join(" or "), Kind::Other);
        }
    }
    let ty = match s.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(a)) => {
            a.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or("object")
        }
        _ => "object",
    };
    match ty {
        "string" => ("string".to_owned(), Kind::Text),
        "number" => ("number".to_owned(), Kind::Number),
        "integer" => ("integer".to_owned(), Kind::Integer),
        "boolean" => ("bool".to_owned(), Kind::Bool),
        "array" => {
            let item = s.get("items").map_or(("value".to_owned(), Kind::Other), form_of);
            let fixed = s
                .get("minItems")
                .and_then(Value::as_u64)
                .filter(|n| s.get("maxItems").and_then(Value::as_u64) == Some(*n));
            let form = match fixed {
                Some(3) if matches!(item.1, Kind::Number | Kind::Integer) => "[x,y,z]".to_owned(),
                Some(n) if n > 0 && n <= 4 => format!("[{}]", vec![item.0.as_str(); n as usize].join(",")),
                _ => format!("[{}]", item.0),
            };
            (form, Kind::List(Box::new(item.1)))
        }
        _ => ("object".to_owned(), Kind::Other),
    }
}

fn single_enum(a: &Value) -> Option<&Value> {
    a.get("enum").and_then(Value::as_array).filter(|v| v.len() == 1).map(|v| &v[0])
}

fn choices<'a>(values: impl Iterator<Item = &'a Value>) -> String {
    values.map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).collect::<Vec<_>>().join("|")
}

/// Takes a value for what it says where its form is not the parameter's: `"5"` for a number,
/// `"true"` or `1` for a flag, `"1,2,3"` for a list, `5` for a text.
fn coerce(value: &mut Value, kind: &Kind) {
    match (kind, &*value) {
        (Kind::Text, Value::Number(n)) => *value = Value::String(n.to_string()),
        (Kind::Text, Value::Bool(b)) => *value = Value::String(b.to_string()),
        (Kind::Number, Value::String(s)) => {
            if let Ok(x) = s.trim().parse::<f64>()
                && let Some(n) = serde_json::Number::from_f64(x)
            {
                *value = Value::Number(n);
            }
        }
        (Kind::Integer, Value::String(s)) => {
            if let Ok(x) = s.trim().parse::<i64>() {
                *value = Value::Number(x.into());
            }
        }
        (Kind::Integer, Value::Number(n)) => {
            // 3.0 where an integer belongs
            if n.as_i64().is_none()
                && let Some(x) = n.as_f64()
                && x.fract() == 0.0
                && x.abs() < 9.0e15
            {
                *value = Value::Number((x as i64).into());
            }
        }
        (Kind::Bool, Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => *value = Value::Bool(true),
            "false" | "0" | "no" | "off" => *value = Value::Bool(false),
            _ => {}
        },
        (Kind::Bool, Value::Number(n)) => *value = Value::Bool(n.as_f64().is_some_and(|x| x != 0.0)),
        (Kind::List(item), Value::String(s)) => {
            let text = s.trim();
            let parsed = text.starts_with('[').then(|| serde_json::from_str::<Value>(text).ok()).flatten();
            let mut list = parsed.and_then(|v| v.as_array().cloned()).unwrap_or_else(|| {
                text.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|p| Value::String(p.to_owned()))
                    .collect()
            });
            for v in &mut list {
                coerce(v, item);
            }
            *value = Value::Array(list);
        }
        (Kind::List(item), Value::Array(_)) => {
            if let Value::Array(list) = value {
                for v in list {
                    coerce(v, item);
                }
            }
        }
        (Kind::Other, Value::String(s)) => {
            let text = s.trim();
            if (text.starts_with('{') || text.starts_with('['))
                && let Ok(v) = serde_json::from_str::<Value>(text)
            {
                *value = v;
            }
        }
        _ => {}
    }
}

/// The edit distance of two short names.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (prev + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}
