//! The pool of Lua states and one call through it.

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use gz_luadata::Value;
use mlua::chunk::ChunkMode;
use mlua::{Function, HookTriggers, Lua, LuaOptions, MultiValue, RegistryKey, StdLib, Table, VmState};
use parking_lot::Mutex;

use crate::convert::{from_lua, to_lua};

/// What a reader's environment holds: plain Lua, nothing that reaches the machine. The C++
/// editor gave the PDA reader exactly these.
pub const SANDBOX_GLOBALS: [&str; 18] = [
    "string",
    "table",
    "math",
    "pairs",
    "ipairs",
    "type",
    "tostring",
    "tonumber",
    "pcall",
    "select",
    "next",
    "error",
    "unpack",
    "setmetatable",
    "getmetatable",
    "rawget",
    "rawset",
    "assert",
];

/// The message Lua raises when a call runs out of its instruction budget.
const LIMIT_MESSAGE: &str = "execution limit exceeded";

/// How often the hook counts, in instructions.
const HOOK_PERIOD: u32 = 10_000;

/// Scripts a state keeps loaded; an older one is dropped for a newer.
const SCRIPTS_PER_STATE: usize = 4;

/// What one call may cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Instructions one call may run, the script's own load included (the C++ allowed a PDA
    /// read 8,000,000: a big layout needs a few hundred thousand).
    pub instructions: u64,
    /// Bytes a state may hold (best effort: LuaJIT may not let the allocator be replaced).
    pub memory: usize,
    /// How deep a returned value may nest.
    pub depth: usize,
    /// How many values a returned value may hold in all.
    pub values: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { instructions: 8_000_000, memory: 512 << 20, depth: 64, values: 4_000_000 }
    }
}

/// A script of the game: its text and the chunk name its messages carry.
#[derive(Clone, Debug)]
pub struct Script {
    name: String,
    text: Arc<str>,
    rev: u64,
}

impl Script {
    /// `name` is what Lua's messages call it (`xms_pda_layout.script`).
    #[must_use]
    pub fn new(name: impl Into<String>, text: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        let text = text.into();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut h);
        text.hash(&mut h);
        Self { name, text, rev: h.finish() }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    /// A fingerprint of the name and the text: what a cache of the script's results is keyed by.
    #[must_use]
    pub fn rev(&self) -> u64 {
        self.rev
    }
}

/// A text compiled and run in an empty environment; its first result is the argument.
#[derive(Clone, Debug)]
pub struct Chunk {
    /// The chunk name of its messages (`app` gives `app:3: ...`).
    pub name: String,
    pub text: Arc<str>,
}

/// One argument of a call.
#[derive(Clone, Debug)]
pub enum Arg {
    Value(Value),
    Chunk(Chunk),
}

/// Where a call failed. The messages are Lua's own, without a traceback.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LuaError {
    #[error("cannot create a Lua state: {0}")]
    State(String),
    /// The script does not compile.
    #[error("{0}")]
    ScriptCompile(String),
    /// The script's top level raised an error.
    #[error("{0}")]
    ScriptRun(String),
    /// The script defines no such function.
    #[error("the script has no {0}()")]
    NoFunction(String),
    /// A chunk argument does not compile (index from 0).
    #[error("{message}")]
    ArgCompile { index: usize, message: String },
    /// A chunk argument raised an error while it ran.
    #[error("{message}")]
    ArgRun { index: usize, message: String },
    /// The function raised an error.
    #[error("{0}")]
    Call(String),
    /// The function answered something that is no data, or too much of it.
    #[error("{0}")]
    Result(String),
}

/// One Lua state and what it has loaded.
struct Slot {
    lua: Lua,
    /// Instructions left in the running call.
    budget: Arc<AtomicI64>,
    /// Loaded scripts, oldest first: the revision, the text (compared on a hit) and the
    /// environment the script's globals live in.
    scripts: Vec<(u64, Arc<str>, RegistryKey)>,
}

/// The text of an `mlua` error as `lua_tostring` gives it, without the traceback `mlua` adds.
pub(crate) fn message(e: &mlua::Error) -> String {
    match e {
        mlua::Error::RuntimeError(m) => strip_traceback(m),
        mlua::Error::SyntaxError { message, .. } => strip_traceback(message),
        mlua::Error::CallbackError { cause, .. } => message(cause),
        mlua::Error::MemoryError(_) => "not enough memory".to_owned(),
        other => strip_traceback(&other.to_string()),
    }
}

fn strip_traceback(m: &str) -> String {
    m.find("\nstack traceback:").map_or(m, |at| m.get(..at).unwrap_or(m)).to_owned()
}

fn state_error(e: &mlua::Error) -> LuaError {
    LuaError::State(message(e))
}

impl Slot {
    fn new(limits: &Limits) -> Result<Self, LuaError> {
        let lua =
            Lua::new_with(StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::JIT, LuaOptions::default())
                .map_err(|e| state_error(&e))?;
        // a compiled trace runs no hook: the budget must see every instruction
        let jit: Table = lua.globals().get("jit").map_err(|e| state_error(&e))?;
        let off: Function = jit.get("off").map_err(|e| state_error(&e))?;
        off.call::<()>(()).map_err(|e| state_error(&e))?;
        // LuaJIT may keep its own allocator; the limit is then the budget's alone
        let _ = lua.set_memory_limit(limits.memory);
        let budget = Arc::new(AtomicI64::new(0));
        let left = budget.clone();
        lua.set_hook(HookTriggers::new().every_nth_instruction(HOOK_PERIOD), move |_, _| {
            if left.fetch_sub(i64::from(HOOK_PERIOD), Ordering::Relaxed) <= i64::from(HOOK_PERIOD) {
                Err(mlua::Error::runtime(LIMIT_MESSAGE))
            } else {
                Ok(VmState::Continue)
            }
        })
        .map_err(|e| state_error(&e))?;
        Ok(Self { lua, budget, scripts: Vec::new() })
    }

