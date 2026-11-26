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

    // a volume is in nobody's way: a shape is met only by a ray that asks for the volumes
    let a = ok(&w, json!({ "action": "ray", "from": [0, 0.5, -30], "direction": [0, 0, 1], "reach": 20 }));
    assert_eq!(a["hit"], Value::Null);
    let a = ok(
        &w,
        json!({ "action": "ray", "from": [0, 0.5, -30], "direction": [0, 0, 1], "reach": 20, "volumes": true }),
    );
    assert_eq!(a["hit"]["kind"], "shape");
    let a = ok(
        &w,
        json!({ "action": "ray", "from": [0, 0.5, -30], "direction": [0, 0, 1], "reach": 5, "volumes": true }),
    );
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

fn map_of(w: &World, v: Value) -> Result<Value, gz_editor::CmdError> {
    let p: super::shot::MapParams = serde_json::from_value(v).expect("the parameters read");
    super::shot::shot_map(&Scene { state: &w.state, lib: &w.lib, pick: &w.pick }, &p)
}

#[test]
fn the_map_shows_the_scene_from_above_and_reads_back_in_metres() {
    let w = world();
    let a = map_of(
        &w,
        json!({
            "area": [-8, -8, 8, 8], "px": 320, "highlight": ["floating"],
            "marks": [{ "position": [-5, 0, -5], "text": "here" }],
        }),
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    // the picture is the map and its two margins; a pixel reads back to metres
    assert_eq!(a["width"], 320 + 46);
    assert_eq!(a["height"], 320 + 14);
    near(&a["px_per_m"], 20.0);
    assert_eq!(a["map"], json!({ "left": 46, "top": 14, "width": 320, "height": 320 }));
    assert_eq!(a["area"], json!([-8.0, -8.0, 8.0, 8.0]));
    assert_eq!(a["mode"], "objects");
    assert!(a["png_base64"].as_str().is_some_and(|s| s.starts_with("iVBORw0KGgo")));
    // the legend names what the rays met: the wall stands outside the rectangle
    let legend = a["legend"].as_array().expect("a legend");
    let row = |name: &str| legend.iter().find(|r| r["object"] == name);
    assert!(row("wall").is_none());
    let standing = row("standing").expect("the box on the ground");
    near(&standing["centre"][0], 0.0);
    near(&standing["centre"][1], 0.0);
    assert!((num(&standing["area"]) - 1.0).abs() < 0.11, "{}", standing["area"]);
    assert_eq!(standing["named"], true);
    let floating = row("floating").expect("the box in the air");
    near(&floating["centre"][0], 5.0);
    assert_eq!(floating["colour"], "#ff40a0");
    let ground = row("ground").expect("the ground");
    assert_eq!(ground["named"], false);
    assert_eq!(ground["kind"], "object");
    // from the ground to the top of the box in the air
    near(&a["heights"]["min"], 0.0);
    near(&a["heights"]["max"], 3.5);
    // the same scene, the same picture
    let b = map_of(
        &w,
        json!({
            "area": [-8, -8, 8, 8], "px": 320, "highlight": ["floating"],
            "marks": [{ "position": [-5, 0, -5], "text": "here" }],
        }),
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a["png_base64"], b["png_base64"]);
}

#[test]
fn the_map_takes_a_middle_and_a_size_and_says_its_heights() {
    let w = world();
    let a = map_of(
        &w,
        json!({ "position": [0, 0, 0], "size": [16, 8], "px": 160, "mode": "height", "labels": false }),
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a["map"]["width"], 160);
    assert_eq!(a["map"]["height"], 80);
    assert_eq!(a["area"], json!([-8.0, -4.0, 8.0, 4.0]));
    assert_eq!(a["mode"], "height");
    assert!(num(&a["heights"]["contour"]) > 0.0);
    // only one object seen from above: the others are not there
    let a = map_of(&w, json!({ "position": [0, 0, 0], "size": 16, "px": 64, "only": ["standing"] }))
        .unwrap_or_else(|e| panic!("{}", e.message));
    let legend = a["legend"].as_array().expect("a legend");
    assert_eq!(legend.len(), 1);
    assert_eq!(legend[0]["object"], "standing");
    // the AI map's nodes are counted where there is a map
    let a = map_of(&w, json!({ "position": [0, 0, 0], "size": 16, "px": 64, "mode": "ai" }))
        .unwrap_or_else(|e| panic!("{}", e.message));
    assert!(a.get("ai_nodes").is_some());
}

#[test]
fn a_map_that_cannot_be_drawn_is_refused_in_a_sentence() {
    let w = world();
    let refused = |v: Value, words: &str| {
        let e = map_of(&w, v).expect_err("refused");
        assert!(e.message.contains(words), "{}", e.message);
        e.code
    };
    assert_eq!(refused(json!({ "area": [0, 0, 5, 5], "position": [0, 0, 0] }), "give one"), "invalid");
    assert_eq!(refused(json!({ "area": [0, 0, 5, 5], "size": 4 }), "size goes with position"), "invalid");
    assert_eq!(refused(json!({ "position": [0, 0, 0] }), "takes size"), "invalid");
    assert_eq!(refused(json!({ "position": [0, 0, 0], "size": [1, 2, 3] }), "one number"), "invalid");
    assert_eq!(refused(json!({ "area": [0, 0, 5, 5], "px": 4000 }), "px takes"), "invalid");
    assert_eq!(refused(json!({ "area": [0, 0, 5, 5], "px": 8 }), "px takes"), "invalid");
    assert_eq!(refused(json!({ "area": [0, 0, 0.1, 5] }), "half a metre"), "invalid");
    assert_eq!(refused(json!({ "area": [0, 0, 90000, 5] }), "at most"), "invalid");
    assert_eq!(refused(json!({ "area": [0, 0, 5, 5], "grid": 0 }), "grid takes"), "invalid");
    // a strip so narrow that its short side would be drawn wider than it is
    assert_eq!(refused(json!({ "area": [0, 0, 2000, 5], "px": 512 }), "under 16 px"), "invalid");
    assert_eq!(refused(json!({ "area": [0, 0, 5, 5], "highlight": ["nobody"] }), "nobody"), "not_found");
    // the sizes of a rectangle that is not square
    assert_eq!(super::shot::size_px([0.0, 0.0, 10.0, 5.0], 200), [200, 100]);
    assert_eq!(super::shot::size_px([0.0, 0.0, 5.0, 10.0], 200), [100, 200]);
    assert_eq!(super::shot::size_px([0.0, 0.0, 1000.0, 1.0], 200), [200, 16]);
}

// ---- scene.audit ----------------------------------------------------------------------------

fn audit_of(w: &World, v: Value, base: Option<&[&str]>) -> Result<super::audit::Audit, gz_editor::CmdError> {
    let p: super::audit::AuditParams = serde_json::from_value(v).expect("the parameters read");
    let base: Option<rustc_hash::FxHashSet<String>> =
        base.map(|b| b.iter().map(|s| (*s).to_owned()).collect());
    super::audit::audit(&Scene { state: &w.state, lib: &w.lib, pick: &w.pick }, &p, base.as_ref())
}

/// The findings as `code object` lines, in order.
fn found(w: &World, v: Value) -> Vec<String> {
    let a = audit_of(w, v, None).unwrap_or_else(|e| panic!("{}", e.message));
    a.findings
        .iter()
        .map(|f| format!("{} {}", f.code, w.state.get(f.object).map_or("?", |o| o.name.as_str())))
        .collect()
}

/// The world with more in it; the pick tree made again.
fn world_with(add: impl FnOnce(&mut SceneState)) -> World {
    let mut w = world();
    add(&mut w.state);
    w.pick = ScenePick::build(&w.state, &w.lib);
    w
}

#[test]
fn the_audit_finds_what_floats_and_leaves_what_stands() {
    let w = world();
    // the box in the air floats; the box on the ground, the wall and the ground do not
    let a = audit_of(&w, json!({}), None).unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a.checked, 5);
    assert_eq!(a.findings.len(), 1, "{:?}", a.findings);
    assert_eq!(a.findings[0].code, "S201");
    assert_eq!(a.findings[0].text, "floating floats 2.50 m above ground: nothing holds it");
    // a box hung under something, or leaning on something, is held
    let w = world_with(|s| {
        place(s, "lamp", "test\\box", Vec3::new(5.0, 1.9, 0.0), Vec3::ONE);
        place(s, "shelf", "test\\box", Vec3::new(9.4, 2.0, 0.0), Vec3::ONE);
    });
    // the lamp hangs 0.1 m under the box in the air; the shelf touches the wall
    assert_eq!(found(&w, json!({ "objects": ["lamp", "shelf"] })), Vec::<String>::new());
    // off the ground's edge there is nothing to stand on
    let w = world_with(|s| place(s, "lost", "test\\box", Vec3::new(500.0, 5.0, 500.0), Vec3::ONE));
    assert_eq!(found(&w, json!({ "objects": ["lost"] })), ["S205 lost"]);
    assert_eq!(found(&w, json!({ "objects": ["lost"], "checks": ["floating"] })), Vec::<String>::new());
}

