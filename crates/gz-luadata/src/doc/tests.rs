use super::*;
use crate::path::{data_eq, path};
use crate::{parse, parse_named};

/// The lines only `a` has and the lines only `b` has, by a longest common subsequence.
fn line_diff(a: &str, b: &str) -> (Vec<String>, Vec<String>) {
    let x: Vec<&str> = a.split_inclusive('\n').collect();
    let y: Vec<&str> = b.split_inclusive('\n').collect();
    let (n, m) = (x.len(), y.len());
    let mut l = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            l[i][j] = if x[i] == y[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let (mut gone, mut came) = (Vec::new(), Vec::new());
    while i < n || j < m {
        if i < n && j < m && x[i] == y[j] {
            i += 1;
            j += 1;
        } else if j < m && (i == n || l[i][j + 1] >= l[i + 1][j]) {
            came.push(y[j].trim_end().to_owned());
            j += 1;
        } else {
            gone.push(x[i].trim_end().to_owned());
            i += 1;
        }
    }
    (gone, came)
}

fn doc(text: &str) -> LuaDoc {
    LuaDoc::parse(text).unwrap_or_else(|e| panic!("{e} at {}:{}", e.line, e.column))
}

fn p(s: &str) -> Vec<Seg> {
    path(s).unwrap()
}

/// Inputs every reader test shares: the forms a declarative file can take.
pub(crate) const SAMPLES: &[&str] = &[
    "-- header\nreturn { zeta = 1, name = \"n\", kind = \"x\", 2, 3 } -- tail",
    "return { a = 0x10, b = 1e2, c = .5, d = -2, e = - -3, f = true, g = nil, goto = 1 }",
    "return { s = \"a\\tb\\\\\\\"\\65\\x41\\u{416}\", l = [[\nline]], q = 'x\"' }",
    "return { a = -\"5\" }",
    "return { a = \"10\" - 3 }",
    "return { a = -0 }",
    "return { a = -\"5\", a = 1 }",
    "return { a = 1, a = 2 }",
    "return { [1] = -\"7\", \"x\" }",
    "return { [2] = \"b\", [1] = \"a\", [-1] = \"z\", [true] = 1 }",
    "return { a = {1}, a = 2 }",
    "return { a = 2, a = {1} }",
    "return { a = 1, a = nil }",
    "return { 1, nil, 3 }",
    "return { [\"key with space\"] = 1, [\"end\"] = 2; x = 3; }",
    "return { a = 5 - 3, b = 0 - 0, c = 1e999 - 1e999 == nil }",
    "return { [-0] = 1, [0] = 2 }",
    "return {\r\n  a = 1,\r\n  b = [[\r\nx\r\ny]],\r\n}\r\n",
    "return { l = [==[\n]]x]=]]==], m = [[\n\n]] }",
    "--[[ long\ncomment ]] return { --[==[ x ]==] a = 1 } --[[ end ]]",
    "return { s = \"a\\z\n    b\", t = \"c\\\nd\" }",
    "return {}",
    "return { {}, { {} }, { a = { b = { c = {} } } } }",
    "return { x = -1, y = - 1, z = -(1) }",
];

#[test]
fn unchanged_documents_are_their_input() {
    for s in SAMPLES {
        let Ok(want) = parse(s) else { continue };
        let d = doc(s);
        assert_eq!(d.text(), *s);
        assert_eq!(d.value(), want, "{s}");
    }
    let bom = "\u{feff}return { a = 1 }\n";
    let d = doc(bom);
    assert_eq!(d.text(), bom);
    assert!(d.has_bom());
    assert_eq!(d.value().get_number("a", 0.0), 1.0);
}

#[test]
fn refusals_read_as_the_reader_writes_them() {
    let cases = [
        "return { a = }",
        "return {\n a = 1\n b = 2 }",
        "return { a = \"abc }",
        "return { a = \"abc\n}",
        "return { a = 1 } x",
        "return { \"a\" \"b\" }",
        "return { a = 1abc }",
        "return { a = b }",
        "local x = 1 return { x }",
        "return { a = 1 + 2 }",
        "",
        "return 1",
        "return {}, {}",
        "return {\n  a = 1,\n  b = -{},\n}",
        "return { a = \"x\" - 3 }",
        "return { a = -nil }",
        "return { [nil] = 1 }",
        "return { a = 1, [-\"x\"] = 2 }",
    ];
    for c in cases {
        let want = parse(c).unwrap_err();
        let got = LuaDoc::parse(c).unwrap_err();
        assert_eq!(got, want, "{c:?}");
        assert!(got.line > 0 && got.column > 0, "{c:?}: no place in {got:?}");
    }
    let nested = |n: usize| format!("return {}{}", "{".repeat(n), "}".repeat(n));
    let e = LuaDoc::parse(&nested(50)).unwrap_err();
    assert_eq!(e.message, "table nesting too deep (self reference?)");
    assert!(e.column > 0);
}

#[test]
fn refusals_have_exact_places() {
    let at = |t: &str| {
        let e = parse(t).unwrap_err();
        (e.line, e.column)
    };
    assert_eq!(at("return { a = }"), (1, 14));
    assert_eq!(at("return {\n a = 1\n b = 2 }"), (3, 2));
    assert_eq!(at("return { a = \"abc }"), (1, 14));
    assert_eq!(at("return { a = b }"), (1, 14));
    assert_eq!(at("return {\n  a = 1,\n  b = -{},\n}"), (3, 7));
    assert_eq!(at("return {\n  title = \"Ночь\", b = c }"), (2, 23));
    assert_eq!(at("return { a = 1 } x"), (1, 18));
    let e = LuaDoc::parse("\u{feff}return {\n  a = b }").unwrap_err();
    assert_eq!((e.line, e.column), (2, 7));
}

/// A generated file: duplicate keys, computed values, explicit numeric keys, nils.
fn generated(seed: u64, n: usize) -> String {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut next = |m: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % m
    };
    let keys = ["a", "b", "c", "kind", "id"];
    let mut out = String::from("return {");
    for _ in 0..n {
        let k = keys[next(keys.len() as u64) as usize];
        let val = match next(9) {
            0 => "1".to_owned(),
            1 => "-\"4\"".to_owned(),
            2 => "{ 1, 2 }".to_owned(),
            3 => "nil".to_owned(),
            4 => "\"s\"".to_owned(),
            5 => "2 - 2".to_owned(),
            6 => "-0".to_owned(),
            7 => "{ x = { y = true } }".to_owned(),
            _ => "- -7".to_owned(),
        };
        match next(4) {
            0 => out.push_str(&format!(" {k} = {val},")),
            1 => out.push_str(&format!(" {val};")),
            2 => out.push_str(&format!(" [{}] = {val},", next(4) as i64 - 1)),
            _ => out.push_str(&format!(" [\"{k}\"] = {val},")),
        }
    }
    out.push_str(" }");
    out
}

