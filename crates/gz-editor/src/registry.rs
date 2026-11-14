use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use futures::FutureExt;
use gz_base::Origin;
use gz_store::CmdError;
use gz_task::panic_message;
use rustc_hash::FxHashMap;
use serde_json::{Map, Value};

use crate::args::Args;
use crate::editor::Editor;
use crate::spec::{Effect, Gate, Params, Shape, Spec, distance};

/// What a command answers: the fields of its reply, or why it did not happen.
pub type Reply = Result<Value, CmdError>;

type BoxFut = Pin<Box<dyn Future<Output = Reply> + Send>>;
type Handler = Box<dyn Fn(Arc<Editor>, Args, Origin) -> BoxFut + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct CommandInfo {
    pub name: &'static str,
    /// The word before the dot of the name; the whole name for a command that has no area.
    pub area: &'static str,
    /// What the command says about itself.
    pub spec: Spec,
}

/// A command's card and its parameters, read off their type the first time somebody asks.
struct Card {
    spec: Spec,
    shape: Arc<OnceLock<Shape>>,
    make: fn() -> Shape,
}

impl Card {
    fn shape(&self) -> &Shape {
        self.shape.get_or_init(self.make)
    }
}

/// An area of commands: the word before the dot, and one line on what it is about.
#[derive(Clone, Copy, Debug)]
pub struct AreaInfo {
    pub name: &'static str,
    pub summary: &'static str,
}

/// A longer text `help` can print: how to work with something, beyond one command.
#[derive(Clone, Copy)]
pub struct Topic {
    pub name: &'static str,
    pub summary: &'static str,
    pub text: fn(&Editor) -> String,
}

/// Every command of the editor by its wire name. Built once at start, then only read.
#[derive(Default)]
pub struct Registry {
    map: FxHashMap<&'static str, Handler>,
    cards: FxHashMap<&'static str, Card>,
    info: Vec<CommandInfo>,
    areas: Vec<AreaInfo>,
    topics: Vec<Topic>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an async command with its card: `spec` says what it is and `P` what it takes.
    /// The request is read into `P` before `f` runs, so a wrong or unknown parameter never
    /// reaches the command. A name registered twice keeps the later handler.
    pub fn cmd<P, F, Fut>(&mut self, spec: Spec, f: F)
    where
        P: Params,
        F: Fn(Arc<Editor>, P, Origin) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Reply> + Send + 'static,
    {
        let name = spec.name;
        let shape = Arc::new(OnceLock::new());
        let make: fn() -> Shape = Shape::of::<P>;
        let cell = shape.clone();
        let handler: Handler =
            Box::new(move |ed, args, origin| match cell.get_or_init(make).read::<P>(name, args) {
                Ok(params) => Box::pin(f(ed, params, origin)),
                Err(e) => Box::pin(async move { Err(e) }),
            });
        if self.map.insert(name, handler).is_none() {
            self.info.push(CommandInfo { name, area: spec.area(), spec });
        }
        self.cards.insert(name, Card { spec, shape, make });
    }

    /// [`Self::cmd`] for a command that answers at once from what is in memory. It must not
    /// touch a file or wait.
    pub fn cmd_sync<P, F>(&mut self, spec: Spec, f: F)
    where
        P: Params,
        F: Fn(&Editor, &P, Origin) -> Reply + Send + Sync + 'static,
    {
        self.cmd(spec, move |ed, params: P, origin| {
            let reply = f(&ed, &params, origin);
            async move { reply }
        });
    }

    /// Says what an area of commands is about. An area nobody described is listed by its name.
    pub fn area(&mut self, name: &'static str, summary: &'static str) {
        match self.areas.iter_mut().find(|a| a.name == name) {
            Some(a) => a.summary = summary,
            None => self.areas.push(AreaInfo { name, summary }),
        }
    }

    /// Adds a text `help` prints under the topic's name.
    pub fn topic(&mut self, topic: Topic) {
        match self.topics.iter_mut().find(|t| t.name == topic.name) {
            Some(t) => *t = topic,
            None => self.topics.push(topic),
        }
    }

