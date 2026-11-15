//! A scene made here, with its models given by a provider: a ground quad, a box standing on it,
//! a box floating above it, a wall (a model without surfaces) and a shape.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gz_fmt_level::{AiMapNode, SceneAiMap};
use gz_fmt_object::{Face, FaceVert, Mesh, Object as EditObject, SurfFaces, Surface};
use serde_json::{Value, json};

use super::{QueryParams, Scene, query};
use crate::class::ObjClass;
use crate::library::{Library, LibraryObject};
use crate::math::Vec3;
use crate::model::{ObjData, Object, SceneState};
use crate::objects::{SceneObjectData, ShapeData};
use crate::pick::ScenePick;
use crate::tools::AiMapPart;

fn surface(name: &str, texture: &str, mtl: &str) -> Surface {
    Surface { name: name.into(), texture: texture.into(), game_mtl: mtl.into(), ..Surface::default() }
}

fn mesh(points: Vec<Vec3>, idx: &[[u32; 3]], surfaced: bool) -> Mesh {
    // the lists below wind inwards; a face's front is the other way
    let faces: Vec<Face> =
        idx.iter().map(|t| Face { v: [t[0], t[2], t[1]].map(|i| FaceVert { point: i, vmref: 0 }) }).collect();
    let n = u32::try_from(faces.len()).unwrap_or(0);
    Mesh {
        points: points.into(),
        faces: faces.into(),
        surf_faces: if surfaced {
            vec![SurfFaces { surface: 0, faces: (0..n).collect::<Vec<_>>().into() }]
        } else {
            Vec::new()
        },
        ..Mesh::default()
    }
}

/// A flat square of `half` metres about the origin, its front up.
fn quad(half: f32) -> EditObject {
    let p = vec![
        Vec3::new(-half, 0.0, -half),
        Vec3::new(half, 0.0, -half),
        Vec3::new(half, 0.0, half),
        Vec3::new(-half, 0.0, half),
    ];
    let mut o = EditObject::default();
    o.surfaces.push(surface("ground", "terrain\\grass", "materials\\earth"));
    o.meshes.push(mesh(p, &[[0, 1, 2], [0, 2, 3]], true));
    o
}

/// A box of one metre about the origin, its faces' fronts out.
fn cube(surfaced: bool) -> EditObject {
    let p = vec![
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
    ];
    let idx = [
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [0, 4, 5],
        [0, 5, 1],
        [3, 2, 6],
        [3, 6, 7],
        [0, 3, 7],
        [0, 7, 4],
        [1, 5, 6],
        [1, 6, 2],
    ];
    let mut o = EditObject::default();
    if surfaced {
        o.surfaces.push(surface("box", "props\\wood", "materials\\wood"));
    }
    o.meshes.push(mesh(p, &idx, surfaced));
    o
}

/// The library of the test models, and how many times its provider was asked.
fn library() -> (Library, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let lib = Library::new(Vec::new());
    let models: Vec<(&str, Arc<LibraryObject>)> = vec![
        ("test\\ground", Arc::new(LibraryObject::provided("test\\ground", quad(50.0)))),
        ("test\\box", Arc::new(LibraryObject::provided("test\\box", cube(true)))),
        ("test\\wall", Arc::new(LibraryObject::provided("test\\wall", cube(false)))),
    ];
    let a = asked.clone();
    lib.add_provider(Arc::new(move |name: &str| {
        a.fetch_add(1, Ordering::Relaxed);
        models.iter().find(|(n, _)| *n == name).map(|(_, o)| o.clone())
    }));
    (lib, asked)
}

fn place(s: &mut SceneState, name: &str, reference: &str, at: Vec3, scale: Vec3) {
    let mut o =
        Object::new(ObjClass::SceneObject, name, ObjData::SceneObject(SceneObjectData::library(reference)));
    o.xform.position = at;
    o.xform.scale = scale;
    s.insert(o);
}

struct World {
    state: SceneState,
    lib: Library,
    pick: ScenePick,
}

