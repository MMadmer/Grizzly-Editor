//! The declarations of scripts: what each script (a namespace, by its file's name) declares at
//! its top - functions and values - and the globals `_g.script` gives every script.
//!
//! A declaration is a statement at the top of the file, outside every function body and block:
//! `function name(...)`, `function <script>.name(...)`, `name = function(...)`, `name = <value>`,
//! and a class (`class "name" (base)`, its parameters those of its `__init`). What is `local` is
//! the file's own and no declaration.

use std::ops::Range;

use gz_base::RelPath;
use rustc_hash::FxHashMap;

use crate::lex::{self, KEYWORDS, Tk, Token};
use crate::symbols::{declared_name, params_at};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind {
    Function,
    Value,
}

/// Where a declaration is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclSource {
    /// A script of the linked game, by its path there (`scripts\xr_logic.script`).
    Game(String),
    /// A script of the project.
    Project(RelPath),
}

impl DeclSource {
    /// `game` or `project`: the word answers use.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Self::Game(_) => "game",
            Self::Project(_) => "project",
        }
    }

    /// The path: the game's (`scripts\xr_logic.script`) or the project's.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Game(p) => p,
            Self::Project(p) => p.as_str(),
        }
    }
}

/// A name a script declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    /// The script's namespace (its file's name); empty for a global (`_g.script`'s).
    pub script: String,
    pub name: String,
    pub kind: DeclKind,
    /// A function's parameters as written: `(npc, ini, section)`.
    pub params: String,
    /// 1-based.
    pub line: u32,
    pub source: DeclSource,
}

/// What a completion offer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferKind {
    Keyword,
    Function,
    Value,
    /// A script: what comes after its dot.
    Module,
}

/// A completion offer: what it writes, what it says beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub text: String,
    pub detail: String,
    pub kind: OfferKind,
}

/// One script of an index: its namespace (lower case), where it is, its declarations.
#[derive(Clone, Debug)]
pub struct ScriptDecls {
    /// The file's name without its extension, lower case: `xr_logic`, `_g`.
    pub script: String,
    pub source: DeclSource,
    pub decls: Vec<Decl>,
}

#[derive(Clone, Debug)]
struct Entry {
    script: String,
    source: DeclSource,
    decls: Range<usize>,
}

/// The namespace of the globals.
const GLOBALS: &str = "_g";

/// The declarations of every script the index was made of.
#[derive(Clone, Debug, Default)]
pub struct ScriptIndex {
    decls: Vec<Decl>,
    /// By namespace, in the order the scripts came.
    scripts: Vec<Entry>,
    by_name: FxHashMap<String, usize>,
}

/// Lua's own functions, with their parameters.
const LUA_GLOBALS: &[(&str, &str)] = &[
    ("assert", "(v [, message])"),
    ("collectgarbage", "([opt [, arg]])"),
    ("dofile", "([filename])"),
    ("error", "(message [, level])"),
    ("getfenv", "([f])"),
    ("getmetatable", "(object)"),
    ("ipairs", "(t)"),
    ("load", "(func [, chunkname])"),
    ("loadfile", "([filename])"),
    ("loadstring", "(string [, chunkname])"),
    ("module", "(name [, ...])"),
    ("next", "(table [, index])"),
    ("pairs", "(t)"),
    ("pcall", "(f, ...)"),
    ("print", "(...)"),
    ("rawequal", "(v1, v2)"),
    ("rawget", "(table, index)"),
    ("rawset", "(table, index, value)"),
    ("require", "(modname)"),
    ("select", "(index, ...)"),
    ("setfenv", "(f, table)"),
    ("setmetatable", "(table, metatable)"),
    ("tonumber", "(e [, base])"),
    ("tostring", "(e)"),
    ("type", "(v)"),
    ("unpack", "(list [, i [, j]])"),
    ("xpcall", "(f, err)"),
];