#[test]
fn the_audit_finds_what_is_buried_what_overlaps_and_what_is_twice_there() {
    let w = world_with(|s| {
        // its middle 0.2 m under the ground
        place(s, "sunk", "test\\box", Vec3::new(20.0, -0.2, 0.0), Vec3::ONE);
        // a pole a third in the ground is as it should be
        place(s, "pole", "test\\box", Vec3::new(25.0, 0.2, 0.0), Vec3::ONE);
        // two boxes, one 0.7 m into the other
        place(s, "left", "test\\box", Vec3::new(30.0, 0.5, 0.0), Vec3::ONE);
        place(s, "right", "test\\box", Vec3::new(30.3, 0.5, 0.0), Vec3::ONE);
        // two boxes that only touch
        place(s, "a", "test\\box", Vec3::new(35.0, 0.5, 0.0), Vec3::ONE);
        place(s, "b", "test\\box", Vec3::new(36.0, 0.5, 0.0), Vec3::ONE);
        // one box twice
        place(s, "twin", "test\\box", Vec3::new(40.0, 0.5, 0.0), Vec3::ONE);
        place(s, "twin_again", "test\\box", Vec3::new(40.0, 0.5, 0.0), Vec3::ONE);
    });
    assert_eq!(found(&w, json!({ "objects": ["sunk", "pole"] })), ["S202 sunk"]);
    assert_eq!(found(&w, json!({ "objects": ["left", "right", "a", "b"] })), ["S203 left"]);
    // the pair is said once, from either side
    assert_eq!(found(&w, json!({ "objects": ["right"], "checks": ["S203"] })), ["S203 right"]);
    assert_eq!(
        found(&w, json!({ "objects": ["twin", "twin_again"], "checks": ["duplicate"] })),
        ["S204 twin_again"]
    );
    // an area takes what stands in it
    assert_eq!(found(&w, json!({ "area": [19, -1, 21, 1] })), ["S202 sunk"]);
}

