//! `build.state`: what the last builds of a level were made from, beside what they made.
//!
//! The file is Lua data in the level's build folder (`gamedata\levels\<level>\build.state`), so
//! what is stale is known right after the editor starts, and an agent can read it:
//!
//! ```lua
//! build = 1
//! level = "mp_test"
//! steps = {
//!   geometry = { fingerprint = "9f3a51c2e07b6d14", time = "2026-11-14 12:30:05", seconds = 1.25, quality = "standard" },
//!   light = { fingerprint = "...", time = "...", seconds = 63.5, quality = "draft", failed = "..." },
//! }
//! ai_map = { inputs = "...", nodes = "..." }
//! objects = { scene_object = [[<xform> <data> [g] <name>
//! ...]], ... }
//! ```
//!
//! `objects` holds every object's two hashes as the last build saw them, one line each, per class:
//! what lets the status say "12 objects moved" after a restart. `ai_map` says which AI map the
//! build generated itself: the build makes it again when the ground moves, and keeps a map the
//! author edited by hand.

use std::collections::BTreeMap;
use std::path::Path;

use gz_base::SmolStr;
use gz_luadata::{Table, Value};
use gz_scene::ObjClass;

use crate::fingerprint::{Entry, ObjHash, Objects};
use crate::steps::{Quality, Step};

/// The file's name in the level folder.
pub const FILE: &str = "build.state";
/// The version of the file this editor writes; a newer one is read as no record at all.
pub const VERSION: u32 = 1;

/// What one step's last run was made from.
#[derive(Clone, Debug, PartialEq)]
pub struct StepRecord {
    pub fingerprint: u64,
    /// Local time, `YYYY-MM-DD hh:mm:ss`.
    pub time: String,
    pub seconds: f64,
    pub quality: Quality,
    /// The compiler's flags it was built with (`-noise`); empty for none.
    pub flags: String,
    /// The models the scene's objects were made from when it was built
    /// (`fingerprint::library_stamp`); 0 for a step that reads none.
    pub library: u64,
    /// The sentence the step failed with; a failed step is never fresh.
    pub failed: Option<String>,
}

/// The AI map the build generated: the fingerprint of what it was generated over and of the nodes
/// it gave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Generated {
    pub inputs: u64,
    pub nodes: u64,
}

/// The record of a level's builds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildState {
    pub level: String,
    pub steps: BTreeMap<Step, StepRecord>,
    pub ai_map: Option<Generated>,
    pub objects: Objects,
}

fn hex(v: u64) -> String {
    format!("{v:016x}")
}

fn unhex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim(), 16).ok()
}

/// The local time now as `YYYY-MM-DD hh:mm:ss`.
#[must_use]
pub fn now_text() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

impl BuildState {
    #[must_use]
    pub fn new(level: &str) -> Self {
        Self { level: level.to_owned(), ..Self::default() }
    }

    /// The file's text.
    #[must_use]
    pub fn write(&self) -> String {
        let mut top = Table::new();
        top.set("build", VERSION);
        top.set("level", self.level.as_str());
        let mut steps = Table::new();
        for (step, r) in &self.steps {
            let mut t = Table::new();
            t.set("fingerprint", hex(r.fingerprint));
            t.set("time", r.time.as_str());
            t.set("seconds", (r.seconds * 1000.0).round() / 1000.0);
            t.set("quality", r.quality.name());
            if !r.flags.is_empty() {
                t.set("flags", r.flags.as_str());
            }
            if r.library != 0 {
                t.set("library", hex(r.library));
            }
            if let Some(f) = &r.failed {
                t.set("failed", f.as_str());
            }
            steps.set(step.name(), t);
        }
        top.set("steps", steps);
        if let Some(g) = self.ai_map {
            let mut t = Table::new();
            t.set("inputs", hex(g.inputs));
            t.set("nodes", hex(g.nodes));
            top.set("ai_map", t);
        }
        let mut objects = Table::new();
        for c in ObjClass::ALL {
            let list = self.objects.class(c);
            if list.is_empty() {
                continue;
            }
            let mut text = String::with_capacity(list.len() * 48);
            for e in list {
                text.push_str(&hex(e.hash.xform));
                text.push(' ');
                text.push_str(&hex(e.hash.data));
                text.push_str(if e.graph_point { " g " } else { " - " });
                // a name on one line: the scene keeps none with a line break
                text.push_str(&gz_luadata::one_line(&e.name));
                text.push('\n');
            }
            objects.set(c.name(), text);
        }
        top.set("objects", objects);
        let mut out =
            String::from("-- What the last builds of this level were made from. The editor writes it.\n");
        out.push_str(&gz_luadata::write_asset("", &top));
        out
    }

