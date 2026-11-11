//! The tools of sectors, portals, ways, shapes, lights and fog volumes (`UISectorTool`,
//! `UIPortalTool`, `UIWayTool`, `UIShapeTool`, `UILightTool`, `UIFogVolTool` and the logic
//! under them: `sector.cpp`, `portalutils.cpp`, `portal.cpp`, `WayPoint.cpp`, `EShape.cpp`,
//! `ESceneShapeTools.cpp`, `ESceneLightTools.cpp`, `ESceneFogVolumeTools.cpp`).
//!
//! - [`ObjToolState`] is what the panels switch (the Sector tool's mesh mode and box pick, the
//!   Way tool's Points mode and Auto Link, the Shape tool's shape, Attach on Click and Edit Level
//!   Bound, the fog volume group counter). It lives in [`crate::ToolSettings::obj`]; Undo does not
//!   take it back.
//! - Every change is one [`ToolEdit`] sent as [`SceneCmd::ObjTool`]: a [`Batch`] computed from a
//!   snapshot (refused when the scene moved meanwhile, and computed again by [`commit`]), or an
//!   edit of a way's points the store applies directly (the viewport's gestures).
//! - The geometry ([`sector`], [`portal`], [`shape`]) is pure, over a snapshot and the library;
//!   the long parts run as jobs on the CPU pool ([`cmd`]).
//! - [`cmd::register`] adds `sector_tool`, `portal_tool`, `way_tool`, `shape_tool`, `light_tool`
//!   and `fog_tool`.

pub mod cmd;
pub mod geom;
pub mod portal;
pub mod sector;
pub mod shape;
pub mod way;

use std::sync::Arc;

use gz_base::{Origin, SmolStr};
use gz_editor::{CmdError, Editor, JobCtx, JobError};
use gz_store::Apply;

use crate::apply::{SceneCmd, SceneReply, SelectMode};
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Vec3;
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects::SHAPE_LEVEL_BOUND;
use crate::service::SceneService;

/// What the Sector tool's Edit section holds (`UISectorTool`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectorMode {
    /// New Sector From Mesh.
    pub create_single: bool,
    /// New Sectors From Objects.
    pub create_multiple: bool,
    /// Meshes: Add (else Remove).
    pub mesh_add: bool,
    pub box_pick: bool,
}

impl Default for SectorMode {
    fn default() -> Self {
        Self { create_single: false, create_multiple: false, mesh_add: true, box_pick: false }
    }
}

/// The switches of the object tools' panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjToolState {
    pub sector: SectorMode,
    /// The Way tool selects points (`estWayModePoint`), not whole ways.
    pub way_points: bool,
    pub auto_link: bool,
    /// A click of the Shape tool adds a sphere (else a box).
    pub shape_sphere: bool,
    pub attach_on_click: bool,
    /// The Spawn tool's Attach on Click: a click attaches the object under it to the spawn point.
    pub spawn_attach: bool,
    pub edit_level_bound: bool,
    /// `m_group_counter` of the fog volumes.
    pub fog_group_counter: u32,
}

impl Default for ObjToolState {
    fn default() -> Self {
        Self {
            sector: SectorMode::default(),
            way_points: false,
            auto_link: true,
            // oracle: `m_SphereMode` is never initialised in the C++; a click adds a box here
            shape_sphere: false,
            attach_on_click: false,
            spawn_attach: false,
            edit_level_bound: false,
            fog_group_counter: 0,
        }
    }
}

/// How an added object is named.
#[derive(Clone, Debug, PartialEq)]
pub enum NameRule {
    /// `GenObjectName(cls, buf, pref)`: `pref`, `pref_00` ...; `00`, `01` ... without one.
    Gen(Option<String>),
    /// This name, made unique the same way when it is taken.
    Fixed(String),
}

/// A change of many objects, one undo step.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    /// The snapshot it was computed from; `None` when it was computed inside the store.
    pub serial: Option<u64>,
    pub remove: Vec<ObjId>,
    pub replace: Vec<Object>,
    pub add: Vec<(Object, NameRule)>,
    /// `AppendObject(obj, true)`: the class of each added object is deselected and the added one
    /// selected.
    pub select_added: bool,
    /// The added objects are selected besides what is selected (`Select(true)` of each).
    pub also_select_added: bool,
    /// Objects selected after the change (on top of `select_added`).
    pub select: Vec<ObjId>,
    pub deselect: Vec<ObjId>,
    /// Tool switches the change sets with it.
    pub state: Option<ObjToolState>,
    pub label: String,
    /// A gesture's key: edits with one key in a row are one undo step.
    pub merge: Option<SmolStr>,
}