fn world() -> World {
    let (lib, _) = library();
    let mut state = SceneState::default();
    place(&mut state, "ground", "test\\ground", Vec3::ZERO, Vec3::ONE);
    place(&mut state, "standing", "test\\box", Vec3::new(0.0, 0.5, 0.0), Vec3::ONE);
    place(&mut state, "floating", "test\\box", Vec3::new(5.0, 3.0, 0.0), Vec3::ONE);
    place(&mut state, "wall", "test\\wall", Vec3::new(10.0, 1.5, 0.0), Vec3::new(0.2, 3.0, 10.0));
    let mut shape = Object::new(ObjClass::Shape, "zone", ObjData::Shape(ShapeData::unit_box()));
    shape.xform.position = Vec3::new(0.0, 0.5, -20.0);
    state.insert(shape);
    for r in ["test\\ground", "test\\box", "test\\wall"] {
        assert!(lib.load(r).is_ok(), "{r}");
    }
    let pick = ScenePick::build(&state, &lib);
    World { state, lib, pick }
}

fn ask(w: &World, v: Value) -> Result<Value, gz_editor::CmdError> {
    let p: QueryParams = serde_json::from_value(v).expect("the parameters read");
    query(&Scene { state: &w.state, lib: &w.lib, pick: &w.pick }, &p)
}

fn ok(w: &World, v: Value) -> Value {
    ask(w, v).unwrap_or_else(|e| panic!("{}", e.message))
}

fn refused(w: &World, v: Value, words: &str) {
    let e = ask(w, v).expect_err("refused");
    assert_eq!(e.code, "invalid", "{}", e.message);
    assert!(e.message.contains(words), "{}", e.message);
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("a number, not {v}"))
}

fn near(v: &Value, want: f64) {
    assert!((num(v) - want).abs() < 1e-3, "{v} is not {want}");
}

#[test]
fn ground_names_the_floor_its_object_and_surface() {
    let w = world();
    let a = ok(&w, json!({ "action": "ground", "position": [20, 5, 20] }));
    near(&a["floor"], 0.0);
    assert_eq!(a["ceiling"], Value::Null);
    assert_eq!(a["normal"], json!([0.0, 1.0, 0.0]));
    near(&a["slope"], 0.0);
    assert_eq!(a["object"], "ground");
    assert_eq!(
        a["surface"],
        json!({ "name": "ground", "texture": "terrain\\grass", "material": "materials\\earth" })
    );

    let a = ok(&w, json!({ "action": "ground", "position": [0, 5, 0] }));
    near(&a["floor"], 1.0);
    assert_eq!(a["object"], "standing");
    assert_eq!(a["surface"]["material"], "materials\\wood");

    // under the floating box: the ground below, the box above; from the top: the box's roof
    let a = ok(&w, json!({ "action": "ground", "position": [5, 0.5, 0] }));
    near(&a["floor"], 0.0);
    near(&a["ceiling"], 2.5);
    let a = ok(&w, json!({ "action": "ground", "position": [5, 0.5, 0], "top": true }));
    near(&a["floor"], 3.5);
    assert_eq!(a["object"], "floating");

    // a shape is no ground, and a model without surfaces names none
    let a = ok(&w, json!({ "action": "ground", "position": [0, 5, -20] }));
    assert_eq!(a["object"], "ground");
    let a = ok(&w, json!({ "action": "ground", "position": [10, 5, 0] }));
    near(&a["floor"], 3.0);
    assert_eq!(a["object"], "wall");
    assert_eq!(a["surface"], Value::Null);

    let a = ok(&w, json!({ "action": "ground", "positions": [[20, 5, 20], [0, 5, 0]] }));
    let pts = a["points"].as_array().expect("points");
    assert_eq!(pts.len(), 2);
    assert_eq!(pts[0]["position"], json!([20.0, 5.0, 20.0]));
    near(&pts[1]["floor"], 1.0);

    let a = ok(&w, json!({ "action": "ground", "position": [0, 5, 0], "except": ["standing"] }));
    assert_eq!(a["object"], "ground");
    let a = ok(&w, json!({ "action": "ground", "position": [0, 5, 0], "only": ["ground"] }));
    near(&a["floor"], 0.0);

    let many: Vec<[f32; 3]> = vec![[0.0, 1.0, 0.0]; 1025];
    refused(&w, json!({ "action": "ground", "positions": many }), "1024");
    refused(&w, json!({ "action": "ground" }), "position");
    let e = ask(&w, json!({ "action": "ground", "position": [0, 5, 0], "only": ["nobody"] }))
        .expect_err("unknown");
    assert_eq!(e.code, "not_found");
}