#[test]
fn the_audit_finds_points_under_the_ground_and_ways_off_the_ai_map() {
    use crate::objects::{PointKind, SpawnData, WayData, WayPoint};
    let mut w = world_with(|s| {
        let mut under = Object::new(
            ObjClass::SpawnPoint,
            "deep",
            ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::Spawn))),
        );
        under.xform.position = Vec3::new(15.0, -5.0, 15.0);
        s.insert(under);
        let mut fine = Object::new(
            ObjClass::SpawnPoint,
            "fine",
            ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::Spawn))),
        );
        fine.xform.position = Vec3::new(15.0, 0.1, 18.0);
        s.insert(fine);
        let mut start = Object::new(
            ObjClass::SpawnPoint,
            "start",
            ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::PlayerStart))),
        );
        start.xform.position = Vec3::new(-30.0, 0.1, -30.0);
        s.insert(start);
        let way = WayData {
            kind: 0,
            points: vec![
                WayPoint::new("p0", Vec3::new(0.2, 0.0, 0.1)),
                WayPoint::new("p1", Vec3::new(8.0, 0.0, 8.0)),
            ],
        };
        s.insert(Object::new(ObjClass::Way, "walk", ObjData::Way(way)));
    });
    // no AI map: the checks that need one are named as skipped when they were asked for
    let a = audit_of(&w, json!({ "checks": ["off_ai_map", "underground"] }), None)
        .unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a.findings.len(), 1);
    assert_eq!(
        (a.findings[0].code, a.findings[0].text.as_str()),
        ("S207", "deep is under ground: nothing is under it within 50 m")
    );
    assert_eq!(a.skipped, ["S206, S208: the scene has no AI map"]);
    let nodes = vec![
        AiMapNode::new(Vec3::new(0.0, 0.0, 0.0), Vec3::Y),
        AiMapNode::new(Vec3::new(0.7, 0.0, 0.0), Vec3::Y),
    ];
    let header = SceneAiMap { bbox: gz_fmt_level::nodes_bbox(&nodes), ..SceneAiMap::default() };
    w.state.bulk.ai_map = Some(Arc::new(AiMapPart::from_nodes(header, &nodes, None)));
    // the player's start is a walker off the map; the way's second point is off it too
    let lines = found(&w, json!({ "checks": ["off_ai_map", "way_off_ai_map"] }));
    assert_eq!(lines, ["S206 start", "S208 walk"]);
    let a = audit_of(&w, json!({ "objects": ["walk"] }), None).unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a.findings[0].text, "walk: p1 off the AI map");
}

