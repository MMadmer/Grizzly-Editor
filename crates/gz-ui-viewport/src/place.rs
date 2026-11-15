//! What can be placed in the scene and how it gets there: a drag dropped on the viewport, or a
//! click of Place mode with something armed. Both land on the surface under the pointer (the
//! ground pick, with the snapping that is on) and place through `scene.place`, one undo step,
//! and the placed thing is selected.
//!
//! The drag payloads the viewport takes (the kit's `gz_ui::drop`):
//! - `Row { surface: "place", key }` - a row of the Place panel; `key` is `<kind>` or
//!   `<kind>:<reference>` ([`Placeable::key`]); a shape's reference is its form or its use
//!   (`shape:box`, `shape:ai_map_bounds`);
//! - `Assets` - the Content Browser's entries: an `.object`, a `.group`, a particle effect
//!   (`.pe`, `.pg`), a sound (`.ogg`, `.wav`), a spawn section (kind `spawn_section`, its path
//!   the section).

use std::sync::Arc;

use glam::Vec3;
use gz_editor::{Args, Editor, Value, json};
use gz_render::Ray;
use gz_scene::{ObjClass, SceneService};
use gz_ui::drop::Payload;

use crate::service::Viewport;

/// The surface name of the Place panel's rows.
pub const SURFACE: &str = "place";

/// The uses of a shape a placeable names as its reference (`shape:ai_map_bounds`): a box that
/// `scene.place` makes with that `usage`.
pub const SHAPE_USES: [&str; 2] = ["ai_map_bounds", "level_bound"];

/// A kind of thing `scene.place` places.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Object,
    Group,
    Light,
    Glow,
    Sound,
    SoundEnv,
    Particles,
    Shape,
    Fog,
    Spawn,
    Way,
    Terrain,
}

impl Kind {
    pub const ALL: [Kind; 12] = [
        Kind::Object,
        Kind::Group,
        Kind::Light,
        Kind::Glow,
        Kind::Sound,
        Kind::SoundEnv,
        Kind::Particles,
        Kind::Shape,
        Kind::Fog,
        Kind::Spawn,
        Kind::Way,
        Kind::Terrain,
    ];

    /// The word `scene.place kind=` takes.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Kind::Object => "object",
            Kind::Group => "group",
            Kind::Light => "light",
            Kind::Glow => "glow",
            Kind::Sound => "sound",
            Kind::SoundEnv => "sound_env",
            Kind::Particles => "particles",
            Kind::Shape => "shape",
            Kind::Fog => "fog",
            Kind::Spawn => "spawn",
            Kind::Way => "way",
            Kind::Terrain => "terrain",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Kind> {
        let w = word.trim().to_ascii_lowercase();
        Kind::ALL.into_iter().find(|k| k.key() == w)
    }

    #[must_use]
    pub fn class(self) -> ObjClass {
        match self {
            Kind::Object => ObjClass::SceneObject,
            Kind::Group => ObjClass::Group,
            Kind::Light => ObjClass::Light,
            Kind::Glow => ObjClass::Glow,
            Kind::Sound => ObjClass::SoundSrc,
            Kind::SoundEnv => ObjClass::SoundEnv,
            Kind::Particles => ObjClass::Particles,
            Kind::Shape => ObjClass::Shape,
            Kind::Fog => ObjClass::FogVolume,
            Kind::Spawn => ObjClass::SpawnPoint,
            Kind::Way => ObjClass::Way,
            Kind::Terrain => ObjClass::Terrain,
        }
    }

    /// The kind of a class, for those a click can place.
    #[must_use]
    pub fn of_class(class: ObjClass) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.class() == class)
    }

    /// It needs a reference (a library object, a group, a sound, an effect) or a section.
    #[must_use]
    pub fn needs_reference(self) -> bool {
        matches!(self, Kind::Object | Kind::Group | Kind::Sound | Kind::Particles | Kind::Spawn)
    }
}

/// A thing to place: its kind and, for a kind that takes one, its reference (a spawn's is its
/// section; a shape's `box` makes a box instead of a sphere).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placeable {
    pub kind: Kind,
    pub reference: Option<String>,
}

impl Placeable {
    #[must_use]
    pub fn new(kind: Kind, reference: Option<&str>) -> Self {
        Self {
            kind,
            reference: reference.map(str::trim).filter(|r| !r.is_empty()).map(|r| r.replace('/', "\\")),
        }
    }

