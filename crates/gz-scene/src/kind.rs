//! The scene and the object group as kinds of `gz-asset`: what `asset.check`, `asset.reference`
//! and `asset.kinds` ask of them.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_asset::{AssetFile, KindOps};
use gz_base::Problem;
use gz_editor::Editor;
use gz_luadata::Limits;

use crate::format::{self, group};

/// The keys of a `.scene` itself (its objects are in the files of its folder).
const SCENE_KEYS: &[&str] = &[
    format::HEADER,
    "base",
    "level",
    "build",
    "sun",
    "light_controls",
    "camera",
    "classes",
    "view_flags",
    "random_placement",
    "layers",
];

/// The keys of a `.group`: its header and the lists of objects.
const GROUP_KEYS: &[&str] = &[
    group::HEADER,
    "objects",
    "lights",
    "glows",
    "sources",
    "environments",
    "particles",
    "shapes",
    "volumes",
    "spawns",
    "ways",
    "sectors",
    "portals",
    "groups",
    "terrains",
];

/// The scene kind.
#[must_use]
pub fn scene() -> KindOps {
    KindOps {
        name: "scene",
        ext: ".scene",
        header: format::HEADER,
        version: format::VERSION,
        limits: Limits::scene(),
        migrations: format::MIGRATIONS,
        known: &[("", SCENE_KEYS)],
        reference_doc: "docs/assets/SCENE.md",
        check: check_scene,
        reference: |_| Box::pin(async { format::doc::reference() }),
        diff_rules: format::scene_rules,
        take_theirs: None,
    }
}

/// The object group kind.
#[must_use]
pub fn group() -> KindOps {
    KindOps {
        name: "group",
        ext: ".group",
        header: group::HEADER,
        version: group::VERSION,
        limits: Limits::scene(),
        migrations: group::MIGRATIONS,
        known: &[("", GROUP_KEYS)],
        reference_doc: "docs/assets/SCENE.md",
        check: check_group,
        reference: |_| Box::pin(async { format::doc::reference() }),
        diff_rules: gz_luadata::DiffRules::new,
        take_theirs: None,
    }
}

/// The problems of a scene: what reading it with its folder's files finds.
fn check_scene(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let settings = crate::game::settings(&ed).await;
        let abs = file.abs.clone();
        let main = file.path.clone();
        let lib = gz_project::library_root(&ed);
        let levels = crate::base::BaseLevels::of(&ed).get();
        let names = crate::names::NameSources::names(&ed).await;
        let module = match ed.project_opt() {
            Some(s) => gz_mod::ManifestService::of_session(&ed, &s)
                .await
                .ok()
                .and_then(|m| m.snapshot().manifest().map(|x| x.id.clone())),
            None => None,
        }
        .unwrap_or_default();
        ed.exec
            .io(move || {
                let texts = match crate::service::read_texts(&abs) {
                    Ok(t) => t,
                    Err(e) => {
                        return vec![
                            Problem::error("io", format!("the scene could not be read: {e}")).in_file(main),
                        ];
                    }
                };
                // the folder beside the file, named as the scene
                let stem = main.as_str().rsplit_once('.').map_or(main.as_str(), |(s, _)| s);
                let folder = gz_base::RelPath::new(stem).unwrap_or_else(|_| main.clone());
                let st = settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
                match format::read_based(&texts, &main, &folder, st, None, levels.as_deref()) {
                    Ok(mut read) => {
                        // the groups placed at the read: a group asset that cannot be read is an error
                        let depth = main.as_str().split('/').count();
                        if let Some(root) = abs.ancestors().nth(depth) {
                            let (notes, _) = crate::edit::place_unplaced(
                                &mut read.state,
                                &read.unplaced,
                                root,
                                Some(&lib),
                                st,
                            );
                            read.problems.extend(notes);
                        }
                        let mut more = Vec::new();
                        if let Some(names) = &names {
                            picked_names(&read, &folder, &**names, &mut more);
                            undrawn_models(&read, &folder, &**names, st, &mut more);
                        }
                        directional_lights(&read, &folder, &mut more);
                        stash_names(&read, &folder, &mut more);
                        let game_levels = levels.as_ref().map(|l| l.names()).unwrap_or_default();
                        story_ids(&read, &folder, &main, &module, &game_levels, &mut more);
                        own_level_names(&read, &main, &module, &game_levels, &mut more);
                        glow_shaders(&read, &folder, &mut more);
                        zones_over_a_level(&read, &folder, &mut more);
                        details_over_a_level(&read, &main, &mut more);
                        read.problems.extend(more);
                        read.problems
                    }
                    Err(r) => vec![*r.problem],
                }
            })
            .await
    })
}