/// The game's own names that every script reads, bound by the engine: functions, and the
/// tables (`level.object_by_id`) marked by `true`.
const ENGINE_GLOBALS: &[(&str, bool)] = &[
    ("actor_stats", true),
    ("alife", false),
    ("device", false),
    ("game", true),
    ("game_ini", false),
    ("get_console", false),
    ("get_hud", false),
    ("level", true),
    ("relation_registry", true),
    ("system_ini", false),
    ("time_global", false),
];

/// The standard library: Lua 5.1's and LuaJIT's `bit` and `jit`, each member with its
/// parameters (empty for a value).
const LIBRARY: &[(&str, &[(&str, &str)])] = &[
    (
        "bit",
        &[
            ("arshift", "(x, n)"),
            ("band", "(x1 [, x2 ...])"),
            ("bnot", "(x)"),
            ("bor", "(x1 [, x2 ...])"),
            ("bswap", "(x)"),
            ("bxor", "(x1 [, x2 ...])"),
            ("lshift", "(x, n)"),
            ("rol", "(x, n)"),
            ("ror", "(x, n)"),
            ("rshift", "(x, n)"),
            ("tobit", "(x)"),
            ("tohex", "(x [, n])"),
        ],
    ),
    (
        "coroutine",
        &[
            ("create", "(f)"),
            ("resume", "(co [, val1, ...])"),
            ("running", "()"),
            ("status", "(co)"),
            ("wrap", "(f)"),
            ("yield", "(...)"),
        ],
    ),
    (
        "debug",
        &[
            ("debug", "()"),
            ("getfenv", "(o)"),
            ("gethook", "([thread])"),
            ("getinfo", "([thread,] function [, what])"),
            ("getlocal", "([thread,] level, local)"),
            ("getmetatable", "(object)"),
            ("getregistry", "()"),
            ("getupvalue", "(func, up)"),
            ("setfenv", "(object, table)"),
            ("sethook", "([thread,] hook, mask [, count])"),
            ("setlocal", "([thread,] level, local, value)"),
            ("setmetatable", "(object, table)"),
            ("setupvalue", "(func, up, value)"),
            ("traceback", "([thread,] [message [, level]])"),
            ("upvalueid", "(f, n)"),
            ("upvaluejoin", "(f1, n1, f2, n2)"),
        ],
    ),
    (
        "io",
        &[
            ("close", "([file])"),
            ("flush", "()"),
            ("input", "([file])"),
            ("lines", "([filename])"),
            ("open", "(filename [, mode])"),
            ("output", "([file])"),
            ("popen", "(prog [, mode])"),
            ("read", "(...)"),
            ("stderr", ""),
            ("stdin", ""),
            ("stdout", ""),
            ("tmpfile", "()"),
            ("type", "(obj)"),
            ("write", "(...)"),
        ],
    ),
    (
        "jit",
        &[
            ("arch", ""),
            ("flush", "([func])"),
            ("off", "([func])"),
            ("on", "([func])"),
            ("os", ""),
            ("status", "()"),
            ("version", ""),
            ("version_num", ""),
        ],
    ),
    (
        "math",
        &[
            ("abs", "(x)"),
            ("acos", "(x)"),
            ("asin", "(x)"),
            ("atan", "(x)"),
            ("atan2", "(y, x)"),
            ("ceil", "(x)"),
            ("cos", "(x)"),
            ("cosh", "(x)"),
            ("deg", "(x)"),
            ("exp", "(x)"),
            ("floor", "(x)"),
            ("fmod", "(x, y)"),
            ("frexp", "(x)"),
            ("huge", ""),
            ("ldexp", "(m, e)"),
            ("log", "(x)"),
            ("log10", "(x)"),
            ("max", "(x, ...)"),
            ("min", "(x, ...)"),
            ("modf", "(x)"),
            ("pi", ""),
            ("pow", "(x, y)"),
            ("rad", "(x)"),
            ("random", "([m [, n]])"),
            ("randomseed", "(x)"),
            ("sin", "(x)"),
            ("sinh", "(x)"),
            ("sqrt", "(x)"),
            ("tan", "(x)"),
            ("tanh", "(x)"),
        ],
    ),
    (
        "os",
        &[
            ("clock", "()"),
            ("date", "([format [, time]])"),
            ("difftime", "(t2, t1)"),
            ("execute", "([command])"),
            ("exit", "([code])"),
            ("getenv", "(varname)"),
            ("remove", "(filename)"),
            ("rename", "(oldname, newname)"),
            ("setlocale", "(locale [, category])"),
            ("time", "([table])"),
            ("tmpname", "()"),
        ],
    ),
    (
        "string",
        &[
            ("byte", "(s [, i [, j]])"),
            ("char", "(...)"),
            ("dump", "(function)"),
            ("find", "(s, pattern [, init [, plain]])"),
            ("format", "(formatstring, ...)"),
            ("gmatch", "(s, pattern)"),
            ("gsub", "(s, pattern, repl [, n])"),
            ("len", "(s)"),
            ("lower", "(s)"),
            ("match", "(s, pattern [, init])"),
            ("rep", "(s, n)"),
            ("reverse", "(s)"),
            ("sub", "(s, i [, j])"),
            ("upper", "(s)"),
        ],
    ),
    (
        "table",
        &[
            ("concat", "(table [, sep [, i [, j]]])"),
            ("foreach", "(table, f)"),
            ("foreachi", "(table, f)"),
            ("getn", "(table)"),
            ("insert", "(table, [pos,] value)"),
            ("maxn", "(table)"),
            ("remove", "(table [, pos])"),
            ("sort", "(table [, comp])"),
        ],
    ),
];