#[test]
fn heights_are_a_grid_of_the_ground() {
    let w = world();
    let a = ok(&w, json!({ "action": "heights", "area": [-2, -2, 2, 2] }));
    assert_eq!(a["columns"], 5);
    assert_eq!(a["rows"], 5);
    let rows = a["heights"].as_array().expect("heights");
    near(&rows[2][2], 1.0);
    near(&rows[0][0], 0.0);
    near(&a["min"], 0.0);
    near(&a["max"], 1.0);

    let a = ok(&w, json!({ "action": "heights", "area": [-2, -2, 2, 2], "except": ["standing"] }));
    near(&a["max"], 0.0);
    // outside the ground: nothing
    let a = ok(&w, json!({ "action": "heights", "area": [60, 60, 61, 61], "step": 0.5 }));
    assert_eq!(a["heights"][0][0], Value::Null);
    assert_eq!(a["min"], Value::Null);

    refused(&w, json!({ "action": "heights", "area": [0, 0, 300, 1] }), "257 x 257");
    refused(&w, json!({ "action": "heights", "area": [0, 0, 1, 1], "step": 0 }), "step");
    let _ = ok(&w, json!({ "action": "heights", "area": [0, 0, 256, 256] }));
}

#[test]
fn a_ray_names_what_it_hits() {
    let w = world();
    let a = ok(&w, json!({ "action": "ray", "from": [-20, 0.5, 0], "to": [20, 0.5, 0] }));
    let h = &a["hit"];
    assert_eq!(h["object"], "standing");
    assert_eq!(h["kind"], "object");
    near(&h["distance"], 19.5);
    assert_eq!(h["normal"], json!([-1.0, 0.0, 0.0]));
    assert_eq!(h["surface"]["name"], "box");

    let a = ok(&w, json!({ "action": "ray", "from": [-20, 0.5, 0], "to": [20, 0.5, 0], "all": true }));
    let names: Vec<&str> =
        a["hits"].as_array().expect("hits").iter().filter_map(|h| h["object"].as_str()).collect();
    assert_eq!(names, ["standing", "wall"]);
    assert_eq!(a["total"], 2);

    let a = ok(&w, json!({ "action": "ray", "from": [-20, 0.5, 0], "to": [20, 0.5, 0], "only": ["wall"] }));
    near(&a["hit"]["distance"], 29.9);
    let a =
        ok(&w, json!({ "action": "ray", "from": [-20, 0.5, 0], "to": [20, 0.5, 0], "except": ["standing"] }));
    assert_eq!(a["hit"]["object"], "wall");

    // a shape is geometry for a ray
    let a = ok(&w, json!({ "action": "ray", "from": [0, 0.5, -30], "direction": [0, 0, 1], "reach": 20 }));
    assert_eq!(a["hit"]["kind"], "shape");
    let a = ok(&w, json!({ "action": "ray", "from": [0, 0.5, -30], "direction": [0, 0, 1], "reach": 5 }));
    assert_eq!(a["hit"], Value::Null);

    refused(&w, json!({ "action": "ray", "from": [0, 0, 0], "to": [0, 0, 0] }), "direction");
    refused(&w, json!({ "action": "ray", "from": [0, 0, 0] }), "to");
}

#[test]
fn a_box_lists_what_meets_it() {
    let w = world();
    let a = ok(&w, json!({ "action": "box", "min": [-1, -1, -1], "max": [1, 2, 1] }));
    let names: Vec<&str> =
        a["objects"].as_array().expect("objects").iter().filter_map(|o| o["name"].as_str()).collect();
    assert_eq!(names, ["ground", "standing"]);
    assert_eq!(a["total"], 2);
    assert_eq!(a["objects"][1]["ref"], "test\\box");
    assert_eq!(a["objects"][1]["bounds"], json!({ "min": [-0.5, 0.0, -0.5], "max": [0.5, 1.0, 0.5] }));

    // inside the standing box: its box meets, its faces do not
    let inner = json!({ "action": "box", "min": [-0.2, 0.3, -0.2], "max": [0.2, 0.6, 0.2] });
    assert_eq!(ok(&w, inner.clone())["total"], 1);
    let mut exact = inner;
    exact["exact"] = json!(true);
    assert_eq!(ok(&w, exact)["total"], 0);

    let a = ok(
        &w,
        json!({ "action": "box", "position": [5, 3, 0], "size": [0.4, 0.4, 0.4], "rotation": [0, 45, 0] }),
    );
    assert_eq!(a["objects"][0]["name"], "floating");
    refused(&w, json!({ "action": "box", "min": [0, 0, 0] }), "min and max");
}

