//! The Shape tool's edits: attaching one shape to another,
//! detaching a shape's parts into shapes of their own, and the level bound.

use gz_editor::{CmdError, json};

use super::{Batch, NameRule, ObjToolState, Outcome, selected_of};
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::{Aabb, Mat4, Transform, Vec3};
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects::{CF_SPHERE, SHAPE_LEVEL_BOUND, ShapeData, ShapeDef};

/// What a click of the Shape tool adds: a unit
/// sphere or the identity box.
#[must_use]
pub fn new_shape_data(sphere: bool) -> ShapeData {
    if sphere {
        ShapeData { shapes: vec![ShapeDef::sphere(Vec3::ZERO, 1.0)], usage: 0 }
    } else {
        ShapeData::unit_box()
    }
}

fn shape_of(o: &Object) -> Result<&ShapeData, CmdError> {
    match &o.data {
        ObjData::Shape(s) => Ok(s),
        _ => Err(CmdError::invalid("the object is not a shape")),
    }
}

/// Attaches `from` to the shape: both scales applied, `from`'s shapes carried into the base's space
/// and added to it, `from` removed.
pub fn attach(state: &SceneState, base: ObjId, from: ObjId) -> Result<Outcome, CmdError> {
    if base == from {
        return Err(CmdError::invalid("A shape cannot be attached to itself."));
    }
    let b = state.get(base).ok_or_else(|| CmdError::not_found("shape not found"))?;
    let f = state.get(from).ok_or_else(|| CmdError::not_found("shape not found"))?;
    let (mut bd, mut fd) = (shape_of(b)?.clone(), shape_of(f)?.clone());
    let (mut bx, mut fx) = (b.xform, f.xform);
    bd.apply_scale(&mut bx);
    fd.apply_scale(&mut fx);
    // `M = from->_Transform(); M.mulA_43(_ITransform())`
    let m = fx.matrix().mul_a43(&bx.inverse());
    for s in &fd.shapes {
        if s.kind == CF_SPHERE {
            let mut t = s.clone();
            let c = m.transform_tiny(s.center());
            t.data[0] = c.x;
            t.data[1] = c.y;
            t.data[2] = c.z;
            bd.shapes.push(t);
        } else {
            let mm = s.matrix().mul_a43(&m);
            bd.shapes.push(ShapeDef { pad: s.pad, ..ShapeDef::boxed(&mm) });
        }
    }
    let mut nb = (**b).clone();
    nb.xform = bx;
    nb.data = ObjData::Shape(bd);
    let mut batch = Batch::new("Attach Shape", state.serial);
    batch.remove.push(from);
    batch.replace.push(nb);
    Ok((batch, json!({ "shape": b.name.as_str(), "attached": f.name.as_str() })))
}

/// Adds a shape with Attach on Click: the one selected shape takes `from`.
pub fn attach_to_selected(state: &SceneState, from: ObjId) -> Result<Outcome, CmdError> {
    let sel = selected_of(state, ObjClass::Shape);
    let [base] = sel.as_slice() else { return Err(CmdError::invalid("Select one shape.")) };
    attach(state, *base, from)
}

/// The Detach confirmation's answer: the shapes are deselected, then each selected shape of
/// several parts keeps its first part and the others become shapes named
/// after it, all of them selected.
pub fn detach_selected(state: &SceneState) -> Result<Outcome, CmdError> {
    let sel = selected_of(state, ObjClass::Shape);
    if sel.is_empty() {
        return Err(CmdError::invalid("Select shapes first"));
    }
    let mut b = Batch::new("Detach Shapes", state.serial);
    b.deselect.extend(sel.iter().copied());
    let mut taken: Vec<String> = Vec::new();
    let mut made = 0;
    for id in &sel {
        let Some(o) = state.get(*id) else { continue };
        let Ok(sd) = shape_of(o) else { continue };
        if sd.shapes.len() <= 1 {
            continue;
        }
        let (mut d, mut xf) = (sd.clone(), o.xform);
        d.apply_scale(&mut xf);
        let m = xf.matrix();
        for s in d.shapes.iter().skip(1) {
            let mut t = Transform::default();
            let part = if s.kind == CF_SPHERE {
                t.position = m.transform_tiny(s.center());
                ShapeDef::sphere(Vec3::ZERO, s.radius())
            } else {
                let mut bm = s.matrix().mul_a43(&m);
                t.position = bm.c();
                bm.set_c(Vec3::ZERO);
                ShapeDef { pad: s.pad, ..ShapeDef::boxed(&bm) }
            };
            let name = unique_after(state, &o.name, &taken);
            taken.push(name.clone());
            let mut n = Object::new(
                ObjClass::Shape,
                &name,
                ObjData::Shape(ShapeData { shapes: vec![part], usage: d.usage }),
            );
            n.xform = t;
            b.add.push((n, NameRule::Fixed(name)));
            made += 1;
        }
        d.shapes.truncate(1);
        let mut no = (**o).clone();
        no.xform = xf;
        no.data = ObjData::Shape(d);
        b.replace.push(no);
        b.select.push(*id);
    }
    // the parts are selected too
    b.also_select_added = true;
    Ok((b, json!({ "shapes": sel.len(), "parts": made })))
}