/// How well an offer's text matches what was typed: the case as typed at its start, the start
/// in any case, anywhere in any case; `None` when it does not.
fn rank(text: &str, prefix: &str, lower_prefix: &str) -> Option<u8> {
    if text.starts_with(prefix) {
        Some(0)
    } else {
        let lower = text.to_ascii_lowercase();
        if lower.starts_with(lower_prefix) {
            Some(1)
        } else if lower.contains(lower_prefix) {
            Some(2)
        } else {
            None
        }
    }
}

fn detail_of(d: &Decl) -> String {
    match d.kind {
        DeclKind::Function => d.params.clone(),
        DeclKind::Value => "value".to_owned(),
    }
}

impl ScriptIndex {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// An index of these declarations (a project's script comes after the game's of the same
    /// name, and wins).
    #[must_use]
    pub fn of(decls: Vec<Decl>) -> Self {
        let mut scripts: Vec<ScriptDecls> = Vec::new();
        for d in decls {
            let ns = if d.script.is_empty() { GLOBALS.to_owned() } else { d.script.to_ascii_lowercase() };
            match scripts.iter_mut().rev().find(|s| s.script == ns && s.source == d.source) {
                Some(s) => s.decls.push(d),
                None => scripts.push(ScriptDecls { script: ns, source: d.source.clone(), decls: vec![d] }),
            }
        }
        Self::of_scripts(scripts)
    }

    /// An index of whole scripts, in order: a script of a namespace already there replaces the
    /// earlier one whole, as a module's script replaces the game's in the game.
    #[must_use]
    pub fn of_scripts(scripts: Vec<ScriptDecls>) -> Self {
        let mut last: FxHashMap<String, usize> = FxHashMap::default();
        for (n, s) in scripts.iter().enumerate() {
            last.insert(s.script.to_ascii_lowercase(), n);
        }
        let mut out = Self::default();
        for (n, s) in scripts.into_iter().enumerate() {
            let key = s.script.to_ascii_lowercase();
            if last.get(&key) != Some(&n) {
                continue;
            }
            let from = out.decls.len();
            out.decls.extend(s.decls);
            out.by_name.insert(key.clone(), out.scripts.len());
            out.scripts.push(Entry { script: key, source: s.source, decls: from..out.decls.len() });
        }
        out
    }