#[test]
fn fit_says_what_blocks_and_where_it_rests() {
    let w = world();
    let a = ok(&w, json!({ "action": "fit", "size": [1, 1, 1], "position": [3, 0, 0] }));
    assert_eq!(a["fits"], true);
    assert_eq!(a["blocked_by"], json!([]));
    assert_eq!(a["ground"], json!({ "min": 0.0, "max": 0.0, "centre": 0.0 }));
    near(&a["gap"], 0.0);
    assert_eq!(a["rest"], json!([3.0, 0.0, 0.0]));
    near(&a["tilt"], 0.0);

    let a = ok(&w, json!({ "action": "fit", "size": [1, 1, 1], "position": [0.3, 0, 0] }));
    assert_eq!(a["fits"], false);
    assert_eq!(a["blocked_by"][0]["object"], "standing");
    near(&a["blocked_by"][0]["depth"], 0.3);
    let a = ok(
        &w,
        json!({ "action": "fit", "size": [1, 1, 1], "position": [0.3, 0, 0], "except": ["standing"] }),
    );
    assert_eq!(a["fits"], true);

    // touching the floating box from below is no meeting; 0.2 m into it is
    let a = ok(&w, json!({ "action": "fit", "ref": "test\\box", "position": [5, 2, 0] }));
    assert_eq!(a["fits"], true);
    let a = ok(&w, json!({ "action": "fit", "ref": "test\\box", "position": [5, 2.2, 0] }));
    assert_eq!(a["fits"], false);
    assert_eq!(a["blocked_by"][0]["object"], "floating");
    near(&a["blocked_by"][0]["depth"], 0.2);

    // the floating box at its place meets nothing but floats 2.5 m up
    let a = ok(&w, json!({ "action": "fit", "object": "floating" }));
    assert_eq!(a["fits"], true);
    near(&a["gap"], 2.5);
    assert_eq!(a["rest"], json!([5.0, 0.5, 0.0]));
    // sunk into the ground it is blocked by it
    let a = ok(&w, json!({ "action": "fit", "object": "floating", "position": [5, 0.2, 0] }));
    assert_eq!(a["blocked_by"][0]["object"], "ground");
    near(&a["blocked_by"][0]["depth"], 0.3);

    refused(&w, json!({ "action": "fit", "size": [1, 1, 1] }), "position");
    refused(&w, json!({ "action": "fit", "position": [0, 0, 0] }), "one of ref");
    let e =
        ask(&w, json!({ "action": "fit", "ref": "test\\none", "position": [0, 0, 0] })).expect_err("unknown");
    assert_eq!(e.code, "not_found");
}

#[test]
fn free_spots_are_ones_fit_accepts() {
    let w = world();
    // too wide to stand on the standing box: every spot is on the ground
    let a = ok(
        &w,
        json!({ "action": "free", "size": [1.5, 1, 1.5], "position": [0, 0, 0], "radius": 4, "count": 3 }),
    );
    let spots = a["spots"].as_array().expect("spots");
    assert_eq!(spots.len(), 3);
    let mut last = 0.0;
    for s in spots {
        let d = num(&s["distance"]);
        assert!(d >= last && d <= 4.0, "{s}");
        last = d;
        let at = s["position"].clone();
        near(&at[1], 0.0);
        let f = ok(&w, json!({ "action": "fit", "size": [1.5, 1, 1.5], "position": at }));
        assert_eq!(f["fits"], true, "{s}: {f}");
        near(&f["gap"], 0.0);
    }
    // nearest first: the standing box is in the way of the very middle
    assert!(num(&spots[0]["distance"]) > 0.5);
    // spots keep their spacing
    let (p0, p1) = (&spots[0]["position"], &spots[1]["position"]);
    let apart = (num(&p0[0]) - num(&p1[0])).hypot(num(&p0[2]) - num(&p1[2]));
    assert!(apart >= 1.5 - 1e-3, "{apart}");
    // something small stands on the box's top
    let a = ok(
        &w,
        json!({ "action": "free", "size": [0.5, 0.5, 0.5], "position": [0, 1, 0], "radius": 0.2, "count": 1 }),
    );
    assert_eq!(a["spots"], json!([{ "position": [0.0, 1.0, 0.0], "distance": 0.0 }]));

    refused(&w, json!({ "action": "free", "size": [1, 1, 1], "position": [0, 0, 0], "radius": 51 }), "50");
    refused(&w, json!({ "action": "free", "size": [1, 1, 1], "position": [0, 0, 0], "count": 17 }), "16");
    // nothing to stand on
    let a = ok(&w, json!({ "action": "free", "size": [1, 1, 1], "position": [100, 0, 100], "radius": 5 }));
    assert_eq!(a["spots"], json!([]));
}

