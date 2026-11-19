//! The level of the game a scene lies over.
//!
//! A scene that says `base = "<level>"` holds what a module adds to a level of the game; the
//! level itself - its geometry, its spawn points, its paths, lights, sounds and effects, its AI
//! map - is the game's and comes from the game library as the scene is read
//! ([`crate::format::read_based`]). Whoever holds the game's levels as scenes registers a
//! [`BaseSource`] with [`BaseLevels`]; the scene's crate knows no more of it.

use std::sync::Arc;

use gz_base::SmolStr;
use gz_editor::Editor;
use gz_entities::Settings;
use gz_store::imbl::OrdSet;
use parking_lot::RwLock;

use crate::model::{Backdrop, SceneMeta, SceneState};

/// The levels of the game as scenes.
pub trait BaseSource: Send + Sync {
    /// A level as a scene with every object marked [`crate::model::ModMarks::base`]; a sentence
    /// when there is none. Blocking: a pool thread asks.
    fn level(&self, name: &str, settings: Option<&(dyn Settings + Sync)>) -> Result<Arc<SceneState>, String>;

    /// The names of the levels there are.
    fn names(&self) -> Vec<SmolStr>;
}

/// The level an open scene lies over answers for itself: a read over the open scene needs no
/// library.
impl BaseSource for Backdrop {
    fn level(&self, name: &str, _: Option<&(dyn Settings + Sync)>) -> Result<Arc<SceneState>, String> {
        match &self.state {
            Some(state) if self.level.eq_ignore_ascii_case(name) => Ok(state.clone()),
            _ => Err(format!("the open scene does not lie over '{name}'")),
        }
    }

    fn names(&self) -> Vec<SmolStr> {
        if self.is_set() { vec![self.level.clone()] } else { Vec::new() }
    }
}

/// The editor's source of the levels a scene may lie over.
#[derive(Default)]
pub struct BaseLevels {
    source: RwLock<Option<Arc<dyn BaseSource>>>,
}

impl BaseLevels {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Registers the source: the game library does, once, as the editor starts.
    pub fn set(&self, source: Arc<dyn BaseSource>) {
        *self.source.write() = Some(source);
    }

    #[must_use]
    pub fn get(&self) -> Option<Arc<dyn BaseSource>> {
        self.source.read().clone()
    }
}

/// A level as a base: every object marked the level's own (a library's level is made once and
/// handed to every scene that lies over it).
#[must_use]
pub fn marked(mut level: SceneState) -> SceneState {
    let ids: Vec<_> = level.objects.keys().copied().collect();
    for id in ids {
        if let Some(o) = level.objects.get(&id)
            && !o.marks.base
        {
            let mut o = (**o).clone();
            o.marks.base = true;
            o.marks.authored = false;
            o.marks.removed = false;
            level.objects.insert(id, Arc::new(o));
        }
    }
    level
}

/// The scene a read starts from over a level: the level's objects, options, tools and painted
/// layers, with nothing selected and no file.
#[must_use]
pub fn over(level: &Arc<SceneState>, name: &str) -> SceneState {
    let mut state = (**level).clone();
    state.meta = SceneMeta {
        level: level.meta.level.clone(),
        base: Backdrop { level: SmolStr::new(name), state: Some(level.clone()), pending: Vec::new() },
        ..SceneMeta::default()
    };
    state.selection = OrdSet::new();
    state.serial = 0;
    state.valid = true;
    state
}

#[cfg(test)]
mod tests {
    use gz_base::RelPath;

    use super::*;
    use crate::class::ObjClass;
    use crate::format::{self, Plan, Texts};
    use crate::math::Vec3;

    struct Levels(Option<Arc<SceneState>>);

    impl BaseSource for Levels {
        fn level(&self, name: &str, _: Option<&(dyn Settings + Sync)>) -> Result<Arc<SceneState>, String> {
            self.0.clone().filter(|_| name == "lvl").ok_or_else(|| "the game has no such level".to_owned())
        }

        fn names(&self) -> Vec<SmolStr> {
            vec![SmolStr::new("lvl")]
        }
    }

    fn named(class: ObjClass, name: &str) -> crate::model::Object {
        let mut o = format::default_object(class);
        o.name = SmolStr::new(name);
        o
    }

    /// A level of the game: its geometry and two zones.
    fn level() -> Arc<SceneState> {
        let mut s = SceneState::default();
        s.meta.level.level_path = SmolStr::new("lvl");
        s.insert(named(ObjClass::SceneObject, "level_geometry"));
        s.insert(named(ObjClass::SpawnPoint, "a_zone"));
        s.insert(named(ObjClass::SpawnPoint, "b_zone"));
        Arc::new(marked(s))
    }