    /// The areas somebody described, in registration order.
    #[must_use]
    pub fn areas(&self) -> &[AreaInfo] {
        &self.areas
    }

    #[must_use]
    pub fn topics(&self) -> &[Topic] {
        &self.topics
    }

    /// A command's card and its parameters.
    #[must_use]
    pub fn card(&self, name: &str) -> Option<(Spec, &Shape)> {
        self.cards.get(name).map(|c| (c.spec, c.shape()))
    }

    /// What a call of `name` with `args` does; `None` for a name nobody registered.
    #[must_use]
    pub fn effect_of(&self, name: &str, args: &Args) -> Option<Effect> {
        self.cards.get(name).map(|c| c.spec.effect_of(args))
    }

    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    /// Every command, in registration order.
    #[must_use]
    pub fn commands(&self) -> &[CommandInfo] {
        &self.info
    }

    /// Runs a command and returns the whole reply object: `{"ok":true, ...}` or
    /// `{"ok":false,"error":...}`. A panic inside the command is an error reply.
    pub(crate) async fn call(
        &self,
        editor: Arc<Editor>,
        name: &str,
        args: Args,
        origin: Origin,
        gate: Gate,
    ) -> Value {
        let Some(handler) = self.map.get(name) else {
            return error_reply(&CmdError::not_found(self.unknown(name)));
        };
        if let Some(effect) = self.effect_of(name, &args)
            && let Some(why) = gate.refuse(name, effect)
        {
            return error_reply(&CmdError::new("gate", why));
        }
        match AssertUnwindSafe(handler(editor.clone(), args, origin)).catch_unwind().await {
            Ok(Ok(fields)) => ok_reply(fields),
            Ok(Err(e)) => error_reply(&e),
            Err(payload) => {
                let text = panic_message(&*payload);
                editor.log.error(format!("[mcp] '{name}' stopped on an internal error: {text}"));
                error_reply(&CmdError::internal(format!("internal error in '{name}': {text}")))
            }
        }
    }

    /// "unknown command", with the five nearest of the commands of the same area or with the same verb.
    fn unknown(&self, name: &str) -> String {
        let (area, verb) = name.split_once('.').unwrap_or(("", name));
        let mut near: Vec<(usize, &str)> = self
            .info
            .iter()
            .map(|c| c.name)
            .filter(|n| match n.split_once('.') {
                Some((a, v)) => (!area.is_empty() && a == area) || v == verb,
                None => !verb.is_empty() && n.contains(verb),
            })
            .map(|n| (distance(verb, n.split_once('.').map_or(n, |(_, v)| v)), n))
            .collect();
        near.sort_unstable();
        near.truncate(5);
        if near.is_empty() {
            format!("unknown command '{name}' - help lists the commands")
        } else {
            let names: Vec<&str> = near.into_iter().map(|(_, n)| n).collect();
            format!("unknown command '{name}' - did you mean: {}? help lists the commands", names.join(", "))
        }
    }
}

/// `{"ok":true}` with the reply's fields after it. A reply that is not an object goes under
/// `result`.
#[must_use]
pub fn ok_reply(fields: Value) -> Value {
    let mut out = Map::new();
    out.insert("ok".to_owned(), Value::Bool(true));
    match fields {
        Value::Object(map) => {
            for (k, v) in map {
                if k != "ok" {
                    out.insert(k, v);
                }
            }
        }
        Value::Null => {}
        other => {
            out.insert("result".to_owned(), other);
        }
    }
    Value::Object(out)
}

/// `{"ok":false,"error":<message>,"code":<code>}` with the error's data fields after it.
#[must_use]
pub fn error_reply(e: &CmdError) -> Value {
    let mut out = Map::new();
    out.insert("ok".to_owned(), Value::Bool(false));
    // a report (`CmdError::report`) speaks through its fields alone
    if !(e.code == "report" && e.message.is_empty()) {
        out.insert("error".to_owned(), Value::String(e.message.clone()));
        out.insert("code".to_owned(), Value::String(e.code.to_string()));
    }
    if let Some(Value::Object(data)) = &e.data {
        for (k, v) in data {
            if k != "ok" && k != "error" && k != "code" {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}