/// The problems of an object group.
fn check_group(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let settings = crate::game::settings(&ed).await;
        let text = file.doc.text().to_owned();
        let rel = file.path.clone();
        ed.exec
            .cpu(move || {
                match group::read_group(
                    &text,
                    &rel,
                    settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
                ) {
                    Ok((_, problems)) => problems,
                    Err(r) => vec![*r.problem],
                }
            })
            .await
    })
}

/// Where a key of an object of the scene's own files stands: the file and the span.
fn key_place(
    read: &format::Read,
    folder: &gz_base::RelPath,
    o: &crate::model::Object,
    key: &str,
) -> Option<(gz_base::RelPath, gz_luadata::Span)> {
    use gz_luadata::Seg;
    let (file, list) = format::file_of(o.class)?;
    let doc = read.files.docs.iter().find(|d| d.file == file)?;
    let span = doc.doc.nearest_span(&[Seg::key(list), Seg::item("name", o.name.as_str()), Seg::key(key)]);
    Some((folder.join(file).ok()?, span))
}

/// `S220`: a key that picks something of the game by its name - a sound, a particle effect, a
/// light animation, a sound environment, a texture, a compiled model - names what neither the
/// game nor the project has. The game plays, draws or shows nothing there.
fn picked_names(
    read: &format::Read,
    folder: &gz_base::RelPath,
    names: &dyn crate::names::Names,
    out: &mut Vec<Problem>,
) {
    use crate::format::fields::{self, Ty};
    for o in read.state.objects.values().filter(|o| !o.marks.base) {
        for f in fields::object_fields(o.class) {
            let Ty::Pick(list) = f.ty else { continue };
            if !(f.applies)(o) {
                continue;
            }
            let gz_luadata::Value::String(value) = (f.get)(o) else { continue };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let Some(kind) = crate::names::kind_of_pick(list, value) else { continue };
            let word = crate::names::kind_word(kind);
            // there, and not what the game takes for one
            if let Some(why) = names.refusal(kind, value) {
                let p = Problem::warning(
                    "S220",
                    format!("{} names the {word} '{value}', which the game would not take: {why}", o.name),
                );
                out.push(match key_place(read, folder, o, f.key) {
                    Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
                    None => p,
                });
                continue;
            }
            if names.has(kind, value) != Some(false) {
                continue;
            }
            let near = names.nearest(kind, value);
            let hint = if kind == "fog_profile" {
                "a fog profile is a file under the game's configs, said from there (the game's own is environment/fog/default.ltx)"
                    .to_owned()
            } else if near.is_empty() {
                format!("`asset.find kind={kind}` lists what there is")
            } else {
                format!("nearest: {}", near.join(", "))
            };
            let p = Problem::warning(
                "S220",
                format!(
                    "{} names the {word} '{value}', which neither the game nor the project has: {hint}",
                    o.name
                ),
            );
            out.push(match key_place(read, folder, o, f.key) {
                Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
                None => p,
            });
        }
    }
}

/// `S226`: a stash whose name the spawn layer cannot write as it is, or writes another object
/// under. A quest reaches the stash by its name: the build refuses it.
fn stash_names(read: &format::Read, folder: &gz_base::RelPath, out: &mut Vec<Problem>) {
    for (id, why) in crate::objects::stash::clashes(&read.state) {
        let Some(o) = read.state.get(id) else { continue };
        let p = Problem::error("S226", why);
        out.push(match key_place(read, folder, o, "name") {
            Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
            None => p,
        });
    }
}