    /// `<kind>` or `<kind>:<reference>`: the Place panel's row key and `viewport.gesture`'s payload.
    #[must_use]
    pub fn key(&self) -> String {
        match &self.reference {
            Some(r) => format!("{}:{r}", self.kind.key()),
            None => self.kind.key().to_owned(),
        }
    }

    /// A key as [`Self::key`] writes it.
    #[must_use]
    pub fn parse(key: &str) -> Option<Placeable> {
        let (kind, reference) = match key.split_once(':') {
            Some((k, r)) => (k, Some(r)),
            None => (key, None),
        };
        Some(Placeable::new(Kind::parse(kind)?, reference))
    }

    /// The payload a row of the Place panel drags.
    #[must_use]
    pub fn payload(&self) -> Payload {
        Payload::Row { surface: SURFACE.to_owned(), key: self.key() }
    }

    /// What a drag payload carries that the viewport can place.
    #[must_use]
    pub fn of_payload(p: &Payload) -> Vec<Placeable> {
        match p {
            Payload::Row { surface, key } if surface == SURFACE => {
                Placeable::parse(key).into_iter().collect()
            }
            Payload::Assets(list) => list.iter().filter_map(|a| of_asset(&a.kind, &a.path)).collect(),
            _ => Vec::new(),
        }
    }

    /// The name it goes by in the viewport's hint: its reference's last part, else its kind.
    #[must_use]
    pub fn title(&self) -> String {
        match &self.reference {
            Some(r) => r.rsplit(['\\', '/']).next().unwrap_or(r).to_owned(),
            None => self.kind.class().label().to_owned(),
        }
    }

    /// [`Placeable::title`] as the window shows it: a reference is data, a kind the editor's word.
    #[must_use]
    pub fn title_tx(&self) -> gz_ui::Tx<'static> {
        match &self.reference {
            Some(_) => gz_ui::Tx::raw(self.title()),
            None => gz_ui::Tx::from(self.kind.class().label()),
        }
    }

    /// The arguments of `scene.place` for it at `position`, turned by `rotation` degrees and
    /// scaled by `scale`.
    #[must_use]
    pub fn args(&self, position: Vec3, rotation: Option<Vec3>, scale: Option<Vec3>) -> Args {
        let mut a = Args::new();
        a.insert("kind".into(), json!(self.kind.key()));
        match (self.kind, &self.reference) {
            (Kind::Spawn, Some(r)) => {
                a.insert("section".into(), json!(r));
            }
            // a shape's reference is its form or its use: sphere, box, or a box of a use of its own
            (Kind::Shape, Some(r)) => {
                let r = r.to_ascii_lowercase();
                a.insert("box".into(), json!(r != "sphere"));
                if SHAPE_USES.contains(&r.as_str()) {
                    a.insert("usage".into(), json!(r));
                }
            }
            (_, Some(r)) => {
                a.insert("ref".into(), json!(r));
            }
            _ => {}
        }
        if self.kind == Kind::Way {
            a.insert("points".into(), json!([[position.x, position.y, position.z]]));
        } else {
            a.insert("position".into(), json!([position.x, position.y, position.z]));
        }
        if let Some(r) = rotation.filter(|_| self.kind.class().caps().rotates) {
            a.insert("rotation".into(), json!([r.x, r.y, r.z]));
        }
        if let Some(f) = scale.filter(|_| self.kind.class().caps().scales) {
            a.insert("scale".into(), json!([f.x, f.y, f.z]));
        }
        a
    }
}

/// A Content Browser entry as a placeable thing: by its kind, else by its extension.
fn of_asset(kind: &str, path: &str) -> Option<Placeable> {
    let p = path.replace('/', "\\");
    let lower = p.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let stem = |root: &str| -> String {
        // the reference is the path under its library's root, without the extension
        let tail = match lower.find(&format!("{root}\\")) {
            Some(i) => &p[i + root.len() + 1..],
            None => p.as_str(),
        };
        tail.rsplit_once('.').map_or(tail, |(s, _)| s).to_owned()
    };
    match (kind, ext) {
        ("spawn_section" | "spawn", _) => Some(Placeable::new(Kind::Spawn, Some(&p))),
        (_, "object") => Some(Placeable::new(Kind::Object, Some(&stem("objects")))),
        // a model asset is referred to by its project path, extension and all
        (_, "meshasset") => Some(Placeable::new(Kind::Object, Some(&p))),
        (_, "group") => Some(Placeable::new(Kind::Group, Some(&p))),
        ("particles", _) | (_, "pe" | "pg") => {
            Some(Placeable::new(Kind::Particles, Some(&stem("particles"))))
        }
        ("sound", _) | (_, "ogg" | "wav") => Some(Placeable::new(Kind::Sound, Some(&stem("sounds")))),
        _ => None,
    }
}

