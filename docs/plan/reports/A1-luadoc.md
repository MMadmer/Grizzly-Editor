# A1 milestone: `LuaDoc` is usable

`gz_luadata::LuaDoc` (`crates/gz-luadata/src/doc/`): a declarative Lua file as a lossless document. Tests:
`bash tools/rust/cg.sh test -p gz-luadata` (byte identity, parity with `parse_named`, edits, diffs, rebase, speed).

```rust
LuaDoc::parse(text) -> Result<LuaDoc, LuaDataError>             // Limits::asset(): the game's reader judges first
LuaDoc::parse_with(text, chunk_name, Limits::scene())            // editor-only files of megabytes (scene data)
LuaDoc::parse_value(text, name, limits) -> (LuaDoc, Value)       // both, read once
doc.text() / into_text()      // unchanged: the input byte for byte (CRLF, BOM kept)
doc.value() -> Value          // == parse_named's value
doc.get(&path) -> Option<Value>;  doc.span(&path) / value_span(&path) -> Option<Span>
doc.nearest_span(&path) -> Span   // the longest prefix that exists (a problem about a missing key)
doc.head_span() -> Span           // `return {`: a problem of the whole file;  doc.keys(&table_path)
doc.set(&path, &v) / remove(&path) / insert(&table, Slot::Key{key, after} | Slot::Position(n), &v)
doc.move_item(&list, from, to)
doc.apply_diff(&old, &new, &rules) -> Result<bool /*text changed*/, _>
LuaDoc::rebase(&base, &mine, theirs_text, &rules) -> Result<(LuaDoc, Vec<Conflict>), _>   // rebase_with: name, limits
Conflict { path: Vec<Seg>, mine: Option<Value>, theirs: Option<Value>, span: Option<Span> }  // result holds mine
DiffRules::new().keyed("nodes", "id").node_layout("nodes").order("", &["nq", "id", ...]).order("nodes.*", &[...])
Seg::Key(k) | Seg::Index(n /*from 0, the array part*/) | Seg::Item { field, value };  gz_luadata::path::path("nodes[id=a].params.text")
Span { line, column, end_line, end_column }   // 1-based, columns in characters, end exclusive
gz_luadata::{data_eq, covers, get_at, set_at}  // value helpers: equal as data; set_at(&mut v, &path, Value::Nil) removes
```

Rules for a model that saves through it: `old` = the model's value right after reading, `new` = now; keys neither
names are never touched (unknown keys survive); do not materialise defaults the file did not write; put new keys in
the order you want them written (`put`), the rules' `order` decides where among existing keys they land. A
`LuaDataError` now carries the exact line and column (characters) for every refusal.