#[test]
fn the_audit_names_what_the_game_s_level_lacks_and_refuses_what_it_cannot_do() {
    let w = world();
    // against the game's own objects: what is neither there nor marked is not shipped
    let base = ["ground", "standing", "wall", "zone"];
    let a = audit_of(&w, json!({ "checks": ["not_authored"] }), Some(&base))
        .unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a.findings.len(), 1);
    assert_eq!(a.findings[0].code, "S210");
    assert!(a.findings[0].text.starts_with("floating is not in the game's level"));
    // marked, it is the module's; and then the module's own objects are what an audit reads
    let w = world_with(|s| {
        let mut o = Object::new(
            ObjClass::SceneObject,
            "mine",
            ObjData::SceneObject(SceneObjectData::library("test\\box")),
        );
        o.xform.position = Vec3::new(-20.0, 3.0, 0.0);
        o.marks.authored = true;
        s.insert(o);
    });
    let a = audit_of(&w, json!({}), Some(&base)).unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a.checked, 1);
    assert_eq!(a.findings.iter().map(|f| f.code).collect::<Vec<_>>(), ["S201"]);
    // without the game's level the check is skipped, and says so when it was asked for
    let a = audit_of(&w, json!({ "checks": ["S210"] }), None).unwrap_or_else(|e| panic!("{}", e.message));
    assert!(a.findings.is_empty());
    assert_eq!(a.skipped.len(), 1);
    let refused = |v: Value, words: &str| {
        let e = audit_of(&w, v, None).expect_err("refused");
        assert!(e.message.contains(words), "{}", e.message);
        e.code
    };
    assert_eq!(refused(json!({ "checks": ["tidy"] }), "not 'tidy'"), "invalid");
    assert_eq!(refused(json!({ "objects": ["floating"], "area": [0, 0, 1, 1] }), "give one"), "invalid");
    assert_eq!(refused(json!({ "objects": ["nobody"] }), "nobody"), "not_found");
}

