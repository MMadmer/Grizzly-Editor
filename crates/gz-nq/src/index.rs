//! The project's graphs as one read-only view (`NqProjectIndex`) and the node search shared by
//! `quest_find` and `quest_project_find`.
//!
//! The service builds the [`IndexSnapshot`] from the project's file tree (a file is read and
//! parsed again only when its stamp moved) with the dirty open documents laid over their disk
//! entries; this module holds what is pure: the entry, the fingerprint, the search.

use std::sync::Arc;

use serde_json::{Value as Json, json};

use crate::asset::{self, Graph};
use crate::catalog::Catalog;
use crate::text::{fold, trim};

/// One graph of the project.
#[derive(Clone, Debug)]
pub struct IndexEntry {
    /// Project-relative, forward slashes: what the wire says.
    pub rel: String,
    /// The path key the fingerprint hashes: the absolute path, backslashes, lower case.
    pub key: String,
    pub graph: Arc<Graph>,
    pub error: String,
    pub readable: bool,
    /// The model holds all of the file (false for a document opened from a malformed file).
    pub complete: bool,
    /// From an open document with unsaved edits, not from the disk.
    pub memory: bool,
    pub content_hash: u64,
}

impl IndexEntry {
    #[must_use]
    pub fn source(&self) -> &'static str {
        if self.memory { "memory" } else { "disk" }
    }

    /// A readable whole graph: what the id lists count (`SProjectQuest::readable`).
    #[must_use]
    pub fn usable(&self) -> bool {
        self.readable && self.complete
    }

    /// A behaviour graph: by its header when readable, by its name otherwise.
    #[must_use]
    pub fn behaviour(&self) -> bool {
        if self.readable { self.graph.is_behaviour() } else { asset::is_behaviour_path(&self.rel) }
    }
}

/// Every graph of the project, sorted by path key.
#[derive(Clone, Debug, Default)]
pub struct IndexSnapshot {
    pub entries: Vec<IndexEntry>,
    /// Moves whenever the fingerprint does.
    pub generation: u32,
    pub fingerprint: u64,
}

impl IndexSnapshot {
    /// The ids of the other usable graphs, both kinds: one id space per module (`OtherQuestIds`).
    #[must_use]
    pub fn other_ids(&self, except_rel: Option<&str>) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.usable() && !e.graph.id.is_empty())
            .filter(|e| except_rel.is_none_or(|x| !e.rel.eq_ignore_ascii_case(x)))
            .map(|e| e.graph.id.clone())
            .collect()
    }

    /// The ids of one kind (`ProjectIdsOfKind`).
    #[must_use]
    pub fn ids_of_kind(&self, behaviour: bool) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.usable() && !e.graph.id.is_empty() && e.behaviour() == behaviour)
            .map(|e| e.graph.id.clone())
            .collect()
    }

    #[must_use]
    pub fn get(&self, rel: &str) -> Option<&IndexEntry> {
        self.entries.iter().find(|e| e.rel.eq_ignore_ascii_case(rel))
    }
}

pub const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
const FNV_PRIME: u64 = 1_099_511_628_211;

#[must_use]
pub fn hash_bytes(mut hash: u64, data: &[u8]) -> u64 {
    for &b in data {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn hash_string(hash: u64, s: &str) -> u64 {
    hash_bytes(hash_bytes(hash, s.as_bytes()), &[0])
}

/// The FNV-1a hash of a file's text.
#[must_use]
pub fn content_hash(text: &str) -> u64 {
    hash_bytes(FNV_OFFSET, text.as_bytes())
}

/// The fingerprint of a snapshot: the root, then every entry's path key, content hash, flags and
/// error.
#[must_use]
pub fn fingerprint(root_key: &str, entries: &[IndexEntry]) -> u64 {
    let mut f = hash_string(FNV_OFFSET, root_key);
    for e in entries {
        f = hash_string(f, &e.key);
        f = hash_bytes(f, &e.content_hash.to_le_bytes());
        let flags = u8::from(e.readable) | (u8::from(e.complete) << 1) | (u8::from(e.memory) << 2);
        f = hash_bytes(f, &[flags]);
        f = hash_string(f, &e.error);
    }
    f
}

/// `%016llx`.
#[must_use]
pub fn fingerprint_text(f: u64) -> String {
    format!("{f:016x}")
}

/// An entry read from the disk: the file's text parsed, or why it does not parse.
#[must_use]
pub fn disk_entry(catalog: &Catalog, rel: &str, key: &str, text: &str) -> IndexEntry {
    let hash = content_hash(text);
    match asset::parse(catalog, text, &asset::base_name(rel)) {
        Ok(g) => IndexEntry {
            rel: rel.to_owned(),
            key: key.to_owned(),
            graph: Arc::new(g),
            error: String::new(),
            readable: true,
            complete: true,
            memory: false,
            content_hash: hash,
        },
        Err(e) => IndexEntry {
            rel: rel.to_owned(),
            key: key.to_owned(),
            graph: Arc::new(Graph::default()),
            error: e.with_position(),
            readable: false,
            complete: false,
            memory: false,
            content_hash: hash,
        },
    }
}

// ---- the search -------------------------------------------------------------------------------

/// One term of a query: tokens AND-joined, quotes keep a phrase, `-token` excludes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub text: String,
    pub exclude: bool,
}

/// The terms of a query (`ParseSearch`), folded.
#[must_use]
pub fn parse_query(query: &str) -> Vec<Term> {
    let b = trim(query).as_bytes();
    let mut out = Vec::new();
    let mut p = 0;
    while p < b.len() {
        while b.get(p).is_some_and(|&c| c <= 0x20) {
            p += 1;
        }
        if p >= b.len() {
            break;
        }
        let exclude = b.get(p) == Some(&b'-') && b.get(p + 1).is_some_and(|&c| c > 0x20);
        if exclude {
            p += 1;
        }
        let quoted = b.get(p) == Some(&b'"');
        if quoted {
            p += 1;
        }
        let begin = p;
        if quoted {
            while b.get(p).is_some_and(|&c| c != b'"') {
                p += 1;
            }
        } else {
            while b.get(p).is_some_and(|&c| c > 0x20) {
                p += 1;
            }
        }
        let raw = String::from_utf8_lossy(b.get(begin..p).unwrap_or(&[])).into_owned();
        if quoted && b.get(p) == Some(&b'"') {
            p += 1;
        }
        let text = fold(&raw);
        if !text.is_empty() {
            out.push(Term { text, exclude });
        }
    }
    out
}

/// Every term holds (`SearchMatches`).
#[must_use]
pub fn matches(text: &str, terms: &[Term]) -> bool {
    let folded = fold(text);
    !terms.is_empty() && terms.iter().all(|t| folded.contains(&t.text) != t.exclude)
}

/// A field holds one of the including terms (`SearchFieldHit`).
#[must_use]
pub fn field_hit(text: &str, terms: &[Term]) -> bool {
    let folded = fold(text);
    terms.iter().any(|t| !t.exclude && folded.contains(&t.text))
}

/// A node that matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindHit {
    pub node: String,
    pub kind: String,
    pub title: String,
    /// `id`, `kind`, `comment` or `content`.
    pub matched: &'static str,
}