/// Arms Place mode: the scene's mode becomes Place with the thing's kind, and each click places
/// it until Esc. `None` takes it away and goes back to Select.
pub fn arm(editor: &Arc<Editor>, what: Option<Placeable>) {
    let vp = Viewport::of(editor);
    vp.with(|s| s.armed.clone_from(&what));
    let mut a = Args::new();
    match &what {
        Some(p) => {
            a.insert("mode".into(), json!("place"));
            a.insert("kind".into(), json!(p.kind.key()));
        }
        None => {
            a.insert("mode".into(), json!("select"));
        }
    }
    editor.post("scene.mode", a);
}

/// What Place mode places: the armed thing, or a bare thing of the kind the scene's Place mode
/// names when nothing of that kind was armed.
#[must_use]
pub fn armed(editor: &Editor) -> Option<Placeable> {
    let kind = editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .map(|svc| svc.store.snapshot().tools.placing)
        .and_then(Kind::of_class)?;
    let held = Viewport::of(editor).read(|s| s.armed.clone());
    Some(held.filter(|p| p.kind == kind).unwrap_or(Placeable { kind, reference: None }))
}

/// Where a thing put down at the pointer lands: on the surface under it (Surface Snap), else on
/// the grid's plane; the grid step snaps the spot's two flat axes. The surface's up comes with it
/// while Align to Surface is on.
pub(crate) async fn landing(
    editor: &Arc<Editor>,
    svc: &Arc<SceneService>,
    ray: Ray,
) -> Option<(Vec3, Option<Vec3>)> {
    let (surface, align, grid, far) = Viewport::of(editor).read(|s| {
        (s.prefs.snaps.surface, s.prefs.snaps.align, s.snap(crate::gizmo::GizmoType::Move), s.nav.far)
    });
    let (at, normal) = crate::pane::pick_ground(editor, svc, ray, surface, far).await?;
    Some((on_grid(at, grid), align.then_some(normal)))
}

/// A spot on the grid's step along its two flat axes; its height stays the surface's.
#[must_use]
pub fn on_grid(at: Vec3, step: Option<f32>) -> Vec3 {
    match step {
        Some(step) => Vec3::new(crate::gizmo::snapto(at.x, step), at.y, crate::gizmo::snapto(at.z, step)),
        None => at,
    }
}

/// Whether the viewport takes a payload: it carries something it can place.
#[must_use]
pub fn takes(p: &Payload) -> bool {
    !Placeable::of_payload(p).is_empty()
}

/// The ghost of what a payload carries, standing where the ground was met (on its slope with
/// Align to Surface).
#[must_use]
pub fn ghost_of(p: &Payload, hit: (Vec3, Vec3), align: bool) -> Option<crate::service::Ghost> {
    let what = Placeable::of_payload(p).into_iter().next()?;
    Some(crate::service::Ghost { what, position: hit.0, up: align.then_some(hit.1) })
}

/// What the scene's random placement of objects gives the next one, while it is on: an object of
/// its list (when it names any), a turn (radians) and a scale out of its ranges.
struct Randomized {
    reference: Option<String>,
    rotation: Option<Vec3>,
    scale: Option<Vec3>,
}

fn random_placement(snap: &gz_scene::SceneState) -> Option<Randomized> {
    use gz_scene::tools::{
        FL_APPEND_RANDOM, FL_APPEND_RANDOM_PROPORTIONAL, FL_APPEND_RANDOM_ROTATION, FL_APPEND_RANDOM_SCALE,
        ToolOpts,
    };
    let part = snap.tools.part(ObjClass::SceneObject);
    if part.flags & FL_APPEND_RANDOM == 0 {
        return None;
    }
    let ToolOpts::Objects(o) = &part.opts else { return None };
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.subsec_nanos() as i32);
    let mut rng = gz_particles::Rng::new(seed);
    let reference = (!o.objects.is_empty()).then(|| {
        let i = rng.rand_i_max(i32::try_from(o.objects.len()).unwrap_or(i32::MAX)) as usize;
        o.objects.get(i).map(ToString::to_string).unwrap_or_default()
    });
    let rotation = (part.flags & FL_APPEND_RANDOM_ROTATION != 0).then(|| {
        Vec3::new(
            rng.rand_f_range(o.min_rotation.x, o.max_rotation.x),
            rng.rand_f_range(o.min_rotation.y, o.max_rotation.y),
            rng.rand_f_range(o.min_rotation.z, o.max_rotation.z),
        )
    });
    let scale = (part.flags & FL_APPEND_RANDOM_SCALE != 0).then(|| {
        if part.flags & FL_APPEND_RANDOM_PROPORTIONAL != 0 {
            Vec3::splat(rng.rand_f_range(o.min_scale.x, o.max_scale.x))
        } else {
            Vec3::new(
                rng.rand_f_range(o.min_scale.x, o.max_scale.x),
                rng.rand_f_range(o.min_scale.y, o.max_scale.y),
                rng.rand_f_range(o.min_scale.z, o.max_scale.z),
            )
        }
    });
    Some(Randomized { reference, rotation, scale })
}