    fn texts() -> Texts {
        Texts {
            scene: "return {\n  scene = 2,\n  base = \"lvl\",\n}\n".into(),
            files: vec![
                (
                    "spawns.lua".into(),
                    "return {\n  spawns = {\n    -- the field goes\n    { name = \"a_zone\", removed = true },\n    { name = \"nope\", removed = true },\n  },\n}\n".into(),
                ),
                (
                    "objects.lua".into(),
                    "return {\n  objects = {\n    { name = \"crate\", pos = { 1, 2, 3 }, ref = \"props\\\\box\" },\n  },\n}\n".into(),
                ),
            ],
            layers: Vec::new(),
        }
    }

    fn read(t: &Texts, levels: &Levels) -> format::Read {
        let main = RelPath::new("levels/base.scene").unwrap();
        let dir = RelPath::new("levels/base").unwrap();
        format::read_based(t, &main, &dir, None, None, Some(levels)).unwrap()
    }

    fn file<'a>(plan: &'a Plan, name: &str) -> Option<&'a str> {
        plan.files
            .iter()
            .find(|f| f.file == name)
            .and_then(|f| f.bytes.as_deref())
            .map(|b| std::str::from_utf8(b).unwrap())
    }

    fn take_out(s: &mut SceneState, name: &str, on: bool) {
        let mut o = (**s.find(ObjClass::SpawnPoint, name).unwrap()).clone();
        o.marks.removed = on;
        s.replace(o);
    }

    #[test]
    fn a_scene_over_a_level_holds_the_level_and_writes_only_its_own() {
        let levels = Levels(Some(level()));
        let r = read(&texts(), &levels);
        let s = &r.state;
        assert_eq!(s.meta.base.level, "lvl");
        assert_eq!(s.meta.level.level_path, "lvl", "a key the .scene leaves out is the level's");
        assert!(s.find_any("level_geometry").is_some_and(|o| o.marks.base && !o.marks.authored));
        assert!(s.find_any("a_zone").is_some_and(|o| o.marks.base && o.marks.removed));
        assert!(s.find_any("b_zone").is_some_and(|o| o.marks.base && !o.marks.removed));
        let own = s.find_any("crate").unwrap();
        assert!(own.marks.authored && !own.marks.base, "what the files hold is the module's");
        assert_eq!(own.xform.position, Vec3::new(1.0, 2.0, 3.0));
        assert!(s.find_any("nope").is_none(), "an entry that takes out is no object");
        let codes: Vec<&str> = r.problems.iter().map(|p| p.code.as_str()).collect();
        assert_eq!(codes, ["S061"], "{:?}", r.problems);

        // nothing changed: nothing is written
        let plan = format::write(s, Some(&r.files), None).unwrap();
        assert!(plan.files.is_empty(), "{:?}", plan.files);

        // one more of the level's spawn points goes: one line of one file
        let mut s2 = s.clone();
        take_out(&mut s2, "b_zone", true);
        let plan = format::write(&s2, Some(&r.files), None).unwrap();
        assert_eq!(plan.files.len(), 1);
        let spawns = file(&plan, "spawns.lua").unwrap();
        assert!(spawns.contains("{ name = \"b_zone\", removed = true }"), "{spawns}");
        assert!(spawns.contains("-- the field goes") && spawns.contains("\"nope\""), "{spawns}");

        // both back in the game: the file keeps the entry the scene holds no object for
        let mut s3 = s2.clone();
        take_out(&mut s3, "a_zone", false);
        take_out(&mut s3, "b_zone", false);
        let plan = format::write(&s3, Some(&plan.after), None).unwrap();
        let spawns = file(&plan, "spawns.lua").expect("the file stays: it holds an entry");
        assert!(
            !spawns.contains("a_zone") && !spawns.contains("b_zone") && spawns.contains("\"nope\""),
            "{spawns}"
        );

        // saved as another scene: the level is not copied into it
        let whole = format::write(&s2, None, None).unwrap();
        let scene = file(&whole, "").unwrap();
        assert!(scene.contains("base = \"lvl\"") && !scene.contains("path"), "{scene}");
        let objects = file(&whole, "objects.lua").unwrap();
        assert!(objects.contains("\"crate\"") && !objects.contains("level_geometry"), "{objects}");
        assert!(!objects.contains("authored"), "the mark is implied: {objects}");
        let spawns = file(&whole, "spawns.lua").unwrap();
        assert!(
            spawns.contains("a_zone") && spawns.contains("b_zone") && !spawns.contains("section"),
            "{spawns}"
        );
        // and the entry the scene holds no object for goes with it: it is a line of the files
        assert_eq!(s.meta.base.pending, ["nope"]);
        assert!(spawns.contains("{ name = \"nope\", removed = true }"), "{spawns}");
    }

    #[test]
    fn without_the_level_a_scene_keeps_what_its_files_say() {
        let r = read(&texts(), &Levels(None));
        assert_eq!(r.state.meta.base.level, "lvl");
        assert!(r.state.meta.base.state.is_none());
        assert!(r.problems.iter().any(|p| p.code == "S060" && p.line == Some(3)), "{:?}", r.problems);
        assert_eq!(r.state.objects.len(), 1, "the scene's own object alone");
        // a save leaves the entries nobody could read
        let mut s = r.state.clone();
        let mut o = (**s.find_any("crate").unwrap()).clone();
        o.xform.position.y = 5.0;
        s.replace(o);
        let plan = format::write(&s, Some(&r.files), None).unwrap();
        assert_eq!(plan.files.len(), 1, "{:?}", plan.files);
        assert!(file(&plan, "objects.lua").is_some());
        // saved as another scene, or as a recovery snapshot: the lines that take spawn points
        // out go along, though nobody could read them into objects
        let whole = format::write(&s, None, None).unwrap();
        let spawns = file(&whole, "spawns.lua").expect("the lines are written into the new file");
        assert!(
            spawns.contains("{ name = \"a_zone\", removed = true }")
                && spawns.contains("{ name = \"nope\", removed = true }"),
            "{spawns}"
        );
        // read with the level at hand again they take the spawn point out
        let back = Texts {
            scene: file(&whole, "").unwrap().to_owned(),
            files: vec![
                ("spawns.lua".into(), spawns.to_owned()),
                ("objects.lua".into(), file(&whole, "objects.lua").unwrap().to_owned()),
            ],
            layers: Vec::new(),
        };
        let again = read(&back, &Levels(Some(level())));
        assert!(again.state.find_any("a_zone").is_some_and(|o| o.marks.base && o.marks.removed));
    }

    #[test]
    fn the_level_s_own_is_not_changed_but_a_spawn_point_is_taken_out() {
        use gz_store::{Store, StoreOptions};
        use gz_task::{ExecOptions, Runtime};

        use crate::apply::SceneCmd;
        let r = read(&texts(), &Levels(Some(level())));
        let geometry = r.state.find_any("level_geometry").unwrap().id;
        let zone = r.state.find_any("b_zone").unwrap().id;
        let own = r.state.find_any("crate").unwrap().id;
        let rt = Runtime::start(ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 }).unwrap();
        let store = Store::spawn("scene", r.state, &rt.exec(), StoreOptions::default());
        let run = |cmd: SceneCmd| rt.block_on(store.send(cmd, gz_base::Origin::Mcp));
        // the level's geometry keeps its name, and it is not deleted
        let e = run(SceneCmd::Rename { id: geometry, name: "mine".into() }).unwrap_err();
        assert!(e.message.contains("the level's own"), "{}", e.message);
        assert!(run(SceneCmd::Delete(vec![geometry, own])).is_err());
        assert!(store.snapshot().get(own).is_some(), "nothing goes when one of them is refused");
        // a spawn point of the level is taken out instead of deleted
        run(SceneCmd::Delete(vec![zone])).unwrap();
        assert!(store.snapshot().get(zone).is_some_and(|o| o.marks.removed));
        // what the scene holds over a level is the module's without being told
        assert!(store.snapshot().get(own).is_some_and(|o| o.marks.authored));

        // a copy of one of the level's objects is the scene's own, whole, and is written
        let bundle = crate::edit::bundle_of(&store.snapshot(), &[geometry]);
        run(SceneCmd::Paste {
            bundle: Box::new(bundle),
            offset: Vec3::new(1.0, 0.0, 0.0),
            label: "Duplicate".into(),
        })
        .unwrap();
        let s = store.snapshot();
        let copy = s
            .of_class(ObjClass::SceneObject)
            .find(|o| o.id != geometry && o.name.starts_with("level_geometry"))
            .expect("the copy is in the scene");
        assert!(!copy.marks.base && copy.marks.authored && SceneState::written(copy));
        let text = format::group::group_text(&s, &[geometry], Vec3::ZERO, None).unwrap();
        assert!(
            !text.contains("removed"),
            "a group asset holds the object, not a line that takes it out: {text}"
        );

        // the level's own stay out of a group, and a reading of another scene's files stays out
        run(SceneCmd::Select { ids: vec![geometry], mode: crate::apply::SelectMode::Set }).unwrap();
        let e = run(SceneCmd::Group { name: "all".into() }).unwrap_err();
        assert!(e.message.contains("the level's own"), "{}", e.message);
        let e = run(SceneCmd::Reload {
            state: Box::new(SceneState::default()),
            label: "Reload from Disk".into(),
            clean: true,
            of: std::path::PathBuf::from("another.scene"),
        })
        .unwrap_err();
        assert_eq!(e.code, "another_scene");
        assert!(store.snapshot().get(geometry).is_some(), "the scene is as it was");
    }
}