impl Batch {
    #[must_use]
    pub fn new(label: &str, serial: u64) -> Self {
        Self { serial: Some(serial), label: label.to_owned(), ..Self::default() }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty()
            && self.replace.is_empty()
            && self.add.is_empty()
            && self.select.is_empty()
            && self.deselect.is_empty()
            && self.state.is_none()
    }
}

/// An edit of the object tools.
#[derive(Clone, Debug)]
pub enum ToolEdit {
    Batch(Box<Batch>),
    /// The panels' switches: no undo step, nothing unsaved.
    State(ObjToolState),
    /// Points of a way: select (in Points mode the selection of a way's points is the point's
    /// own flag, saved with the scene but not an undo step).
    SelectPoints {
        way: ObjId,
        points: Vec<usize>,
        mode: SelectMode,
    },
    /// Moves points of a way to positions, as one gesture.
    MovePoints {
        way: ObjId,
        points: Vec<(usize, Vec3)>,
        merge: Option<SmolStr>,
    },
}

/// Applies an edit (the store's side of [`SceneCmd::ObjTool`]).
pub(crate) fn apply(state: &mut SceneState, edit: ToolEdit, cx: &mut Apply) -> Result<SceneReply, CmdError> {
    match edit {
        ToolEdit::State(s) => {
            let before = state.tools.obj;
            state.tools.obj = s;
            if before.edit_level_bound != s.edit_level_bound {
                show_level_bounds(state, s.edit_level_bound);
            }
            Ok(SceneReply::None)
        }
        ToolEdit::SelectPoints { way, points, mode } => {
            let mut o = (**state.get(way).ok_or_else(|| missing(way))?).clone();
            let ObjData::Way(w) = &mut o.data else {
                return Err(CmdError::invalid("the object is not a way"));
            };
            if mode == SelectMode::Set {
                for p in &mut w.points {
                    p.selected = false;
                }
            }
            let mut n = 0;
            for i in points {
                let Some(p) = w.points.get_mut(i) else { continue };
                n += 1;
                p.selected = match mode {
                    SelectMode::Set | SelectMode::Add => true,
                    SelectMode::Remove => false,
                    SelectMode::Toggle => !p.selected,
                };
            }
            state.replace(o);
            Ok(SceneReply::Count(n))
        }
        ToolEdit::MovePoints { way, points, merge } => {
            let mut o = (**state.get(way).ok_or_else(|| missing(way))?).clone();
            let ObjData::Way(w) = &mut o.data else {
                return Err(CmdError::invalid("the object is not a way"));
            };
            let mut n = 0;
            for (i, p) in points {
                if !p.is_finite() {
                    return Err(CmdError::invalid("non-finite component"));
                }
                if let Some(wp) = w.points.get_mut(i) {
                    wp.position = p;
                    n += 1;
                }
            }
            if n == 0 {
                cx.unchanged();
                return Ok(SceneReply::Count(0));
            }
            state.replace(o);
            state.serial += 1;
            match merge {
                Some(key) => cx.undoable_merged("Move Points", key),
                None => cx.undoable("Move Points"),
            }
            Ok(SceneReply::Count(n))
        }
        ToolEdit::Batch(b) => apply_batch(state, *b, cx),
    }
}

fn missing(id: ObjId) -> CmdError {
    CmdError::not_found(format!("object {} is not in the scene", id.0))
}

/// A name no class uses, by the rule; `taken` are the names given in this batch already.
fn name_by(state: &SceneState, rule: &NameRule) -> String {
    match rule {
        NameRule::Gen(pref) => state.gen_name(pref.as_deref()),
        NameRule::Fixed(n) => {
            let n = n.to_lowercase();
            if state.name_used(&n) { state.gen_name(Some(&n)) } else { n }
        }
    }
}