    fn entry(&self, script: &str) -> Option<&Entry> {
        self.by_name.get(&script.to_ascii_lowercase()).and_then(|&i| self.scripts.get(i))
    }

    fn decls_of(&self, e: &Entry) -> &[Decl] {
        self.decls.get(e.decls.clone()).unwrap_or(&[])
    }

    /// Where a script is: its source.
    #[must_use]
    pub fn source_of(&self, script: &str) -> Option<&DeclSource> {
        self.entry(script).map(|e| &e.source)
    }

    /// Where a name is declared: `xr_logic.pick_section_from_condlist` (a script's), `printf` (a
    /// global), `xr_logic` (the script itself, at its first line). The project's first.
    #[must_use]
    pub fn lookup(&self, dotted: &str) -> Vec<Decl> {
        let dotted = dotted.trim();
        if let Some(at) = dotted.find(['.', ':']) {
            let (ns, rest) = dotted.split_at(at);
            let member = rest.get(1..).unwrap_or("");
            let member = member.split(['.', ':']).next().unwrap_or("");
            return self
                .entry(ns)
                .map(|e| self.decls_of(e).iter().filter(|d| d.name == member).cloned().collect())
                .unwrap_or_default();
        }
        if dotted.is_empty() {
            return Vec::new();
        }
        let globals: Vec<Decl> = self
            .entry(GLOBALS)
            .map(|e| self.decls_of(e).iter().filter(|d| d.name == dotted).cloned().collect())
            .unwrap_or_default();
        if !globals.is_empty() {
            return globals;
        }
        self.entry(dotted)
            .map(|e| {
                vec![Decl {
                    script: e.script.clone(),
                    name: e.script.clone(),
                    kind: DeclKind::Value,
                    params: String::new(),
                    line: 1,
                    source: e.source.clone(),
                }]
            })
            .unwrap_or_default()
    }

    /// What `prefix` completes to: after `qualifier.` the script's names, else the scripts, the
    /// globals, Lua's words and its standard library; ranked, at most `limit`.
    #[must_use]
    pub fn complete(&self, qualifier: Option<&str>, prefix: &str, limit: usize) -> Vec<Offer> {
        let lower_prefix = prefix.to_ascii_lowercase();
        let mut found: Vec<(u8, Offer)> = Vec::new();
        let mut offer = |text: &str, detail: &str, kind: OfferKind| {
            if let Some(r) = rank(text, prefix, &lower_prefix) {
                found.push((r, Offer { text: text.to_owned(), detail: detail.to_owned(), kind }));
            }
        };
        match qualifier.map(str::trim).filter(|q| !q.is_empty()) {
            Some(q) => {
                if let Some((_, members)) = LIBRARY.iter().find(|(lib, _)| *lib == q) {
                    for (name, params) in *members {
                        let (detail, kind) = if params.is_empty() {
                            ("value", OfferKind::Value)
                        } else {
                            (*params, OfferKind::Function)
                        };
                        offer(name, detail, kind);
                    }
                } else if let Some(e) = self.entry(q) {
                    for d in self.decls_of(e) {
                        let kind =
                            if d.kind == DeclKind::Function { OfferKind::Function } else { OfferKind::Value };
                        offer(&d.name, &detail_of(d), kind);
                    }
                }
            }
            None => {
                for k in KEYWORDS {
                    offer(k, "keyword", OfferKind::Keyword);
                }
                for (name, params) in LUA_GLOBALS {
                    offer(name, params, OfferKind::Function);
                }
                for (lib, _) in LIBRARY {
                    offer(lib, "library", OfferKind::Module);
                }
                for (name, table) in ENGINE_GLOBALS {
                    offer(name, "engine", if *table { OfferKind::Module } else { OfferKind::Function });
                }
                for e in &self.scripts {
                    if e.script == GLOBALS {
                        for d in self.decls_of(e) {
                            let kind = if d.kind == DeclKind::Function {
                                OfferKind::Function
                            } else {
                                OfferKind::Value
                            };
                            offer(&d.name, &detail_of(d), kind);
                        }
                    } else {
                        offer(&e.script, "script", OfferKind::Module);
                    }
                }
            }
        }
        found.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb).then(a.text.len().cmp(&b.text.len())).then(a.text.cmp(&b.text))
        });
        let mut out: Vec<Offer> = Vec::with_capacity(limit.min(found.len()));
        for (_, o) in found {
            if out.len() >= limit {
                break;
            }
            if !out.iter().any(|x| x.text == o.text) {
                out.push(o);
            }
        }
        out
    }

    /// How many scripts it holds.
    #[must_use]
    pub fn scripts(&self) -> usize {
        self.scripts.len()
    }

    #[must_use]
    pub fn decls(&self) -> &[Decl] {
        &self.decls
    }
}