    /// The environment of `script`, loading the script on its first use in this state.
    fn env(&mut self, script: &Script) -> Result<Table, LuaError> {
        if let Some((_, _, key)) =
            self.scripts.iter().find(|(rev, text, _)| *rev == script.rev && **text == *script.text)
        {
            return self.lua.registry_value::<Table>(key).map_err(|e| state_error(&e));
        }
        let lua = &self.lua;
        let env = lua.create_table().map_err(|e| state_error(&e))?;
        let globals = lua.globals();
        for name in SANDBOX_GLOBALS {
            let v: mlua::Value = globals.raw_get(name).map_err(|e| state_error(&e))?;
            env.raw_set(name, v).map_err(|e| state_error(&e))?;
        }
        let f = lua
            .load(&*script.text)
            .set_name(format!("={}", script.name))
            .set_mode(ChunkMode::Text)
            .set_environment(env.clone())
            .into_function()
            .map_err(|e| LuaError::ScriptCompile(message(&e)))?;
        f.call::<()>(()).map_err(|e| LuaError::ScriptRun(message(&e)))?;
        let key = lua.create_registry_value(env.clone()).map_err(|e| state_error(&e))?;
        if self.scripts.len() >= SCRIPTS_PER_STATE {
            let (_, _, old) = self.scripts.remove(0);
            let _ = lua.remove_registry_value(old);
        }
        self.scripts.push((script.rev, script.text.clone(), key));
        Ok(env)
    }

    fn call(
        &mut self,
        script: &Script,
        function: &str,
        args: &[Arg],
        omit: &[&str],
        limits: &Limits,
    ) -> Result<Value, LuaError> {
        self.budget.store(i64::try_from(limits.instructions).unwrap_or(i64::MAX), Ordering::Relaxed);
        let env = self.env(script)?;
        let lua = &self.lua;
        let f = match env.raw_get::<mlua::Value>(function).map_err(|e| state_error(&e))? {
            mlua::Value::Function(f) => f,
            _ => return Err(LuaError::NoFunction(function.to_owned())),
        };
        let mut values = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let v = match arg {
                Arg::Value(v) => to_lua(lua, v, limits.depth).map_err(|e| LuaError::Result(message(&e)))?,
                Arg::Chunk(c) => {
                    let empty = lua.create_table().map_err(|e| state_error(&e))?;
                    let chunk = lua
                        .load(&*c.text)
                        .set_name(format!("={}", c.name))
                        .set_mode(ChunkMode::Text)
                        .set_environment(empty)
                        .into_function()
                        .map_err(|e| LuaError::ArgCompile { index, message: message(&e) })?;
                    let out: MultiValue =
                        chunk.call(()).map_err(|e| LuaError::ArgRun { index, message: message(&e) })?;
                    out.into_iter().next().unwrap_or(mlua::Value::Nil)
                }
            };
            values.push(v);
        }
        let out: MultiValue =
            f.call(MultiValue::from_vec(values)).map_err(|e| LuaError::Call(message(&e)))?;
        let first = out.into_iter().next().unwrap_or(mlua::Value::Nil);
        from_lua(&first, omit, limits).map_err(LuaError::Result)
    }
}

/// The pool of Lua states. `Send + Sync`: callers on any thread share one host; each call has a
/// state to itself.
pub struct LuaHost {
    idle: Mutex<Vec<Slot>>,
    keep: usize,
    limits: Limits,
}

impl Default for LuaHost {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl LuaHost {
    /// A host that keeps as many idle states as the machine has threads.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        let keep = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        Self { idle: Mutex::new(Vec::new()), keep, limits }
    }

    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// States waiting for a call.
    #[must_use]
    pub fn idle_states(&self) -> usize {
        self.idle.lock().len()
    }

    /// Drops every idle state (and the scripts loaded in them).
    pub fn clear(&self) {
        self.idle.lock().clear();
    }

    /// Runs `function` of `script` with `args` and answers its first result. Blocking: call it
    /// on a pool thread.
    pub fn call(&self, script: &Script, function: &str, args: &[Arg]) -> Result<Value, LuaError> {
        self.call_omitting(script, function, args, &[])
    }

    /// [`Self::call`], leaving the keys `omit` of a returned table out.
    pub fn call_omitting(
        &self,
        script: &Script,
        function: &str,
        args: &[Arg],
        omit: &[&str],
    ) -> Result<Value, LuaError> {
        let taken = self.idle.lock().pop();
        let mut slot = match taken {
            Some(s) => s,
            None => Slot::new(&self.limits)?,
        };
        let out = slot.call(script, function, args, omit, &self.limits);
        // a state whose script or function failed may hold anything half done; an argument that
        // did not compile or run left the script alone
        let reusable = matches!(
            out,
            Ok(_) | Err(LuaError::ArgCompile { .. } | LuaError::ArgRun { .. } | LuaError::NoFunction(_))
        );
        if reusable {
            let mut idle = self.idle.lock();
            if idle.len() < self.keep {
                idle.push(slot);
            }
        }
        out
    }
}