fn apply_batch(state: &mut SceneState, b: Batch, cx: &mut Apply) -> Result<SceneReply, CmdError> {
    if let Some(serial) = b.serial
        && serial != state.serial
    {
        return Err(CmdError::busy("the scene changed meanwhile; ask again"));
    }
    if b.is_empty() {
        cx.unchanged();
        return Ok(SceneReply::Count(0));
    }
    let content = !(b.remove.is_empty() && b.replace.is_empty() && b.add.is_empty());
    for id in &b.remove {
        state.remove(*id);
    }
    for o in b.replace {
        if state.get(o.id).is_none() {
            return Err(missing(o.id));
        }
        if state.find_any(&o.name).is_some_and(|x| x.id != o.id) {
            return Err(CmdError::invalid("the name is used by another object"));
        }
        state.replace(o);
    }
    let mut added = Vec::with_capacity(b.add.len());
    for (mut o, rule) in b.add {
        o.name = SmolStr::new(name_by(state, &rule));
        o.marks.authored = true;
        let class = o.class;
        if b.select_added {
            for id in state.by_class[class.index()].clone() {
                state.selection.remove(&id);
            }
        }
        let id = state.insert(o);
        if b.select_added || b.also_select_added {
            state.selection.insert(id);
        }
        added.push(id);
    }
    for id in b.deselect {
        state.selection.remove(&id);
    }
    for id in b.select {
        if state.get(id).is_some_and(|o| o.flags.visible) {
            state.selection.insert(id);
        }
    }
    if let Some(s) = b.state {
        let before = state.tools.obj;
        state.tools.obj = s;
        if before.edit_level_bound != s.edit_level_bound {
            show_level_bounds(state, s.edit_level_bound);
        }
    }
    if content {
        state.serial += 1;
        match b.merge {
            Some(key) => cx.undoable_merged(b.label, key),
            None => cx.undoable(b.label),
        }
    }
    Ok(match added.as_slice() {
        [one] => SceneReply::Id(*one),
        _ => SceneReply::Count(added.len()),
    })
}

/// `CEditShape::OnFrame`: a level bound shape is seen only while Edit Level Bound is on.
pub(crate) fn show_level_bounds(state: &mut SceneState, on: bool) {
    let ids: Vec<ObjId> = state
        .of_class(ObjClass::Shape)
        .filter(|o| {
            matches!(&o.data, ObjData::Shape(s) if s.usage == SHAPE_LEVEL_BOUND) && o.flags.visible != on
        })
        .map(|o| o.id)
        .collect();
    for id in ids {
        if let Some(o) = state.get(id) {
            let mut o = (**o).clone();
            o.flags.visible = on;
            state.replace(o);
        }
        if !on {
            state.selection.remove(&id);
        }
    }
}

/// The ids of a class's selected visible objects in their order (`GetQueryObjects(lst, cls, 1, 1,
/// 0)`).
#[must_use]
pub fn selected_of(state: &SceneState, class: ObjClass) -> Vec<ObjId> {
    state.of_class(class).filter(|o| state.is_selected(o.id) && o.flags.visible).map(|o| o.id).collect()
}

/// What a computed change answers besides the batch.
pub type Outcome = (Batch, gz_editor::Value);

/// What a computation may look at while it runs: its job, when it is one.
#[derive(Clone, Default)]
pub struct Ctl(Option<JobCtx>);

impl Ctl {
    #[must_use]
    pub fn of_job(ctx: JobCtx) -> Self {
        Self(Some(ctx))
    }

    /// The job was asked to stop.
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.0.as_ref().is_some_and(|c| c.cancel.is_cancelled())
    }

    pub fn stage(&self, text: &str) {
        if let Some(c) = &self.0 {
            c.stage(text);
        }
    }
}

/// The error a stopped computation answers.
#[must_use]
pub fn stopped() -> CmdError {
    CmdError::new("cancelled", "Stopped.")
}