/// `S227`: a spawn point of the module whose story id is not named after the module. A story id
/// is one name in the whole game: of two objects that carry one the game keeps it for the first -
/// the game's own, then the module earlier in the load order - and takes it from the other, which
/// a quest or a script then does not find by it.
fn story_ids(
    read: &format::Read,
    folder: &gz_base::RelPath,
    main: &gz_base::RelPath,
    module: &str,
    game_levels: &[gz_base::SmolStr],
    out: &mut Vec<Problem>,
) {
    if module.is_empty() {
        return;
    }
    // a whole scene of a level of the game holds the game's own objects: the module's are what it
    // marks authored, as the spawn layer takes them
    let level = level_of(read, main);
    let of_the_game =
        !read.state.meta.base.is_set() && game_levels.iter().any(|l| l.eq_ignore_ascii_case(&level));
    let ours =
        |o: &&std::sync::Arc<crate::model::Object>| !o.marks.base && (!of_the_game || o.marks.authored);
    for o in read.state.of_class(crate::class::ObjClass::SpawnPoint).filter(ours) {
        let Some(sid) = crate::props::story_id_of(o) else { continue };
        if gz_mod::names::is_own(&sid, module) {
            continue;
        }
        let p = Problem::warning(
            "S227",
            format!(
                "{} carries the story id '{sid}', which is not named after the module: a story id is one name in the whole game, and of two objects that carry it the game keeps it for the first - the game's own, or a module's earlier in the load order - and takes it from the other. Name the module's own '{}', or carry the module id as a word of it",
                o.name,
                gz_mod::names::own_form(&sid, module)
            ),
        );
        out.push(match key_place(read, folder, o, "props") {
            Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
            None => p,
        });
    }
}

/// The level a scene is of: the level it lies over, else its name - its file's, since the check
/// reads the scene from its texts.
fn level_of(read: &format::Read, main: &gz_base::RelPath) -> String {
    let stem = main.file_name().rsplit_once('.').map_or(main.file_name(), |(s, _)| s).to_owned();
    Some(read.state.meta.level_name()).filter(|l| !l.is_empty()).unwrap_or(stem)
}

/// `S229`: the name of a level of the module's own is one name for the whole game - of two modules
/// that bring a level of it, the game joins one and leaves the other out. A level of the game is
/// the game's; the objects of every scene are composed under the module's own name
/// (`<module>.<name>`), its smart terrains and way paths too.
fn own_level_names(
    read: &format::Read,
    main: &gz_base::RelPath,
    module: &str,
    game_levels: &[gz_base::SmolStr],
    out: &mut Vec<Problem>,
) {
    // without the game's levels nothing tells a level of the module's own
    if module.is_empty() || read.state.meta.base.is_set() || game_levels.is_empty() {
        return;
    }
    let level = level_of(read, main);
    if level.is_empty()
        || game_levels.iter().any(|l| l.eq_ignore_ascii_case(&level))
        || gz_mod::names::is_own(&level, module)
    {
        return;
    }
    out.push(
        Problem::warning(
            "S229",
            format!(
                "the level '{level}' is the module's own and not named after the module: a level's name is one name for the whole game, and of two modules that bring a level of it the game joins one and leaves the other out - name the scene '{}', or carry the module id as a word of it",
                gz_mod::names::own_form(&level, module)
            ),
        )
        .in_file(main.clone())
        .at_line(1),
    );
}

/// `S225`: a glow that names a shader no glow is drawn with. The game draws a glow with the lens
/// flares' shader or the sun's and takes the flares' for any other name: the glow is drawn, and
/// not the way the name says.
fn glow_shaders(read: &format::Read, folder: &gz_base::RelPath, out: &mut Vec<Problem>) {
    use crate::objects::{GlowShader, glow_shader};
    for o in read.state.of_class(crate::class::ObjClass::Glow).filter(|o| !o.marks.base) {
        let crate::model::ObjData::Glow(g) = &o.data else { continue };
        if glow_shader(&g.shader) != GlowShader::Other {
            continue;
        }
        let p = Problem::warning(
            "S225",
            format!(
                "{} names the shader '{}', which no glow is drawn with: the game draws a glow like a lens flare (`effects\\flare`, also when `shader` is left out) or like the sun (`effects\\sun`: hidden by depth too), and takes the flares' for this one",
                o.name, g.shader
            ),
        );
        out.push(match key_place(read, folder, o, "shader") {
            Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
            None => p,
        });
    }
}

