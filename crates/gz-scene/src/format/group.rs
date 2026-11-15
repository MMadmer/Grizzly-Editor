//! Sets of objects as text: a `.group` asset (objects saved for reuse) and the clipboard of
//! Copy and Paste are one format - the header `group = 1`, then the lists of the scene's folder
//! files in one file, positions relative to the set's pivot.

use gz_base::{Problem, RelPath, SmolStr};
use gz_entities::Settings;
use gz_luadata::{Limits, LuaDoc, Seg, Table, Value};

use super::values::*;
use super::{CLASS_FILES, Chain, Cx, Refused, default_object, object_of, object_value, spawn};
use crate::math::Vec3;
use crate::model::{ObjData, ObjId, Object, SceneState, co};

pub const EXT: &str = "group";
pub const HEADER: &str = "group";
pub const VERSION: u32 = 1;
/// The steps that lift an older `.group` to [`VERSION`] (the group kind's `KindOps.migrations`).
pub const MIGRATIONS: &[gz_asset::Migration] = &[];
/// The group's own chain.
pub const CHAIN: Chain = Chain { current: VERSION, first: 1, steps: MIGRATIONS };

/// A set of objects as read: the objects in the file's order, the groups among them with their
/// members (by index into `objects`).
#[derive(Clone, Debug, Default)]
pub struct Bundle {
    pub objects: Vec<Object>,
    pub groups: Vec<(usize, Vec<usize>)>,
}

/// The ids of a selection with every member of its groups, in placement order.
#[must_use]
pub fn with_members(state: &SceneState, ids: &[ObjId]) -> Vec<ObjId> {
    let mut all: Vec<ObjId> = Vec::new();
    for id in ids {
        if !all.contains(id) {
            all.push(*id);
        }
        if let Some(ObjData::Group(g)) = state.get(*id).map(|o| &o.data) {
            for m in &g.members {
                if !all.contains(m) {
                    all.push(*m);
                }
            }
        }
    }
    all.sort();
    all
}