/// A shape name made unique the way new names are, with the names this batch gives already.
fn unique_after(state: &SceneState, pref: &str, taken: &[String]) -> String {
    let pref = pref.to_lowercase();
    for i in 0u32.. {
        let n = if i == 0 { pref.clone() } else { format!("{pref}_{:02}", i - 1) };
        if !state.name_used(&n) && !taken.contains(&n) {
            return n;
        }
    }
    String::new()
}

/// `Scene->GetBox` of the scene objects merged with the groups'.
fn level_box(state: &SceneState, lib: &Library) -> Option<Aabb> {
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    let mut out: Option<Aabb> = None;
    for class in [ObjClass::SceneObject, ObjClass::Group] {
        for o in state.of_class(class) {
            if let Some(b) = crate::objects::world_box(state, o, &local) {
                let acc = out.get_or_insert(Aabb::INVALID);
                acc.modify(b.min);
                acc.modify(b.max);
            }
        }
    }
    out
}

/// Edits the level bound: the level bound shape, made when there is none
/// (`level_bbox`, one identity box), fitted to the scene objects and groups when asked or made.
/// `state_after` rides along (Edit Level Bound switched on).
pub fn edit_level_bound(
    state: &SceneState,
    lib: &Library,
    recalc: bool,
    state_after: Option<ObjToolState>,
) -> Result<Outcome, CmdError> {
    let existing = state
        .of_class(ObjClass::Shape)
        .find(|o| matches!(&o.data, ObjData::Shape(s) if s.usage == SHAPE_LEVEL_BOUND))
        .cloned();
    let fit = recalc || existing.is_none();
    let mut o = existing.as_ref().map_or_else(
        || {
            Object::new(
                ObjClass::Shape,
                "level_bbox",
                ObjData::Shape(ShapeData {
                    shapes: vec![ShapeDef::boxed(&Mat4::IDENTITY)],
                    usage: SHAPE_LEVEL_BOUND,
                }),
            )
        },
        |e| (**e).clone(),
    );
    let mut fitted = None;
    if fit {
        match level_box(state, lib) {
            Some(b) => {
                o.xform.scale = b.max - b.min;
                o.xform.position = b.center();
                fitted = Some(b);
            }
            None if recalc => return Err(CmdError::invalid("There is nothing to fit the level bound to.")),
            None => {}
        }
    }
    let mut batch = Batch::new(
        if existing.is_none() { "Add Level Bound" } else { "Recalculate Level Bound" },
        state.serial,
    );
    batch.state = state_after;
    let name = o.name.to_string();
    let created = existing.is_none();
    if created {
        o.flags.visible = state_after.is_none_or(|s| s.edit_level_bound) || state.tools.obj.edit_level_bound;
        batch.add.push((o, NameRule::Fixed("level_bbox".into())));
        batch.select_added = true;
    } else if fit {
        batch.replace.push(o);
    }
    Ok((
        batch,
        json!({
            "shape": if created { "level_bbox" } else { name.as_str() },
            "created": created,
            "fitted": fitted.map(|b| json!({ "min": b.min.to_array(), "max": b.max.to_array() })),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attached_sphere_lands_where_it_was() {
        let mut s = SceneState::default();
        let mut a = Object::new(ObjClass::Shape, "a", ObjData::Shape(new_shape_data(false)));
        a.xform.position = Vec3::new(10.0, 0.0, 0.0);
        let mut b = Object::new(ObjClass::Shape, "b", ObjData::Shape(new_shape_data(true)));
        b.xform.position = Vec3::new(12.0, 1.0, 0.0);
        let (a, b) = (s.insert(a), s.insert(b));
        let (batch, _) = attach(&s, a, b).unwrap();
        assert_eq!(batch.remove, vec![b]);
        let ObjData::Shape(d) = &batch.replace[0].data else { panic!() };
        assert_eq!(d.shapes.len(), 2);
        assert!((d.shapes[1].center() - Vec3::new(2.0, 1.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn a_detached_box_keeps_its_place() {
        let mut s = SceneState::default();
        let mut d = new_shape_data(false);
        let mut m = Mat4::IDENTITY;
        m.set_c(Vec3::new(3.0, 0.0, 0.0));
        d.shapes.push(ShapeDef::boxed(&m));
        let mut o = Object::new(ObjClass::Shape, "zone", ObjData::Shape(d));
        o.xform.position = Vec3::new(1.0, 2.0, 3.0);
        let id = s.insert(o);
        s.selection.insert(id);
        let (b, _) = detach_selected(&s).unwrap();
        assert_eq!(b.add.len(), 1);
        let (part, _) = &b.add[0];
        assert_eq!(part.name, "zone_00");
        assert!((part.xform.position - Vec3::new(4.0, 2.0, 3.0)).length() < 1e-5);
    }
}