/// Places `what` where `ray` lands, as one undo step, and selects it. Answers the name it got.
pub(crate) async fn place_at(editor: &Arc<Editor>, what: &Placeable, ray: Ray) -> Option<String> {
    let svc = SceneService::of(editor).ok()?;
    let random_list = what.kind == Kind::Object
        && random_placement(&svc.store.snapshot()).is_some_and(|r| r.reference.is_some());
    if what.kind.needs_reference() && what.reference.is_none() && !random_list {
        editor.log.info(format!(
            "Pick the {} to place in the Place panel first.",
            what.kind.class().label().to_lowercase()
        ));
        return None;
    }
    let (at, up) = landing(editor, &svc, ray).await?;
    let mut rotation = up.map(|n| gz_scene::math::normal_align(n, Vec3::Z));
    let mut scale = None;
    let mut what = what.clone();
    if what.kind == Kind::Object
        && let Some(r) = random_placement(&svc.store.snapshot())
    {
        if let Some(pick) = r.reference {
            what.reference = Some(pick);
        }
        rotation = r.rotation.or(rotation);
        scale = r.scale;
    }
    let degrees = rotation.map(|r| Vec3::new(r.x.to_degrees(), r.y.to_degrees(), r.z.to_degrees()));
    let origin = editor.ui_origin();
    let reply = editor.call("scene.place", what.args(at, degrees, scale), origin).await;
    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        let why = reply.get("error").and_then(Value::as_str).unwrap_or("It could not be placed.");
        editor.log.error(why.to_owned());
        return None;
    }
    // a library object answers its object, a `.group` asset the objects it placed
    let names: Vec<String> = match reply.get("objects").and_then(Value::as_array) {
        Some(list) => {
            list.iter().filter_map(|o| o.get("name").and_then(Value::as_str)).map(str::to_owned).collect()
        }
        None => reply
            .get("object")
            .and_then(|o| o.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .into_iter()
            .collect(),
    };
    let mut sel = Args::new();
    sel.insert("objects".into(), json!(names));
    let _ = editor.call("scene.select", sel, origin).await;
    editor.exec.wake().ping();
    names.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
    use gz_ui::drop::Asset;

    /// A frame of a bare context with `events`, the viewport's drop zone over its middle; what
    /// the zone took.
    fn frame(ctx: &Context, t: f64, events: Vec<Event>) -> Option<Payload> {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut got = None;
        let input = RawInput { screen_rect: Some(screen), time: Some(t), events, ..RawInput::default() };
        let mut out = ctx.run_ui(input, |ui| {
            gz_ui::drop::begin(ui.ctx());
            let rect = Rect::from_min_max(pos2(100.0, 100.0), pos2(700.0, 500.0));
            got = gz_ui::drop::Zone::new("Viewport", rect, &takes).take(ui);
        });
        out.textures_delta.clear();
        got
    }

    #[test]
    fn the_viewport_takes_a_place_row_let_go_over_it_and_nothing_else() {
        let ctx = Context::default();
        let row = Placeable::new(Kind::Object, Some("props\\box")).payload();
        let at = pos2(400.0, 300.0);
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        // the row is dragged in from elsewhere: the pointer over the viewport, the payload set
        assert!(frame(&ctx, 0.0, vec![Event::PointerMoved(at), press(true)]).is_none());
        egui::DragAndDrop::set_payload(&ctx, row.clone());
        assert!(frame(&ctx, 0.1, vec![Event::PointerMoved(at + vec2(4.0, 0.0))]).is_none());
        let taken = frame(&ctx, 0.2, vec![press(false)]);
        assert_eq!(taken.as_ref(), Some(&row));
        let what = taken.as_ref().map(Placeable::of_payload).and_then(|l| l.first().cloned());
        assert_eq!(what, Placeable::parse("object:props\\box"));
        // a payload of another surface is not the viewport's
        let other = Payload::Row { surface: "layers".into(), key: "1".into() };
        assert!(frame(&ctx, 0.3, vec![press(true)]).is_none());
        egui::DragAndDrop::set_payload(&ctx, other);
        assert!(frame(&ctx, 0.4, vec![press(false)]).is_none());
    }

    #[test]
    fn the_ghost_stands_where_the_ground_was_met_on_the_grid() {
        let row = Placeable::new(Kind::Light, None).payload();
        let hit = (Vec3::new(1.26, 3.5, -0.74), Vec3::new(0.0, 0.8, 0.6));
        let g = ghost_of(&row, hit, false);
        assert_eq!(g.map(|g| (g.what.kind, g.position, g.up)), Some((Kind::Light, hit.0, None)));
        assert_eq!(ghost_of(&row, hit, true).and_then(|g| g.up), Some(hit.1));
        // the grid's step snaps the two flat axes, the surface keeps the height
        assert_eq!(on_grid(hit.0, Some(0.5)), Vec3::new(1.5, 3.5, -0.5));
        assert_eq!(on_grid(hit.0, None), hit.0);
        assert!(ghost_of(&Payload::Text("x".into()), hit, false).is_none());
    }

    #[test]
    fn a_row_key_round_trips_and_names_its_kind() {
        let p = Placeable::new(Kind::Object, Some("props/box_wood_01"));
        assert_eq!(p.key(), "object:props\\box_wood_01");
        assert_eq!(Placeable::parse(&p.key()), Some(p.clone()));
        assert_eq!(Placeable::of_payload(&p.payload()), vec![p]);
        assert_eq!(Placeable::parse("light"), Some(Placeable { kind: Kind::Light, reference: None }));
        assert!(Placeable::parse("tree:oak").is_none());
        let other = Payload::Row { surface: "layers".into(), key: "light".into() };
        assert!(Placeable::of_payload(&other).is_empty());
    }

    #[test]
    fn the_browsers_entries_become_what_they_place() {
        let asset = |path: &str, kind: &str| Asset { path: path.into(), kind: kind.into(), file: None };
        let list = Payload::Assets(vec![
            asset("rawdata/objects/props/box_wood_01.object", "object"),
            asset("stalker_bandit_1", "spawn_section"),
            asset("rawdata/sounds/ambient/wind.ogg", "sound"),
            asset("explosions/explosion_01.pe", "particles"),
            asset("levels/escape.scene", "scene"),
        ]);
        let got: Vec<String> = Placeable::of_payload(&list).iter().map(Placeable::key).collect();
        assert_eq!(
            got,
            [
                "object:props\\box_wood_01",
                "spawn:stalker_bandit_1",
                "sound:ambient\\wind",
                "particles:explosions\\explosion_01"
            ]
        );
    }

    #[test]
    fn the_arguments_of_scene_place_say_what_and_where() {
        let a = Placeable::new(Kind::Spawn, Some("stalker")).args(Vec3::new(1.0, 2.0, 3.0), None, None);
        assert_eq!(a.get("section"), Some(&json!("stalker")));
        assert_eq!(a.get("position"), Some(&json!([1.0, 2.0, 3.0])));
        // a sound turns no way: no rotation is sent for it
        let s =
            Placeable::new(Kind::Sound, Some("wind")).args(Vec3::ZERO, Some(Vec3::new(0.0, 90.0, 0.0)), None);
        assert!(s.get("rotation").is_none());
        let b = Placeable::parse("shape:ai_map_bounds")
            .map(|p| p.args(Vec3::ZERO, None, None))
            .unwrap_or_default();
        assert_eq!((b.get("box"), b.get("usage")), (Some(&json!(true)), Some(&json!("ai_map_bounds"))));
        let sphere = Placeable::new(Kind::Shape, Some("sphere")).args(Vec3::ZERO, None, None);
        assert_eq!((sphere.get("box"), sphere.get("usage")), (Some(&json!(false)), None));
        let w = Placeable::new(Kind::Way, None).args(Vec3::ONE, None, None);
        assert_eq!(w.get("points"), Some(&json!([[1.0, 1.0, 1.0]])));
    }
}