/// The `.group` text of objects, positions relative to `pivot`.
pub fn group_text(
    state: &SceneState,
    ids: &[ObjId],
    pivot: Vec3,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<String, String> {
    let cx = Cx { state, settings };
    let mut top = Table::new();
    top.put(HEADER, int(VERSION));
    for cf in &CLASS_FILES {
        for (list, class) in cf.lists {
            let dflt = default_object(*class);
            let mut items = Vec::new();
            for id in ids {
                let Some(o) = state.get(*id) else { continue };
                if o.class != *class {
                    continue;
                }
                let mut moved = (**o).clone();
                crate::apply::translate(&mut moved, -pivot);
                items.push(object_value(&cx, &moved, &dflt)?);
            }
            if !items.is_empty() {
                top.put(list, Value::Table(Table { arr: items, fields: Vec::new() }));
            }
        }
    }
    Ok(gz_luadata::write_asset("", &top))
}

/// Reads a set of objects; their positions are the file's (relative to its pivot).
pub fn read_group(
    text: &str,
    file: &RelPath,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<(Bundle, Vec<Problem>), Refused> {
    read_group_with(text, file, settings, &CHAIN)
}

/// [`read_group`] through a migration chain.
pub fn read_group_with(
    text: &str,
    file: &RelPath,
    settings: Option<&(dyn Settings + Sync)>,
    chain: &Chain,
) -> Result<(Bundle, Vec<Problem>), Refused> {
    let (mut doc, mut v) = LuaDoc::parse_value(text, HEADER, Limits::scene()).map_err(|e| Refused {
        code: "syntax",
        problem: Box::new(Problem::error("syntax", e.message).in_file(file.clone()).at(
            e.line.max(1),
            e.column.max(1),
            e.line.max(1),
            e.column.max(1) + 1,
        )),
    })?;
    let header_at = |doc: &LuaDoc| doc.span(&[Seg::key(HEADER)]).unwrap_or_else(|| doc.head_span());
    match v.get(HEADER).and_then(Value::as_number) {
        Some(n) if n > f64::from(chain.current) => {
            let s = header_at(&doc);
            return Err(Refused {
                code: "newer_version",
                problem: Problem::error(
                    "newer_version",
                    format!("{file} was made by a newer Grizzly Editor (group = {n}; this one reads up to {}): update the editor to open it", chain.current),
                )
                .in_file(file.clone())
                .at(s.line, s.column, s.end_line, s.end_column)
                .into(),
            });
        }
        Some(n) if n >= 0.0 && n.fract() == 0.0 && n < f64::from(chain.current) => {
            chain.lift(&mut doc, HEADER, file, n as u32)?;
            v = doc.value();
        }
        Some(_) => {}
        None => {
            return Err(Refused {
                code: "E001",
                problem: Problem::error(
                    "E001",
                    format!("{file} is no object group: it has no 'group = {VERSION}'"),
                )
                .in_file(file.clone())
                .at(
                    doc.head_span().line,
                    doc.head_span().column,
                    doc.head_span().end_line,
                    doc.head_span().end_column,
                )
                .into(),
            });
        }
    }
    let mut bundle = Bundle::default();
    let mut problems = Vec::new();
    let mut members: Vec<(usize, Vec<SmolStr>)> = Vec::new();
    for cf in &CLASS_FILES {
        for (list, class) in cf.lists {
            let Some(items) = v.get(list) else { continue };
            for (i, item) in items.arr().iter().enumerate() {
                let mut found = Vec::new();
                let mut o = object_of(item, *class, &mut found);
                for (key, p) in found {
                    let mut path = vec![Seg::key(list), Seg::Index(i)];
                    if let Some(k) = key {
                        path.push(Seg::key(k));
                    }
                    let s = doc.nearest_span(&path);
                    problems.push(p.in_file(file.clone()).at(s.line, s.column, s.end_line, s.end_column));
                }
                if let (ObjData::Spawn(_), Some(st)) = (&o.data, settings) {
                    spawn::resolve(&mut o, st);
                }
                if matches!(o.data, ObjData::Group(_)) {
                    let names = item
                        .get("members")
                        .map(|x| x.arr().iter().filter_map(Value::as_str).map(SmolStr::new).collect())
                        .unwrap_or_default();
                    members.push((bundle.objects.len(), names));
                }
                bundle.objects.push(o);
            }
        }
    }
    for (g, names) in members {
        let idx: Vec<usize> =
            names.iter().filter_map(|n| bundle.objects.iter().position(|o| o.name == *n)).collect();
        bundle.groups.push((g, idx));
    }
    Ok((bundle, problems))
}

/// Puts a bundle's objects into the scene under names no object uses, each moved by `offset`;
/// its groups hold their members again. Answers the new ids in the bundle's order.
pub fn insert(state: &mut SceneState, bundle: Bundle, offset: Vec3) -> Vec<ObjId> {
    let mut ids = Vec::with_capacity(bundle.objects.len());
    for mut o in bundle.objects {
        if !state.name_free(&o.name) {
            o.name = SmolStr::new(state.unique_name(&o.name));
        }
        o.owner = None;
        o.flags.co &= !co::IN_GROUP;
        if let ObjData::Group(g) = &mut o.data {
            g.members.clear();
        }
        crate::apply::translate(&mut o, offset);
        ids.push(state.insert(o));
    }
    for (g, members) in bundle.groups {
        let Some(gid) = ids.get(g).copied() else { continue };
        let mids: Vec<ObjId> =
            members.iter().filter_map(|m| ids.get(*m).copied()).filter(|m| *m != gid).collect();
        for m in &mids {
            if let Some(o) = state.get(*m) {
                let mut o = (**o).clone();
                o.owner = Some(gid);
                o.flags.co |= co::IN_GROUP;
                state.replace(o);
            }
        }
        if let Some(go) = state.get(gid) {
            let mut go = (**go).clone();
            if let ObjData::Group(gd) = &mut go.data {
                gd.members = mids;
            }
            state.replace(go);
        }
    }
    ids
}