/// Computes a change from the snapshot there is now and sends it; when the scene moved while it
/// was computed the result is dropped and computed again (four times at most). `f` runs on the
/// CPU pool.
pub async fn commit<F>(
    editor: &Arc<Editor>,
    svc: &SceneService,
    origin: Origin,
    ctl: Ctl,
    f: F,
) -> Result<gz_editor::Value, CmdError>
where
    F: Fn(&SceneState, &Library, &Ctl) -> Result<Outcome, CmdError> + Send + Sync + 'static,
{
    let f = Arc::new(f);
    let ctl = Arc::new(ctl);
    let mut last = CmdError::busy("the scene kept changing; ask again");
    for _ in 0..4 {
        let snap = svc.store.snapshot();
        if !snap.valid {
            return Err(CmdError::new("no_scene", "no scene"));
        }
        let lib = svc.library.clone();
        let (g, c) = (f.clone(), ctl.clone());
        let (batch, answer) = editor.exec.cpu(move || g(&snap, &lib, &c)).await?;
        if ctl.stopped() {
            return Err(stopped());
        }
        match svc.store.send(SceneCmd::ObjTool(Box::new(ToolEdit::Batch(Box::new(batch)))), origin).await {
            Ok(_) => return Ok(answer),
            Err(e) if e.code == "busy" => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// [`commit`] as a job of the Task Center (`title` on its card, Stop asks it to stop): the
/// library objects of the scene are loaded first. Answers when the job ended.
pub async fn commit_job<F>(
    editor: &Arc<Editor>,
    svc: &Arc<SceneService>,
    origin: Origin,
    title: &str,
    f: F,
) -> Result<gz_editor::Value, CmdError>
where
    F: Fn(&SceneState, &Library, &Ctl) -> Result<Outcome, CmdError> + Send + Sync + 'static,
{
    let (ed, s) = (editor.clone(), svc.clone());
    let started = editor.jobs.start("scene_tool", Some("scene_tool"), title, move |ctx| async move {
        ctx.stage("Loading the library objects");
        let snap = s.store.snapshot();
        s.prefetch_library(&ed, &snap).await;
        commit(&ed, &s, origin, Ctl::of_job(ctx), f)
            .await
            .map_err(|e| JobError { cancelled: e.code == "cancelled", message: e.message })
    });
    let id = started.map_err(|busy| CmdError::busy(format!("{} is still running", busy.title)))?;
    match editor.jobs.wait(id).await {
        Some(i) if i.state == gz_editor::JobState::Done => {
            Ok(i.result.map_or(gz_editor::Value::Null, |v| (*v).clone()))
        }
        Some(i) if i.state == gz_editor::JobState::Cancelled => Err(stopped()),
        Some(i) => Err(CmdError::invalid(i.error.unwrap_or_else(|| "The command failed.".into()))),
        None => Err(CmdError::internal("the job is gone")),
    }
}

/// Sets the panels' switches.
pub async fn set_state(
    svc: &SceneService,
    origin: Origin,
    f: impl FnOnce(&mut ObjToolState),
) -> Result<ObjToolState, CmdError> {
    let mut s = svc.store.snapshot().tools.obj;
    f(&mut s);
    svc.store.send(SceneCmd::ObjTool(Box::new(ToolEdit::State(s))), origin).await?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{GlowData, ShapeData};
    use gz_store::{Store, StoreOptions};
    use gz_task::{ExecOptions, Runtime};

    #[test]
    fn a_stale_batch_is_refused_and_a_fresh_one_names_its_objects() {
        let rt = Runtime::start(ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 }).unwrap();
        let mut s = SceneState::default();
        s.insert(Object::new(ObjClass::Glow, "00", ObjData::Glow(GlowData::default())));
        let store = Store::spawn("scene", s, &rt.exec(), StoreOptions::default());
        let send = |b: Batch| {
            rt.block_on(store.send(SceneCmd::ObjTool(Box::new(ToolEdit::Batch(Box::new(b)))), Origin::Mcp))
        };
        let mut b = Batch::new("Add", 99);
        b.add.push((
            Object::new(ObjClass::Shape, "", ObjData::Shape(ShapeData::unit_box())),
            NameRule::Gen(None),
        ));
        assert!(send(b.clone()).is_err());
        b.serial = Some(0);
        b.select_added = true;
        let Ok(SceneReply::Id(id)) = send(b) else { panic!() };
        let st = store.snapshot();
        assert_eq!(st.get(id).unwrap().name, "01");
        assert!(st.is_selected(id));
        assert!(store.is_dirty());
    }

    #[test]
    fn level_bounds_hide_with_the_switch() {
        let mut s = SceneState::default();
        let id = s.insert(Object::new(
            ObjClass::Shape,
            "level_bbox",
            ObjData::Shape(ShapeData { usage: SHAPE_LEVEL_BOUND, ..ShapeData::unit_box() }),
        ));
        show_level_bounds(&mut s, false);
        assert!(!s.get(id).unwrap().flags.visible);
        show_level_bounds(&mut s, true);
        assert!(s.get(id).unwrap().flags.visible);
    }
}
