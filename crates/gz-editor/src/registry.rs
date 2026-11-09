use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;

use futures::FutureExt;
use gz_base::Origin;
use gz_store::CmdError;
use gz_task::panic_message;
use rustc_hash::FxHashMap;
use serde_json::{Map, Value};

use crate::args::Args;
use crate::editor::Editor;

/// What a command answers: the fields of its reply, or why it did not happen.
pub type Reply = Result<Value, CmdError>;

type BoxFut = Pin<Box<dyn Future<Output = Reply> + Send>>;
type Handler = Box<dyn Fn(Arc<Editor>, Args, Origin) -> BoxFut + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct CommandInfo {
    pub name: &'static str,
    /// The crate that registered it, for the parity report.
    pub area: &'static str,
}

/// Every command of the editor by its wire name. Built once at start, then only read.
#[derive(Default)]
pub struct Registry {
    map: FxHashMap<&'static str, Handler>,
    info: Vec<CommandInfo>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an async command. A name registered twice keeps the later handler.
    pub fn add<F, Fut>(&mut self, area: &'static str, name: &'static str, f: F)
    where
        F: Fn(Arc<Editor>, Args, Origin) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Reply> + Send + 'static,
    {
        if self.map.insert(name, Box::new(move |ed, args, origin| Box::pin(f(ed, args, origin)))).is_none() {
            self.info.push(CommandInfo { name, area });
        }
    }

    /// Registers a command that answers at once from what is in memory - a read of snapshots.
    /// It must not touch a file or wait.
    pub fn add_sync<F>(&mut self, area: &'static str, name: &'static str, f: F)
    where
        F: Fn(&Editor, &Args, Origin) -> Reply + Send + Sync + 'static,
    {
        self.add(area, name, move |ed, args, origin| {
            let reply = f(&ed, &args, origin);
            async move { reply }
        });
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
    pub(crate) async fn call(&self, editor: Arc<Editor>, name: &str, args: Args, origin: Origin) -> Value {
        let Some(handler) = self.map.get(name) else {
            return error_reply(&CmdError::not_found(format!("unknown command '{name}'")));
        };
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

/// `{"ok":false,"error":<message>}` with the error's data fields after it.
#[must_use]
pub fn error_reply(e: &CmdError) -> Value {
    let mut out = Map::new();
    out.insert("ok".to_owned(), Value::Bool(false));
    out.insert("error".to_owned(), Value::String(e.message.clone()));
    if let Some(Value::Object(data)) = &e.data {
        for (k, v) in data {
            if k != "ok" && k != "error" {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}