/// The targets of an assignment whose `=` is at `eq`: the names before it (`a, b =`), each with
/// its line - `None` when the statement is `local` or a `for`.
fn targets<'a>(tokens: &[Token<'a>], eq: usize, own: &str) -> Option<Vec<(&'a str, u32)>> {
    let mut out = Vec::new();
    let mut j = eq;
    loop {
        let last = tokens.get(j.checked_sub(1)?)?;
        if last.kind != Tk::Name {
            return None;
        }
        // `name` or `<script>.name`
        let before = j.checked_sub(2).and_then(|i| tokens.get(i));
        let dotted = before.is_some_and(|t| t.is(".") || t.is(":"));
        let mut start = j - 1;
        if dotted {
            let ns = tokens.get(j.checked_sub(3)?)?;
            let chained =
                j.checked_sub(4).and_then(|i| tokens.get(i)).is_some_and(|t| t.is(".") || t.is(":"));
            if ns.kind == Tk::Name
                && ns.text.eq_ignore_ascii_case(own)
                && !chained
                && before.is_some_and(|t| t.is("."))
            {
                out.push((last.text, last.line));
            }
            start = j - 3;
        } else {
            out.push((last.text, last.line));
        }
        match start.checked_sub(1).and_then(|i| tokens.get(i)) {
            Some(t) if t.is(",") => j = start - 1,
            Some(t) if t.is("local") || t.is("for") => return None,
            _ => return Some(out),
        }
    }
}