#[test]
fn values_agree_with_the_reader() {
    for seed in 0..400 {
        let t = generated(seed, 1 + (seed as usize % 12));
        match parse(&t) {
            Ok(want) => {
                let d = doc(&t);
                assert!(data_eq(&d.value(), &want) && d.value() == want, "{t}\n{:?}\n{want:?}", d.value());
            }
            Err(e) => assert_eq!(LuaDoc::parse(&t).unwrap_err(), e, "{t}"),
        }
    }
}

#[test]
fn paths_get_and_span() {
    let t = "return {\n  nq = 1,\n  nodes = {\n    { id = \"a\", kind = \"k\", params = { n = 2 } },\n    { id = \"b\" },\n  },\n}\n";
    let d = doc(t);
    assert_eq!(d.get(&p("nodes[id=a].params.n")), Some(Value::from(2)));
    assert_eq!(d.get(&p("nodes[1].id")), Some(Value::from("b")));
    assert_eq!(d.get(&p("nodes[id=c]")), None);
    let s = d.span(&p("nodes[id=a].params.n")).unwrap();
    assert_eq!((s.line, s.column, s.end_line, s.end_column), (4, 40, 4, 45));
    let s = d.span(&p("nodes[id=b]")).unwrap();
    assert_eq!((s.line, s.column), (5, 5));
    assert_eq!(d.nearest_span(&p("nodes[id=b].kind")).line, 5);
    assert_eq!(d.head_span().line, 1);
    let keys: Vec<String> = d.keys(&[]).into_iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(keys, ["nq", "nodes"]);
}