/// S214: a radio bound to a playlist of the module that mod.ltx lacks; one it declares, the
/// game's music, silence and another module's playlist pass; without the module's playlists the
/// check does not run.
#[test]
fn the_audit_names_a_radio_bound_to_a_playlist_the_module_lacks() {
    use crate::objects::radio::{RadioCx, RadioPlaylist, Radios};
    use crate::objects::{PointKind, SpawnData};
    let w = world_with(|s| {
        for (name, plays) in [
            ("lost", "gone"),
            ("road", "road"),
            ("keyed", "my_mod.road"),
            ("game", "game"),
            ("other", "other.x"),
        ] {
            let mut d = SpawnData::of_kind(PointKind::Spawn);
            d.plays = plays.into();
            let mut o = Object::new(ObjClass::SpawnPoint, name, ObjData::Spawn(Box::new(d)));
            o.xform.position = Vec3::new(15.0, 0.1, 18.0);
            s.insert(o);
        }
    });
    let radio = RadioCx {
        radios: Arc::new(Radios::unlinked()),
        module: "my_mod".into(),
        playlists: Arc::from(vec![RadioPlaylist { id: "road".into(), title: "Road songs".into() }]),
    };
    let p: super::audit::AuditParams =
        serde_json::from_value(json!({ "checks": ["radio_playlist"] })).expect("read");
    let sc = Scene { state: &w.state, lib: &w.lib, pick: &w.pick };
    let a = super::audit::audit_with(&sc, &p, None, Some(&radio), None)
        .unwrap_or_else(|e| panic!("{}", e.message));
    let names: Vec<&str> =
        a.findings.iter().filter_map(|f| w.state.get(f.object).map(|o| o.name.as_str())).collect();
    assert_eq!(names, ["lost"], "{:?}", a.findings);
    assert_eq!(a.findings[0].code, "S214");
    assert!(a.findings[0].text.starts_with("lost plays the playlist 'gone', which mod.ltx does not declare"));
    assert!(found(&w, json!({ "checks": ["S214"] })).is_empty());
}

/// S218 and S219: a stash whose model has no unseen case, and a stash that is scaled; a stash
/// with its case and a scaled object that is no stash are left alone, and without the game's
/// models S218 does not run.
#[test]
fn the_audit_names_a_stash_without_its_case_and_a_scaled_one() {
    use crate::objects::Stash;
    /// The game's models: every case but the lost box's.
    struct Models;
    impl crate::names::Names for Models {
        fn has(&self, kind: &str, name: &str) -> Option<bool> {
            (kind == "model").then(|| name != "test\\lost_case.ogf")
        }

        fn nearest(&self, _: &str, _: &str) -> Vec<String> {
            Vec::new()
        }
    }
    let w = world_with(|s| {
        for (name, reference, scale, stash) in [
            ("cache", "test\\box", 1.0, true),
            ("lost", "test\\lost", 1.0, true),
            ("big", "test\\box", 1.5, true),
            ("crate", "test\\lost", 2.0, false),
        ] {
            let d =
                SceneObjectData { stash: stash.then(Stash::default), ..SceneObjectData::library(reference) };
            let mut o = Object::new(ObjClass::SceneObject, name, ObjData::SceneObject(d));
            o.xform.position = Vec3::new(30.0, 0.0, 30.0);
            o.xform.scale = Vec3::splat(scale);
            o.marks.authored = true;
            s.insert(o);
        }
    });
    let p: super::audit::AuditParams =
        serde_json::from_value(json!({ "checks": ["stash_no_case", "stash_scaled"] })).expect("read");
    let sc = Scene { state: &w.state, lib: &w.lib, pick: &w.pick };
    let a = super::audit::audit_with(&sc, &p, None, None, Some(&Models))
        .unwrap_or_else(|e| panic!("{}", e.message));
    let lines: Vec<String> = a
        .findings
        .iter()
        .map(|f| format!("{} {}", f.code, w.state.get(f.object).map_or("?", |o| o.name.as_str())))
        .collect();
    assert_eq!(lines, ["S218 lost", "S219 big"], "{:?}", a.findings);
    assert_eq!(
        a.findings[0].text,
        "lost is a stash, and its model has no unseen case 'test\\lost_case' in the game or the project: the stash opens by inventory_box's own case box (0.94 x 0.43 x 0.48 m), which the build uses instead"
    );
    assert!(
        a.findings[1].text.starts_with("big is a stash and is scaled (1.50 x 1.50 x 1.50)"),
        "{}",
        a.findings[1].text
    );
    // without the game's models nobody says a case is missing
    let a = super::audit::audit_with(&sc, &p, None, None, None).unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(a.findings.len(), 1);
    assert_eq!(a.skipped, ["S218: the game library's models are not at hand"]);
}