    /// Reads a record; `None` for a file that is not one this editor can read (a newer one, a
    /// broken one): the level then counts as never built.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        let v = gz_luadata::parse_named(text, FILE).ok()?;
        let version = v.get("build").and_then(Value::as_number)?;
        if version < 1.0 || version > f64::from(VERSION) {
            return None;
        }
        let mut out = Self { level: v.get_string("level", ""), ..Self::default() };
        if let Some(steps) = v.get("steps") {
            for (name, t) in steps.fields() {
                let Some(step) = Step::parse(name) else { continue };
                let Some(fingerprint) = t.get("fingerprint").and_then(Value::as_str).and_then(unhex) else {
                    continue;
                };
                out.steps.insert(
                    step,
                    StepRecord {
                        fingerprint,
                        time: t.get_string("time", ""),
                        seconds: t.get_number("seconds", 0.0),
                        quality: Quality::parse(&t.get_string("quality", "standard")).unwrap_or_default(),
                        flags: t.get_string("flags", ""),
                        library: t.get("library").and_then(Value::as_str).and_then(unhex).unwrap_or(0),
                        failed: t.get("failed").and_then(Value::as_str).map(str::to_owned),
                    },
                );
            }
        }
        if let Some(a) = v.get("ai_map") {
            let get = |k: &str| a.get(k).and_then(Value::as_str).and_then(unhex);
            if let (Some(inputs), Some(nodes)) = (get("inputs"), get("nodes")) {
                out.ai_map = Some(Generated { inputs, nodes });
            }
        }
        let mut by_class: Vec<Vec<Entry>> = vec![Vec::new(); ObjClass::ALL.len()];
        if let Some(objects) = v.get("objects") {
            for (name, text) in objects.fields() {
                let Some(c) = ObjClass::ALL.into_iter().find(|c| c.name() == name.as_str()) else { continue };
                let Some(text) = text.as_str() else { continue };
                for line in text.lines() {
                    let mut parts = line.splitn(4, ' ');
                    let (Some(x), Some(d), Some(g), Some(n)) =
                        (parts.next(), parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    let (Some(xform), Some(data)) = (unhex(x), unhex(d)) else { continue };
                    by_class[c.index()].push(Entry {
                        name: SmolStr::new(n),
                        hash: ObjHash { xform, data },
                        graph_point: g == "g",
                    });
                }
            }
        }
        out.objects = Objects { by_class };
        Some(out)
    }

    /// Reads the record of a level folder. Blocking.
    #[must_use]
    pub fn load(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(FILE)).ok()?;
        Self::read(&text)
    }

    /// Writes the record into a level folder, as one atomic write. Blocking.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        gz_editor::fsx::write_atomic(&dir.join(FILE), self.write().as_bytes())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_record_reads_back_as_written() {
        let mut s = BuildState::new("mp_test");
        s.steps.insert(
            Step::Geometry,
            StepRecord {
                fingerprint: 0x0123_4567_89ab_cdef,
                time: "2026-11-14 12:30:05".into(),
                seconds: 1.25,
                quality: Quality::Draft,
                flags: String::new(),
                library: 0x00c0_ffee,
                failed: None,
            },
        );
        s.steps.insert(
            Step::Light,
            StepRecord {
                fingerprint: u64::MAX,
                time: "2026-11-14 12:31:00".into(),
                seconds: 63.5,
                quality: Quality::Final,
                flags: "-noise -nosun".into(),
                library: 0,
                failed: Some("The compile failed: \"x\"".into()),
            },
        );
        s.ai_map = Some(Generated { inputs: 7, nodes: 9 });
        let mut by_class = vec![Vec::new(); ObjClass::ALL.len()];
        by_class[ObjClass::SceneObject.index()] = vec![
            Entry {
                name: "box with spaces 01".into(),
                hash: ObjHash { xform: 1, data: 2 },
                graph_point: false,
            },
            Entry { name: "]]odd[[".into(), hash: ObjHash { xform: 3, data: 4 }, graph_point: false },
        ];
        by_class[ObjClass::SpawnPoint.index()] =
            vec![Entry { name: "gp_01".into(), hash: ObjHash { xform: 5, data: 6 }, graph_point: true }];
        s.objects = Objects { by_class };
        let text = s.write();
        assert_eq!(BuildState::read(&text).unwrap(), s, "{text}");
    }

    #[test]
    fn a_newer_or_broken_record_counts_as_none() {
        assert!(BuildState::read("return { build = 99, level = \"x\" }").is_none());
        assert!(BuildState::read("return {").is_none());
        assert!(BuildState::read("return { level = \"x\" }").is_none());
    }
}