#[test]
fn the_ai_map_answers_points_and_walks() {
    let mut w = world();
    let mut nodes = vec![
        AiMapNode::new(Vec3::new(0.0, 0.0, 0.0), Vec3::Y),
        AiMapNode::new(Vec3::new(0.7, 0.0, 0.0), Vec3::Y),
        AiMapNode::new(Vec3::new(1.4, 0.0, 0.0), Vec3::Y),
        AiMapNode::new(Vec3::new(10.0, 0.0, 10.0), Vec3::Y),
    ];
    nodes[0].links[2] = Some(1);
    nodes[1].links = [Some(0), None, Some(2), None];
    nodes[2].links[0] = Some(1);
    let header = SceneAiMap { bbox: gz_fmt_level::nodes_bbox(&nodes), ..SceneAiMap::default() };
    w.state.bulk.ai_map = Some(Arc::new(AiMapPart::from_nodes(header, &nodes, None)));

    let a = ok(&w, json!({ "action": "ai", "positions": [[0.1, 0.2, 0], [5, 0, 5]] }));
    assert_eq!(a["nodes"], 4);
    assert_eq!(a["points"][0]["on_map"], true);
    assert_eq!(a["points"][0]["node"], json!([0.0, 0.0, 0.0]));
    assert_eq!(a["points"][1]["on_map"], false);

    let a = ok(&w, json!({ "action": "ai", "from": [0, 0, 0], "to": [1.4, 0, 0] }));
    assert_eq!(a["reachable"], true);
    near(&a["length"], 1.4);
    let a = ok(&w, json!({ "action": "ai", "from": [0, 0, 0], "to": [10, 0, 10] }));
    assert_eq!(a["reachable"], false);
    assert_eq!(a["length"], Value::Null);

    refused(&w, json!({ "action": "ai", "from": [0, 0, 0] }), "positions");
}

#[test]
fn the_library_asks_providers_and_forgets() {
    let (lib, asked) = library();
    let rev = lib.rev();
    assert!(lib.cached("test\\box").is_none());
    let o = lib.load("TEST/Box").expect("provided");
    assert_eq!(o.object.surfaces[0].name, "box");
    assert_eq!(o.bounds.min, Vec3::splat(-0.5));
    assert_eq!(asked.load(Ordering::Relaxed), 1);
    assert_ne!(lib.rev(), rev);
    assert!(lib.cached("test\\box").is_some());
    let _ = lib.load("test\\box");
    assert_eq!(asked.load(Ordering::Relaxed), 1, "cached: not asked again");

    let rev = lib.rev();
    assert!(lib.forget("test/box"));
    assert_ne!(lib.rev(), rev);
    assert!(lib.cached("test\\box").is_none());
    assert!(!lib.forget("test\\box"), "nothing left to forget");
    let _ = lib.load("test\\box");
    assert_eq!(asked.load(Ordering::Relaxed), 2, "read again after forget");

    // a reference nobody provides falls through to the roots, and fails there
    assert!(lib.load("test\\nothing").is_err());
}

#[test]
fn a_hit_carries_its_surface() {
    let w = world();
    let pick = &w.pick;
    let hit = crate::pick::ray_scene_objects(
        pick,
        &w.state,
        &w.lib,
        Vec3::new(0.0, 5.0, 0.0),
        Vec3::NEG_Y,
        30.0,
        None,
    )
    .expect("a hit");
    assert_eq!(hit.surface, Some(0));
    let wall = crate::pick::ray_scene_objects(
        pick,
        &w.state,
        &w.lib,
        Vec3::new(10.0, 5.0, 0.0),
        Vec3::NEG_Y,
        30.0,
        None,
    )
    .expect("a hit");
    assert_eq!(wall.surface, None);
}