/// `S224`: an object whose model the game draws nothing for - it is there, it stops a body, a
/// box of it holds things, and nobody sees it. A section whose own model is such is made to be
/// unseen (a stash is an unseen case inside something that is drawn): there it is the section's
/// way and nothing is said. A model named a collider is unseen by its name.
fn undrawn_models(
    read: &format::Read,
    folder: &gz_base::RelPath,
    names: &dyn crate::names::Names,
    settings: Option<&(dyn gz_entities::Settings + Sync)>,
    out: &mut Vec<Problem>,
) {
    use crate::model::ObjData;
    let model_ref = |name: &str| {
        let name = name.trim().replace('/', "\\");
        if name.to_ascii_lowercase().ends_with(".ogf") { name } else { format!("{name}.ogf") }
    };
    for o in read.state.objects.values().filter(|o| !o.marks.base) {
        let (model, key) = match &o.data {
            ObjData::SceneObject(d) if d.reference.to_ascii_lowercase().ends_with(".ogf") => {
                (model_ref(&d.reference), "ref")
            }
            ObjData::Spawn(d) => {
                let Some(model) = crate::objects::visual_ref(o) else { continue };
                // the section's own model is unseen too: the section is made that way
                let section = d.entity.as_ref().map(|e| e.section().to_owned()).unwrap_or_default();
                let own = settings
                    .filter(|s| s.section_exist(&section))
                    .and_then(|s| s.r_string(&section, "visual"))
                    .map(model_ref);
                if own.is_some_and(|own| names.undrawn(&own) == Some(true)) {
                    continue;
                }
                (model, "props")
            }
            _ => continue,
        };
        if model.to_ascii_lowercase().contains("collider") || names.undrawn(&model) != Some(true) {
            continue;
        }
        let p = Problem::warning(
            "S224",
            format!(
                "{} wears the model '{model}', which the game draws nothing for: the object is there and nobody sees it (`asset.find kind=model` marks such a model `invisible`)",
                o.name
            ),
        );
        out.push(match key_place(read, folder, o, key) {
            Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
            None => p,
        });
    }
}

/// `S222`: a sector or a portal of a scene that lies over a level of the game. The level's
/// visibility cells are the game's own, compiled into its level, and a module adds none: what the
/// module places is hung on the level's sectors where it stands.
fn zones_over_a_level(read: &format::Read, folder: &gz_base::RelPath, out: &mut Vec<Problem>) {
    use crate::class::ObjClass;
    if !read.state.meta.base.is_set() {
        return;
    }
    for class in [ObjClass::Sector, ObjClass::Portal] {
        for o in read.state.of_class(class).filter(|o| !o.marks.base) {
            let what = if class == ObjClass::Sector { "sector" } else { "portal" };
            let p = Problem::warning(
                "S222",
                format!(
                    "{} is a {what} of a scene over a level of the game: the level's own sectors and portals are the game's, and a module adds none - it is not built (the module's objects hang on the level's sectors where they stand)",
                    o.name
                ),
            );
            out.push(match key_place(read, folder, o, "name") {
                Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
                None => p,
            });
        }
    }
    // the level's bound and the bounds of its AI map are the game's as well
    for o in read.state.of_class(ObjClass::Shape).filter(|o| !o.marks.base) {
        let crate::model::ObjData::Shape(d) = &o.data else { continue };
        let what = match d.usage {
            crate::objects::SHAPE_LEVEL_BOUND => "the bound of the level",
            u if u == crate::bulk::ground::SHAPE_AI_MAP_BOUNDS => "the bounds of the AI map",
            _ => continue,
        };
        let p = Problem::warning(
            "S222",
            format!(
                "{} says {what} in a scene over a level of the game: the level and its AI map are the game's, and the shape changes nothing",
                o.name
            ),
        );
        out.push(match key_place(read, folder, o, "usage") {
            Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
            None => p,
        });
    }
}