/// The nodes of one graph a query finds; `title` gives a kind's title as the caller shows it.
#[must_use]
pub fn find_in_graph(g: &Graph, terms: &[Term], title: &dyn Fn(&str) -> String) -> Vec<FindHit> {
    if terms.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for n in &g.nodes {
        let t = title(&n.kind);
        let searchable = format!("{}\n{t}", asset::write_node(n, 0));
        if !matches(&searchable, terms) {
            continue;
        }
        let matched = if field_hit(&n.id, terms) {
            "id"
        } else if field_hit(&n.kind, terms) || field_hit(&t, terms) {
            "kind"
        } else if field_hit(&n.comment, terms) {
            "comment"
        } else {
            "content"
        };
        out.push(FindHit { node: n.id.to_string(), kind: n.kind.to_string(), title: t, matched });
    }
    out
}

/// What a project search found (`SFindResponse`).
#[derive(Clone, Debug, Default)]
pub struct ProjectFind {
    pub results: Vec<(String, FindHit, &'static str)>,
    pub diagnostics: Vec<Json>,
    pub complete: bool,
    pub generation: u32,
    pub fingerprint: u64,
}

/// The nodes of every graph a query finds (`FindNodes`); a graph that could not be read is a
/// diagnostic and makes the answer incomplete.
#[must_use]
pub fn find_nodes(snapshot: &IndexSnapshot, catalog: &Catalog, query: &str) -> ProjectFind {
    let mut out = ProjectFind {
        complete: true,
        generation: snapshot.generation,
        fingerprint: snapshot.fingerprint,
        ..ProjectFind::default()
    };
    let terms = parse_query(query);
    let title = |k: &str| catalog.find(k).map_or_else(|| k.to_owned(), |x| x.title.clone());
    for e in &snapshot.entries {
        if !e.usable() {
            out.diagnostics.push(json!({
                "path": e.rel,
                "code": if e.readable { "incomplete_model" } else { "unreadable" },
                "message": if e.error.is_empty() { "quest could not be inspected" } else { e.error.as_str() },
                "source": e.source(),
            }));
            out.complete = false;
            continue;
        }
        for hit in find_in_graph(&e.graph, &terms, &title) {
            out.results.push((e.rel.clone(), hit, e.source()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries() {
        let t = parse_query("  wolf -boar \"two words\" -");
        assert_eq!(
            t,
            [
                Term { text: "wolf".into(), exclude: false },
                Term { text: "boar".into(), exclude: true },
                Term { text: "two words".into(), exclude: false },
                Term { text: "-".into(), exclude: false },
            ]
        );
        let terms = parse_query("Волк -кабан");
        assert!(matches("волк пришёл", &terms));
        assert!(!matches("волк и кабан", &terms));
        assert!(!matches("anything", &[]));
        assert_eq!(fingerprint_text(0xab), "00000000000000ab");
    }

    #[test]
    fn finds_nodes_by_field() {
        let mut g = Graph::default();
        let mut n = crate::asset::Node::new("meet_wolf", "dialog.topic");
        n.comment = "the boar talk".into();
        g.nodes.push_back(n);
        g.nodes.push_back(crate::asset::Node::new("other", "flow.step"));
        let title = |k: &str| if k == "dialog.topic" { "Dialog: topic".to_owned() } else { k.to_owned() };
        let hits = find_in_graph(&g, &parse_query("wolf"), &title);
        assert_eq!(
            hits.iter().map(|h| (h.node.as_str(), h.matched)).collect::<Vec<_>>(),
            [("meet_wolf", "id")]
        );
        let hits = find_in_graph(&g, &parse_query("dialog:"), &title);
        assert_eq!(hits.first().map(|h| h.matched), Some("kind"));
        let hits = find_in_graph(&g, &parse_query("boar"), &title);
        assert_eq!(hits.first().map(|h| h.matched), Some("comment"));
    }
}
