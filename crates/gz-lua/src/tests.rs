use gz_luadata::{Table, Value};

use super::*;

fn script(text: &str) -> Script {
    Script::new("test.script", text)
}

fn host() -> LuaHost {
    LuaHost::new(Limits::default())
}

#[test]
fn the_sandbox_has_no_io_os_or_package() {
    let s = script(
        "function probe()\n\
         return { io = type(io), os = type(os), package = type(package), require = type(require),\n\
                  load = type(load), loadstring = type(loadstring), dofile = type(dofile), debug = type(debug),\n\
                  ffi = type(ffi), jit = type(jit), string = type(string), pairs = type(pairs) }\n\
         end",
    );
    let v = host().call(&s, "probe", &[]).unwrap();
    for name in ["io", "os", "package", "require", "load", "loadstring", "dofile", "debug", "ffi", "jit"] {
        assert_eq!(v.get(name).and_then(Value::as_str), Some("nil"), "{name} is reachable");
    }
    assert_eq!(v.get("string").and_then(Value::as_str), Some("table"));
    assert_eq!(v.get("pairs").and_then(Value::as_str), Some("function"));
}

#[test]
fn calling_os_is_an_error() {
    let s = script("function run() return os.time() end");
    match host().call(&s, "run", &[]) {
        Err(LuaError::Call(m)) => assert!(m.contains("os"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_chunk_runs_in_an_empty_environment() {
    let s = script("function id(x) return x end");
    let chunk = Arg::Chunk(Chunk { name: "asset".into(), text: "return { a = type(string), b = 1 }".into() });
    let v = host().call(&s, "id", &[chunk]).unwrap();
    assert_eq!(v.get("a").and_then(Value::as_str), Some("nil"));
    assert_eq!(v.get("b").and_then(Value::as_number), Some(1.0));
}

#[test]
fn values_cross_both_ways() {
    let s = script("function echo(t) return { n = #t, first = t[1], name = t.name, nested = { t.inner.x } } end");
    let mut inner = Table::new();
    inner.fields.push(("x".into(), Value::Number(5.0)));
    let mut t = Table::new();
    t.arr.push(Value::String("a".into()));
    t.arr.push(Value::String("b".into()));
    t.fields.push(("name".into(), Value::String("ШМЯндекс".into())));
    t.fields.push(("inner".into(), Value::Table(inner)));
    let v = host().call(&s, "echo", &[Arg::Value(Value::Table(t))]).unwrap();
    assert_eq!(v.get("n").and_then(Value::as_number), Some(2.0));
    assert_eq!(v.get("first").and_then(Value::as_str), Some("a"));
    assert_eq!(v.get("name").and_then(Value::as_str), Some("ШМЯндекс"));
    assert_eq!(v.get("nested").map(|n| n.arr().len()), Some(1));
}

#[test]
fn errors_say_where() {
    let h = host();
    assert!(matches!(h.call(&script("function ("), "f", &[]), Err(LuaError::ScriptCompile(_))));
    assert!(matches!(h.call(&script("error('boom')"), "f", &[]), Err(LuaError::ScriptRun(m)) if m.contains("boom")));
    assert!(matches!(h.call(&script("x = 1"), "f", &[]), Err(LuaError::NoFunction(f)) if f == "f"));
    let s = script("function f(x) return x end");
    let bad = Arg::Chunk(Chunk { name: "app".into(), text: "return {".into() });
    assert!(matches!(h.call(&s, "f", &[bad]), Err(LuaError::ArgCompile { index: 0, message }) if message.starts_with("app:1:")));
    let runs = Arg::Chunk(Chunk { name: "app".into(), text: "return nothing.here".into() });
    assert!(matches!(h.call(&s, "f", &[runs]), Err(LuaError::ArgRun { index: 0, .. })));
    let bytecode = Arg::Chunk(Chunk { name: "app".into(), text: "\u{1b}LJ".into() });
    assert!(matches!(h.call(&s, "f", &[bytecode]), Err(LuaError::ArgCompile { .. })));
}

#[test]
fn the_instruction_limit_stops_a_loop() {
    let h = LuaHost::new(Limits { instructions: 200_000, ..Limits::default() });
    let s = script("function spin() while true do end end");
    match h.call(&s, "spin", &[]) {
        Err(LuaError::Call(m)) => assert_eq!(m, "execution limit exceeded"),
        other => panic!("{other:?}"),
    }
    // the next call has a budget of its own
    let ok = script("function one() return 1 end");
    assert_eq!(h.call(&ok, "one", &[]).unwrap().as_number(), Some(1.0));
}

#[test]
fn a_state_keeps_its_script_and_omits_keys() {
    let h = host();
    let s = script("calls = 0\nfunction count() calls = calls + 1 return { calls = calls, state = { big = true } } end");
    let first = h.call_omitting(&s, "count", &[], &["state"]).unwrap();
    assert!(first.get("state").is_none());
    assert_eq!(h.idle_states(), 1);
    let second = h.call(&s, "count", &[]).unwrap();
    assert_eq!(second.get("calls").and_then(Value::as_number), Some(2.0));
    // another text is another script
    let other = script("calls = 0\nfunction count() calls = calls + 1 return { calls = calls } end ");
    assert_eq!(h.call(&other, "count", &[]).unwrap().get("calls").and_then(Value::as_number), Some(1.0));
}

#[test]
fn the_editors_comments_become_blanks() {
    assert!(matches!(vanilla_source("return { a = 1 } -- plain"), std::borrow::Cow::Borrowed(_)));
    assert_eq!(vanilla_source("return { a = 1, // note\n b = 2 }"), "return { a = 1,  \n b = 2 }");
    assert_eq!(vanilla_source("return { /* x\ny */ a = 1 }"), "return {   a = 1 }");
    // a string or a Lua comment keeps its slashes
    assert_eq!(vanilla_source("return { u = \"http://x\", -- a // b\n }"), "return { u = \"http://x\", -- a // b\n }");
    assert_eq!(vanilla_source("return { s = [[/* not */]] }"), "return { s = [[/* not */]] }");
    // the fork's '*' swallows the character after it: '**/' does not close
    assert_eq!(vanilla_source("a /* **/ b */ c"), "a   c");
}