/// The declarations of one script's text.
#[must_use]
pub fn declarations(script: &str, text: &str, source: &DeclSource) -> Vec<Decl> {
    let own = script.to_ascii_lowercase();
    let ns = if own == GLOBALS { String::new() } else { own.clone() };
    let tokens = lex::tokens(text);
    let mut out: Vec<Decl> = Vec::new();
    let mut classes: FxHashMap<&str, usize> = FxHashMap::default();
    let decl = |name: &str, kind: DeclKind, params: String, line: u32| Decl {
        script: ns.clone(),
        name: name.to_owned(),
        kind,
        params,
        line,
        source: source.clone(),
    };
    let (mut depth, mut brackets) = (0usize, 0usize);
    for (i, t) in tokens.iter().enumerate() {
        let top = depth == 0 && brackets == 0;
        match t.kind {
            Tk::Keyword => match t.text {
                "function" => {
                    let local = i.checked_sub(1).and_then(|p| tokens.get(p)).is_some_and(|p| p.is("local"));
                    let (parts, after, line) = declared_name(&tokens, i);
                    if top && !local && tokens.get(after).is_some_and(|t| t.is("(")) {
                        let (params, _) = params_at(&tokens, after);
                        match parts.as_slice() {
                            [name] => out.push(decl(name, DeclKind::Function, params, line)),
                            [prefix, ".", name] if prefix.eq_ignore_ascii_case(&own) => {
                                out.push(decl(name, DeclKind::Function, params, line));
                            }
                            [class, ":", "__init"] => {
                                if let Some(d) = classes.get(class).and_then(|&n| out.get_mut(n)) {
                                    d.params = params;
                                }
                            }
                            _ => {}
                        }
                    }
                    depth += 1;
                }
                "do" | "if" | "repeat" => depth += 1,
                "end" | "until" => depth = depth.saturating_sub(1),
                _ => {}
            },
            Tk::Punct => match t.text {
                "(" | "{" | "[" => brackets += 1,
                ")" | "}" | "]" => brackets = brackets.saturating_sub(1),
                "=" if top => {
                    let Some(names) = targets(&tokens, i, &own) else { continue };
                    let value = tokens.get(i + 1);
                    let function = names.len() == 1
                        && value.is_some_and(|v| v.is("function"))
                        && tokens.get(i + 2).is_some_and(|p| p.is("("));
                    // a list of targets reads right to left
                    for (name, line) in names.into_iter().rev() {
                        if function {
                            let (params, _) = params_at(&tokens, i + 2);
                            out.push(decl(name, DeclKind::Function, params, line));
                        } else {
                            out.push(decl(name, DeclKind::Value, String::new(), line));
                        }
                    }
                }
                _ => {}
            },
            // `class "name" (base)` and `class("name")`
            Tk::Name if top && t.text == "class" => {
                let next = tokens.get(i + 1);
                let name = match next {
                    Some(n) if n.kind == Tk::Str => Some(n),
                    Some(n) if n.is("(") => tokens.get(i + 2).filter(|n| n.kind == Tk::Str),
                    _ => None,
                };
                if let Some(n) = name {
                    let body = n.string_body();
                    if !body.is_empty() {
                        classes.insert(body, out.len());
                        out.push(decl(body, DeclKind::Function, "()".to_owned(), n.line));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn game(path: &str) -> DeclSource {
        DeclSource::Game(path.to_owned())
    }

    fn names(decls: &[Decl]) -> Vec<(&str, DeclKind, &str, u32)> {
        decls.iter().map(|d| (d.name.as_str(), d.kind, d.params.as_str(), d.line)).collect()
    }

    const SCRIPT: &str = "\
local helper = 1
local function private() end
function pick(list, n)
  local inner = function() end
  if list then
    nested = 2
  end
  for i = 1, 3 do
    looped = i
  end
end
my_script.member = function(a) end
my_script.other = 5
other.field = 1
LIMIT, COUNT = 10, 20
handler = function(self, ...) end
tbl = { key = 1, fn = function() end }
repeat x = 1 until true
class \"binder\" (object_binder)
function binder:__init(obj) end
function binder:update(delta) end
function my_script.dotted(z) end
";

    #[test]
    fn a_scripts_top_level_declarations() {
        let src = game("scripts\\my_script.script");
        let d = declarations("my_script", SCRIPT, &src);
        assert_eq!(
            names(&d),
            [
                ("pick", DeclKind::Function, "(list, n)", 3),
                ("member", DeclKind::Function, "(a)", 12),
                ("other", DeclKind::Value, "", 13),
                ("LIMIT", DeclKind::Value, "", 15),
                ("COUNT", DeclKind::Value, "", 15),
                ("handler", DeclKind::Function, "(self, ...)", 16),
                ("tbl", DeclKind::Value, "", 17),
                ("binder", DeclKind::Function, "(obj)", 19),
                ("dotted", DeclKind::Function, "(z)", 22),
            ]
        );
        assert!(d.iter().all(|d| d.script == "my_script" && d.source == src));
        // the globals have no script
        let g = declarations("_G", "function printf(fmt, ...) end\n", &game("scripts\\_g.script"));
        assert_eq!(g[0].script, "");
    }

    fn index() -> ScriptIndex {
        let g = game("scripts\\_g.script");
        let logic = game("scripts\\xr_logic.script");
        let mine = DeclSource::Project(RelPath::new("gamedata/scripts/xr_logic.script").unwrap());
        ScriptIndex::of_scripts(vec![
            ScriptDecls {
                script: "_g".into(),
                source: g.clone(),
                decls: declarations("_g", "function printf(fmt, ...) end\nfunction prefetch(x) end\n", &g),
            },
            ScriptDecls {
                script: "xr_logic".into(),
                source: logic.clone(),
                decls: declarations(
                    "xr_logic",
                    "function game_only() end\nfunction pick_section(a) end\n",
                    &logic,
                ),
            },
            ScriptDecls {
                script: "xr_effects".into(),
                source: game("scripts\\xr_effects.script"),
                decls: Vec::new(),
            },
            ScriptDecls {
                script: "xr_logic".into(),
                source: mine.clone(),
                decls: declarations("xr_logic", "function pick_section(a, b) end\nPRINT = 1\n", &mine),
            },
        ])
    }

    #[test]
    fn a_projects_script_replaces_the_games_whole() {
        let i = index();
        assert_eq!(i.scripts(), 3);
        let found = i.lookup("xr_logic.pick_section");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].params.as_str(), found[0].source.word()), ("(a, b)", "project"));
        assert!(i.lookup("xr_logic.game_only").is_empty());
        assert_eq!(i.lookup("xr_logic:pick_section").len(), 1);
        assert_eq!(i.lookup("XR_LOGIC.pick_section").len(), 1);
        // a global, then a script itself
        assert_eq!(i.lookup("printf")[0].source.path(), "scripts\\_g.script");
        let module = i.lookup("xr_effects");
        assert_eq!(
            (module[0].name.as_str(), module[0].line, module[0].kind),
            ("xr_effects", 1, DeclKind::Value)
        );
        assert!(i.lookup("nobody").is_empty() && i.lookup("").is_empty());
        // the same through `of`, from the declarations alone
        let again = ScriptIndex::of(i.decls().to_vec());
        assert_eq!(again.scripts(), 2);
        assert_eq!(again.lookup("xr_logic.pick_section")[0].params, "(a, b)");
    }

    #[test]
    fn completion_is_ranked() {
        let i = index();
        let texts = |q: Option<&str>, p: &str| -> Vec<String> {
            i.complete(q, p, 50).into_iter().map(|o| o.text).collect()
        };
        assert_eq!(texts(Some("xr_logic"), ""), ["PRINT", "pick_section"]);
        // the case as typed first, then any case, then anywhere; shorter first
        assert_eq!(texts(Some("xr_logic"), "p"), ["pick_section", "PRINT"]);
        let all = i.complete(None, "pr", 50);
        let words: Vec<&str> = all.iter().map(|o| o.text.as_str()).collect();
        assert_eq!(words, ["print", "printf", "prefetch"]);
        assert_eq!(all[1].detail, "(fmt, ...)");
        assert_eq!(texts(None, "xr_"), ["xr_logic", "xr_effects"]);
        let m = &i.complete(None, "xr_logic", 5)[0];
        assert_eq!((m.kind, m.detail.as_str()), (OfferKind::Module, "script"));
        assert_eq!(i.complete(None, "funct", 5)[0].kind, OfferKind::Keyword);
        assert_eq!(texts(Some("string"), "fo"), ["format"]);
        assert_eq!(texts(Some("bit"), "bx"), ["bxor"]);
        let lib = &i.complete(Some("math"), "pi", 5)[0];
        assert_eq!((lib.kind, lib.detail.as_str()), (OfferKind::Value, "value"));
        let e = &i.complete(None, "time_gl", 5)[0];
        assert_eq!((e.text.as_str(), e.detail.as_str()), ("time_global", "engine"));
        assert_eq!(i.complete(None, "", 3).len(), 3);
        // substring matches come last
        assert_eq!(texts(None, "logic").first().map(String::as_str), Some("xr_logic"));
        assert!(i.complete(Some("nobody"), "", 10).is_empty());
    }
}