/// `S223`: a painted layer of a scene over a level of the game that the game takes from the
/// level alone. What grows on a level is the level's own file, read whole; its AI map is the
/// level's too, and a module changes that under a terrain that cuts the ground, never by
/// painting. Such a layer stays in the project and does not ship.
fn details_over_a_level(read: &format::Read, main: &gz_base::RelPath, out: &mut Vec<Problem>) {
    use format::layers::{LAYERS, own};
    if !read.state.meta.base.is_set() {
        return;
    }
    for layer in LAYERS {
        let why = match layer.class {
            crate::class::ObjClass::DetailObjects
                if read.state.bulk.details.as_ref().is_some_and(|d| !d.model.slots.is_empty()) =>
            {
                "the scene paints details (grass) over a level of the game: the game grows the level's own and reads none of a module's, so the layer does not ship"
            }
            crate::class::ObjClass::AiMap if own(layer, &read.state) => {
                "the scene holds an AI map of its own over a level of the game: the level's AI map is the game's, and a module changes it under a terrain that cuts the ground alone - the painted map does not ship"
            }
            _ => continue,
        };
        out.push(Problem::warning("S223", why).in_file(main.clone()));
    }
}

/// `S221`: a directional light asked to shine in the game. The game has one directional light,
/// the level's sun, which is the scene's own setting: such an object is baked into the lightmaps
/// when it is asked to, and shines in the game never.
fn directional_lights(read: &format::Read, folder: &gz_base::RelPath, out: &mut Vec<Problem>) {
    use crate::objects::{LT_DIRECT, light_flags};
    for o in read.state.of_class(crate::class::ObjClass::Light).filter(|o| !o.marks.base) {
        let crate::model::ObjData::Light(d) = &o.data else { continue };
        if d.kind != LT_DIRECT || d.flags & light_flags::AFFECT_DYNAMIC == 0 {
            continue;
        }
        let also = if d.flags & light_flags::AFFECT_STATIC != 0 {
            "it lights the lightmaps only"
        } else {
            "it lights nothing"
        };
        let p = Problem::warning(
            "S221",
            format!(
                "{} is a directional light asked to shine in the game: the game's one directional light is the level's sun (the scene's sun setting), so {also}; make it a point or a spot light, or light the lightmaps with it alone",
                o.name
            ),
        );
        out.push(match key_place(read, folder, o, "kind") {
            Some((file, s)) => p.in_file(file).at(s.line, s.column, s.end_line, s.end_column),
            None => p,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists that know every name and call a model with `_case` in it one nobody sees.
    struct Lists;

    impl crate::names::Names for Lists {
        fn has(&self, _: &str, _: &str) -> Option<bool> {
            Some(true)
        }

        fn nearest(&self, _: &str, _: &str) -> Vec<String> {
            Vec::new()
        }

        fn undrawn(&self, model: &str) -> Option<bool> {
            Some(model.contains("_case"))
        }
    }

    #[test]
    fn a_model_nobody_sees_is_said_at_its_object_and_a_collider_is_not() {
        let objects = [
            r#"{ name = "unseen", pos = { 1, 0, 0 }, ref = "dynamics/cache/box_case.ogf" },"#,
            r#"{ name = "seen", pos = { 2, 0, 0 }, ref = "dynamics/box/box_wood.ogf" },"#,
            r#"{ name = "wall", pos = { 3, 0, 0 }, ref = "dynamics/cache/collider_case.ogf" },"#,
        ];
        let texts = format::Texts {
            scene: "return {\n  scene = 2,\n}\n".into(),
            files: vec![(
                "objects.lua".into(),
                format!("return {{\n  objects = {{\n    {}\n  }},\n}}\n", objects.join("\n    ")),
            )],
            layers: Vec::new(),
        };
        let main = gz_base::RelPath::new("levels/yard.scene").unwrap();
        let folder = gz_base::RelPath::new("levels/yard").unwrap();
        let read = format::read_based(&texts, &main, &folder, None, None, None).unwrap();
        let mut out = Vec::new();
        undrawn_models(&read, &folder, &Lists, None, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        let p = &out[0];
        assert_eq!(p.code.as_str(), "S224");
        assert!(p.message.contains("unseen") && p.message.contains("box_case.ogf"), "{}", p.message);
        // on the object's own line of its file
        assert_eq!(p.file.as_ref().map(gz_base::RelPath::as_str), Some("levels/yard/objects.lua"));
        assert_eq!(p.line, Some(3));
    }

    /// A story id the module's spawn point carries is named after the module, or it is said at
    /// its line: another module, or the game, may carry the same and stop the game.
    #[test]
    fn a_story_id_not_named_after_the_module_is_said_at_its_line() {
        let st = gz_entities::MapSettings::new()
            .section("script", &[("current_server_entity_version", "12")])
            .section(
                "stalker",
                &[
                    ("class", "AI_STL_S"),
                    ("ef_creature_type", "1"),
                    ("going_speed", "1.5"),
                    ("terrain", "stalker_terrain"),
                    ("MaxHealthValue", "200"),
                    ("eye_range", "100"),
                    ("visual", "actors\\x\\x.ogf"),
                    ("max_item_mass", "50"),
                    ("character_profile", "sim_default"),
                ],
            );
        let spawns = [
            r#"{ name = "wolf", section = "stalker", pos = { 0, 0, 0 }, props = { custom_data = "[story_object]\nstory_id = esc_wolf\n" } },"#,
            r#"{ name = "fox", section = "stalker", pos = { 1, 0, 0 }, props = { custom_data = "[story_object]\nstory_id = my_mod_fox\n" } },"#,
            r#"{ name = "owl", section = "stalker", pos = { 2, 0, 0 } },"#,
        ];
        let texts = format::Texts {
            scene: "return {\n  scene = 2,\n}\n".into(),
            files: vec![(
                "spawns.lua".into(),
                format!("return {{\n  spawns = {{\n    {}\n  }},\n}}\n", spawns.join("\n    ")),
            )],
            layers: Vec::new(),
        };
        let main = gz_base::RelPath::new("levels/yard.scene").unwrap();
        let folder = gz_base::RelPath::new("levels/yard").unwrap();
        let read = format::read_based(&texts, &main, &folder, Some(&st), None, None).unwrap();
        assert!(read.problems.is_empty(), "{:?}", read.problems);
        let mut out = Vec::new();
        story_ids(&read, &folder, &main, "my_mod", &[], &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!((out[0].code.as_str(), out[0].line), ("S227", Some(3)));
        assert!(
            out[0].message.contains("'esc_wolf'") && out[0].message.contains("'my_mod_esc_wolf'"),
            "{}",
            out[0].message
        );
        // a scene with no module to name says nothing
        let mut none = Vec::new();
        story_ids(&read, &folder, &main, "", &[], &mut none);
        assert!(none.is_empty());
        // a whole scene of a level of the game: its objects are the game's but what is authored
        story_ids(&read, &folder, &main, "my_mod", &["yard".into()], &mut none);
        assert!(none.is_empty(), "{none:?}");
    }

    /// The name of a level of the module's own is named after the module; a level of the game is
    /// the game's.
    #[test]
    fn a_level_of_the_modules_own_is_named_after_it() {
        let texts = format::Texts {
            scene: "return {\n  scene = 2,\n}\n".into(),
            files: Vec::new(),
            layers: Vec::new(),
        };
        let main = gz_base::RelPath::new("levels/yard.scene").unwrap();
        let folder = gz_base::RelPath::new("levels/yard").unwrap();
        let read = format::read_based(&texts, &main, &folder, None, None, None).unwrap();
        let game: Vec<gz_base::SmolStr> = vec!["l01_escape".into()];
        let mut out = Vec::new();
        own_level_names(&read, &main, "my_mod", &game, &mut out);
        let codes: Vec<(&str, Option<u32>)> = out.iter().map(|p| (p.code.as_str(), p.line)).collect();
        assert_eq!(codes, [("S229", Some(1))], "{out:?}");
        assert!(
            out[0].message.contains("'yard'") && out[0].message.contains("'my_mod_yard'"),
            "{}",
            out[0].message
        );
        // a level of the game, a level of the module's name, or no game to tell one by, says nothing
        let mut none = Vec::new();
        own_level_names(&read, &main, "my_mod", &["yard".into()], &mut none);
        own_level_names(&read, &main, "yard", &game, &mut none);
        own_level_names(&read, &main, "my_mod", &[], &mut none);
        assert!(none.is_empty(), "{none:?}");
    }

    /// A stash whose name the spawn layer writes another object under is an error at its line;
    /// one of a name of its own is no problem.
    #[test]
    fn a_stash_whose_name_another_object_takes_in_the_layer_is_an_error_at_its_line() {
        let objects = [
            r#"{ name = "esc_cache", pos = { 1, 0, 0 }, ref = "dynamics/box", stash = true },"#,
            r#"{ name = "free_cache", pos = { 2, 0, 0 }, ref = "dynamics/box", stash = { loot = false } },"#,
        ];
        let spawns = r#"{ name = "esc cache", section = "inventory_box" },"#;
        let texts = format::Texts {
            scene: "return {\n  scene = 2,\n}\n".into(),
            files: vec![
                (
                    "objects.lua".into(),
                    format!("return {{\n  objects = {{\n    {}\n  }},\n}}\n", objects.join("\n    ")),
                ),
                ("spawns.lua".into(), format!("return {{\n  spawns = {{\n    {spawns}\n  }},\n}}\n")),
            ],
            layers: Vec::new(),
        };
        let main = gz_base::RelPath::new("levels/yard.scene").unwrap();
        let folder = gz_base::RelPath::new("levels/yard").unwrap();
        let mut read = format::read_based(&texts, &main, &folder, None, None, None).unwrap();
        // what the module places is its own
        let ids: Vec<_> = read.state.objects.keys().copied().collect();
        for id in ids {
            let mut o = (**read.state.get(id).unwrap()).clone();
            o.marks.authored = true;
            read.state.replace(o);
        }
        let mut out = Vec::new();
        stash_names(&read, &folder, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        let p = &out[0];
        assert_eq!((p.code.as_str(), p.severity), ("S226", gz_base::Severity::Error));
        assert!(
            p.message.contains("esc cache is written into the spawn layer under the same name"),
            "{}",
            p.message
        );
        assert_eq!(p.file.as_ref().map(gz_base::RelPath::as_str), Some("levels/yard/objects.lua"));
        assert_eq!(p.line, Some(3));
    }

    #[test]
    fn a_glow_with_a_shader_no_glow_is_drawn_with_is_said_at_its_key() {
        let glows = [
            r#"{ name = "plain", pos = { 0, 2, 0 }, texture = "glow/a" },"#,
            r#"{ name = "sun", pos = { 1, 2, 0 }, texture = "glow/a", shader = "effects/sun" },"#,
            r#"{ name = "odd", pos = { 2, 2, 0 }, texture = "glow/a", shader = "default" },"#,
        ];
        let texts = format::Texts {
            scene: "return {\n  scene = 2,\n}\n".into(),
            files: vec![(
                "glows.lua".into(),
                format!("return {{\n  glows = {{\n    {}\n  }},\n}}\n", glows.join("\n    ")),
            )],
            layers: Vec::new(),
        };
        let main = gz_base::RelPath::new("levels/yard.scene").unwrap();
        let folder = gz_base::RelPath::new("levels/yard").unwrap();
        let read = format::read_based(&texts, &main, &folder, None, None, None).unwrap();
        assert!(read.problems.is_empty(), "{:?}", read.problems);
        let mut out = Vec::new();
        glow_shaders(&read, &folder, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!((out[0].code.as_str(), out[0].line), ("S225", Some(5)));
        assert!(out[0].message.contains("odd") && out[0].message.contains("'default'"), "{}", out[0].message);
    }
}