const QUEST: &str = "-- A quest the author wrote by hand.
return {
  nq = 1,
  id = \"wolf\",
  title = 'Wolf',   -- single quotes are the author's
  custom = { keep = true },
  nodes = {
    { id = \"start\", kind = \"trigger.start\", out = { next = \"talk\" } },

    -- the talk
    { id = \"talk\", kind = \"dialog.say\",
      params = { text = \"Hello\" },
      out = { next = \"finish\" } },

    { id = \"finish\", kind = \"flow.end\", params = { status = \"completed\" } },
  },
}
";

fn rules() -> DiffRules {
    DiffRules::new()
        .keyed("nodes", "id")
        .node_layout("nodes")
        .order("", &["nq", "beh", "id", "title", "activation", "roles", "vars", "tasks", "nodes"])
        .order(
            "nodes.*",
            &["id", "kind", "once", "params", "cond", "on_enter", "on_exit", "out", "pos", "comment"],
        )
}

/// The model's view of the quest: what a typed reader keeps (no `custom`).
fn model(d: &LuaDoc) -> Value {
    let mut m = d.value();
    m.erase("custom");
    m
}

#[test]
fn edits_touch_only_their_lines() {
    let mut d = doc(QUEST);
    d.set(&p("nodes[id=talk].params.text"), &Value::from("Hi there")).unwrap();
    let (gone, came) = line_diff(QUEST, d.text());
    assert_eq!(gone, ["      params = { text = \"Hello\" },"]);
    assert_eq!(came, ["      params = { text = \"Hi there\" },"]);

    let mut d = doc(QUEST);
    d.set(&p("title"), &Value::from("Old Wolf")).unwrap();
    assert!(d.text().contains("title = 'Old Wolf',   -- single quotes"));

    let mut d = doc(QUEST);
    assert!(d.remove(&p("nodes[id=talk]")).unwrap());
    let (gone, came) = line_diff(QUEST, d.text());
    assert!(came.is_empty(), "{came:?}");
    assert_eq!(gone.len(), 5, "{gone:?}");
    assert!(gone.contains(&"    -- the talk".to_owned()));
    assert!(!d.text().contains("\n\n\n"));

    let mut d = doc(QUEST);
    d.insert(
        &p("nodes[id=start]"),
        Slot::Key { key: "once".into(), after: Some("kind".into()) },
        &Value::Bool(true),
    )
    .unwrap();
    let (gone, came) = line_diff(QUEST, d.text());
    assert_eq!(gone.len(), 1);
    assert_eq!(
        came,
        ["    { id = \"start\", kind = \"trigger.start\", once = true, out = { next = \"talk\" } },"]
    );

    let mut d = doc(QUEST);
    d.insert(
        &p("nodes[id=talk]"),
        Slot::Key { key: "pos".into(), after: None },
        &Value::from(vec![1.into(), 2.into()]),
    )
    .unwrap();
    assert!(d.text().contains("      out = { next = \"finish\" },\n      pos = { 1, 2 } },"), "{}", d.text());

    let mut d = doc(QUEST);
    d.move_item(&p("nodes"), 2, 0).unwrap();
    let ids: Vec<String> =
        d.value().get("nodes").unwrap().arr().iter().map(|n| n.get_string("id", "")).collect();
    assert_eq!(ids, ["finish", "start", "talk"]);
    assert!(d.text().contains("-- the talk"));

    let mut d = doc("return { a = 1, b = 2, c = 3 }");
    d.remove(&p("b")).unwrap();
    assert_eq!(d.text(), "return { a = 1, c = 3 }");
    d.remove(&p("c")).unwrap();
    assert_eq!(d.text(), "return { a = 1 }");
    d.insert(&[], Slot::Key { key: "z".into(), after: None }, &Value::from("x")).unwrap();
    assert_eq!(d.text(), "return { a = 1, z = \"x\" }");
    d.remove(&p("a")).unwrap();
    d.remove(&p("z")).unwrap();
    assert_eq!(d.text(), "return {}");
    d.set(&p("k"), &Value::from(5)).unwrap();
    assert_eq!(d.text(), "return { k = 5 }");
    d.insert(&[], Slot::Head("nq".into()), &Value::from(1)).unwrap();
    assert_eq!(d.text(), "return { nq = 1, k = 5 }");

    let mut d = doc("-- c\nreturn {\n  -- the id\n  id = \"x\",\n}\n");
    d.insert(&[], Slot::Head("nq".into()), &Value::from(1)).unwrap();
    assert_eq!(d.text(), "-- c\nreturn {\n  nq = 1,\n  -- the id\n  id = \"x\",\n}\n");

    let mut d = doc("return {\n\tlist = {\n\t\t\"a\",\n\t\t\"b\"\n\t}\n}\n");
    d.insert(&p("list"), Slot::Position(2), &Value::from("c")).unwrap();
    assert_eq!(d.text(), "return {\n\tlist = {\n\t\t\"a\",\n\t\t\"b\",\n\t\t\"c\"\n\t}\n}\n");
    d.remove(&p("list[2]")).unwrap();
    assert_eq!(d.text(), "return {\n\tlist = {\n\t\t\"a\",\n\t\t\"b\"\n\t}\n}\n");
}

#[test]
fn apply_diff_writes_only_the_difference() {
    let mut d = doc(QUEST);
    let old = model(&d);
    assert!(!d.apply_diff(&old, &old.clone(), &rules()).unwrap());
    assert_eq!(d.text(), QUEST);

    // a changed scalar, a new key in the canonical place, a new node after its linker
    let mut new = old.clone();
    crate::set_at(&mut new, &p("nodes[id=talk].params.text"), Value::from("Bye"));
    crate::set_at(&mut new, &p("activation"), Value::from("auto"));
    crate::set_at(&mut new, &p("nodes[id=talk].out.next"), Value::from("wait"));
    let mut wait = Table::new();
    wait.put("id", "wait");
    wait.put("kind", "wait.time");
    let mut params = Table::new();
    params.put("seconds", 6);
    wait.put("params", params);
    let mut out = Table::new();
    out.put("done", "finish");
    wait.put("out", out);
    let nodes = new.get("nodes").unwrap().clone();
    let mut arr = nodes.arr().to_vec();
    arr.insert(2, Value::Table(wait));
    crate::set_at(&mut new, &p("nodes"), Value::from(arr));
    assert!(d.apply_diff(&old, &new, &rules()).unwrap());
    let text = d.text().to_owned();
    assert!(text.starts_with("-- A quest the author wrote by hand.\n"));
    assert!(text.contains("custom = { keep = true },"), "unknown keys stay");
    assert!(text.contains("-- the talk"));
    assert!(
        text.contains("title = 'Wolf',   -- single quotes are the author's\n  activation = \"auto\",\n"),
        "{text}"
    );
    assert!(
        text.contains("      out = { next = \"wait\" } },\n\n    { id = \"wait\", kind = \"wait.time\", params = { seconds = 6 }, out = { done = \"finish\" } },\n\n    { id = \"finish\""),
        "{text}"
    );
    let (gone, came) = line_diff(QUEST, &text);
    assert_eq!(gone.len(), 2, "{gone:?}");
    assert_eq!(came.len(), 5, "{came:?}");
    let mut back = doc(&text).value();
    back.erase("custom");
    assert!(data_eq(&back, &new));

    // a removed node, a removed key, reordered nodes in the model: the file keeps its order
    let old = new.clone();
    let mut new = old.clone();
    crate::set_at(&mut new, &p("nodes[id=wait]"), Value::Nil);
    crate::set_at(&mut new, &p("activation"), Value::Nil);
    crate::set_at(&mut new, &p("nodes[id=talk].out.next"), Value::from("finish"));
    crate::set_at(&mut new, &p("nodes[id=talk].params.text"), Value::from("Hello"));
    if let Some(Value::Table(t)) = crate::path::get_at_mut(&mut new, &p("nodes")) {
        t.arr.reverse();
    }
    assert!(d.apply_diff(&old, &new, &rules()).unwrap());
    let (gone, came) = line_diff(QUEST, d.text());
    assert_eq!((gone, came), (Vec::<String>::new(), Vec::<String>::new()), "{}", d.text());
    assert_eq!(d.text(), QUEST);
}

#[test]
fn apply_diff_keeps_line_ends_and_header_goes_first() {
    let crlf = "-- c\r\nreturn {\r\n  id = \"x\",\r\n  list = { 1, 2 },\r\n}\r\n";
    let mut d = doc(crlf);
    let old = d.value();
    let mut new = old.clone();
    crate::set_at(&mut new, &p("nq"), Value::from(1));
    crate::set_at(&mut new, &p("list[2]"), Value::from(3));
    crate::set_at(&mut new, &p("extra"), Value::from(vec![Value::from("a\nb")]));
    d.apply_diff(&old, &new, &rules()).unwrap();
    assert_eq!(
        d.text(),
        "-- c\r\nreturn {\r\n  nq = 1,\r\n  id = \"x\",\r\n  list = { 1, 2, 3 },\r\n  extra = {\r\n    [[\r\na\r\nb]],\r\n  },\r\n}\r\n"
    );
    assert!(data_eq(&doc(d.text()).value(), &new));
    assert!(!d.text().replace("\r\n", "").contains('\n'));
}

#[test]
fn rebase_keeps_both_sides() {
    let base_doc = doc(QUEST);
    let base = model(&base_doc);
    // mine: the talk's text
    let mut mine = base.clone();
    crate::set_at(&mut mine, &p("nodes[id=talk].params.text"), Value::from("Mine"));
    // theirs: a comment, an unknown key and another value, written by someone else
    let theirs = QUEST
        .replace("    -- the talk\n", "    -- the talk, said once\n")
        .replace("  custom = { keep = true },\n", "  custom = { keep = true },\n  editor = { zoom = 2 },\n")
        .replace("{ status = \"completed\" }", "{ status = \"failed\" }");
    let (merged, conflicts) = LuaDoc::rebase(&base, &mine, &theirs, &rules()).unwrap();
    assert!(conflicts.is_empty(), "{conflicts:?}");
    let t = merged.text();
    assert!(t.contains("-- the talk, said once"));
    assert!(t.contains("editor = { zoom = 2 },"));
    assert!(t.contains("{ status = \"failed\" }"));
    assert!(t.contains("{ text = \"Mine\" }"));

    // both change the same value: mine is kept and reported
    let theirs = QUEST.replace("{ text = \"Hello\" }", "{ text = \"Theirs\" }");
    let (merged, conflicts) = LuaDoc::rebase(&base, &mine, &theirs, &rules()).unwrap();
    assert_eq!(conflicts.len(), 1);
    let c = &conflicts[0];
    assert_eq!(crate::path::show(&c.path), "nodes[id=talk].params.text");
    assert_eq!(c.mine, Some(Value::from("Mine")));
    assert_eq!(c.theirs, Some(Value::from("Theirs")));
    assert_eq!(c.span.map(|s| s.line), Some(12));
    assert!(merged.text().contains("{ text = \"Mine\" }"));

    // theirs removed the node mine changed: mine brings it back
    let theirs = QUEST.replace(
        "    -- the talk\n    { id = \"talk\", kind = \"dialog.say\",\n      params = { text = \"Hello\" },\n      out = { next = \"finish\" } },\n\n",
        "",
    );
    let (merged, conflicts) = LuaDoc::rebase(&base, &mine, &theirs, &rules()).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].theirs, None);
    assert!(merged.value().get("nodes").unwrap().arr().iter().any(|n| n.get_string("id", "") == "talk"));

    // a text that does not parse is refused with its place
    let e = LuaDoc::rebase(&base, &mine, "return { a = }", &rules()).unwrap_err();
    assert_eq!((e.line, e.column), (1, 14));
}

#[test]
fn parse_named_keeps_the_chunk_name_in_long_names() {
    let name = "x".repeat(70);
    let e = parse_named("return { a = }", &name).unwrap_err();
    assert_eq!(e.message, format!("{}:1: unexpected symbol near '}}'", "x".repeat(59)));
    assert_eq!((e.line, e.column), (1, 14));
}

/// A scene's spawn list: tens of thousands of tables in a few megabytes.
fn big_scene(target: usize) -> String {
    let mut s = String::from("-- a scene\nreturn {\n  scene = 1,\n  objects = {\n");
    let mut i = 0;
    while s.len() < target {
        s.push_str(&format!(
            "    {{ name = \"obj_{i:06}\", ref = \"props\\\\box_wood_{}\", pos = {{ {}.25, 0.5, -{}.75 }}, rot = {{ 0, {}, 0 }}, flags = {{ visible = true }}, custom_data = [[\n[logic]\nactive = walker@{i}\n]] }},\n",
            i % 17,
            i % 1000,
            i % 777,
            i % 360
        ));
        i += 1;
    }
    s.push_str("  },\n}\n");
    s
}

#[test]
fn speed_of_a_large_file() {
    let text = big_scene(5 << 20);
    let t0 = std::time::Instant::now();
    let (mut d, value) = LuaDoc::parse_value(&text, "", Limits::scene()).unwrap();
    let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let mut new = value.clone();
    crate::set_at(&mut new, &p("objects[name=obj_004321].pos[1]"), Value::from(42));
    let rules = DiffRules::new().keyed("objects", "name");
    let t1 = std::time::Instant::now();
    assert!(d.apply_diff(&value, &new, &rules).unwrap());
    let diff_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let n = value.get("objects").map_or(0, |o| o.arr().len());
    println!(
        "{} bytes, {n} objects: parse {parse_ms:.1} ms, one-value apply_diff {diff_ms:.1} ms",
        text.len()
    );
    let (gone, came) = (text.len(), d.text().len());
    assert!(came.abs_diff(gone) < 8);
    // the bounds hold for the release build; a debug build is an order of magnitude slower
    let scale = if cfg!(debug_assertions) { 15.0 } else { 1.0 };
    assert!(parse_ms < 1000.0 * scale, "parse took {parse_ms} ms");
    assert!(diff_ms < 200.0 * scale, "apply_diff took {diff_ms} ms");
}

#[test]
fn a_list_that_does_not_line_up_is_written_whole() {
    // the model read `pos` as a list and skipped an item of `kit` it could not type
    let text = "return {\n  pos = { x = 1, y = 2 },\n  kit = { \"bread\", { count = 2 }, \"vodka\" },\n  -- a note\n  id = \"a\",\n}\n";
    let mut d = doc(text);
    let old = v("return { pos = { 1, 2 }, kit = { \"bread\", \"vodka\" }, id = \"a\" }");
    let new = v("return { pos = { 3, 2 }, kit = { \"bread\", \"vodka\", \"water\" }, id = \"a\" }");
    d.apply_diff(&old, &new, &DiffRules::new()).unwrap();
    assert_eq!(
        d.text(),
        "return {\n  pos = { 3, 2 },\n  kit = { \"bread\", \"vodka\", \"water\" },\n  -- a note\n  id = \"a\",\n}\n"
    );
}

fn v(text: &str) -> Value {
    parse(text).unwrap()
}
