//! The additive spawn layer (`spawn\<level>.xspawn`).
//!
//! The delta way to put a model on a level: instead of shipping a rewritten level, the module
//! ships a spawn op and the engine composes it into the spawn registry at load. Two modules
//! dropping different props on the same map are independent ops and never touch each other; the
//! base `all.spawn` is untouched, so saves stay compatible.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use gz_base::{Finding, Findings};
use gz_entities::{Entity, Settings, Shape};
use gz_scene::math::{Vec3, get_xyz};
use gz_scene::objects::radio::RadioCx;
use gz_scene::objects::{CF_SPHERE, PointKind, SpawnEntity};
use gz_scene::{ObjClass, ObjData, Object, SceneState};

use crate::ExportError;
use crate::cfmt::{f, f3, g};
use crate::names::{config_visual, sanitize_op_name, strip_ini_section, valid_path_name};
use crate::visuals::{Found, Visuals, checked_fixed_bones, ogf_bone_names, ogf_is_ghost};

/// What one bake reads besides the scene.
pub struct SpawnInput<'a> {
    pub scene: &'a SceneState,
    pub level: String,
    pub selected_only: bool,
    /// The section of a prop's op (`physic_object` without one).
    pub section: Option<String>,
    /// A game mode gate written on every op.
    pub mode: Option<String>,
    /// The scene is the source of truth for the level's file: no merge with earlier ops, and an
    /// empty scene clears the file (the build's bake). The menu and MCP exports merge.
    pub mirror: bool,
    pub visuals: &'a Visuals,
    /// The linked game's configs.
    pub settings: Option<&'a (dyn Settings + Sync)>,
    /// The linked game's patrol names.
    pub game_patrols: &'a HashSet<String>,
    /// What tells a radio, and the module's playlists: what a radio's Plays bakes into.
    pub radio: Option<&'a RadioCx>,
    /// The `.xspawn` on disk, for the merge (empty when there is none).
    pub old_text: &'a str,
    /// The linked game loads a module's piece of a level (`level-pieces`): a static object of the
    /// module is geometry of that piece, compiled by the level build with its collision, and no
    /// object of this layer - written here as well it would stand in the game twice.
    pub statics_in_piece: bool,
}

/// The counts and lines the result dialog and `level.export kind=spawn` report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpawnResult {
    pub exported: u32,
    /// No reference, an unusable object.
    pub skipped: u32,
    /// A scale other than 1: the spawn path has no scale.
    pub scaled: u32,
    /// A model found neither in the game nor in the project.
    pub missing: u32,
    /// Shipped, but the engine draws nothing for its model.
    pub invisible: u32,
    /// Player Starts (`op = start`).
    pub starts: u32,
    /// Way paths (`[path:<name>]`).
    pub paths: u32,
    /// Placed stalkers that carry a standing order.
    pub orders: u32,
    /// Radios bound to what they play (`[xms_radio]`).
    pub radios: u32,
    /// Base spawns the mod takes out (`op = remove`).
    pub removed: u32,
    pub names: Vec<String>,
    /// What was left out or changed on the way, one line each.
    pub warnings: Vec<String>,
}

/// What a bake gives: the text and the files beside it.
#[derive(Clone, Debug, Default)]
pub struct SpawnBake {
    pub result: SpawnResult,
    /// The file's new text; `None` when a mirror pass found nothing and the file goes away.
    pub text: Option<String>,
    /// `spawn\custom_data\<name>` and its text.
    pub custom_data: Vec<(String, String)>,
    /// Models to stage: source file -> `gamedata\meshes\<visual>.ogf`.
    pub stage: Vec<(PathBuf, String)>,
    /// What the bake did and what it left out.
    pub log: Vec<Finding>,
}

impl SpawnBake {
    fn warn(&mut self, text: String) {
        self.log.warn(format!("[XMS] spawn export: {text}"));
        self.result.warnings.push(text);
    }
}

/// The entity of a spawn point, decoded with the game's configs when the scene kept it raw.
fn entity_of<'a>(
    e: &'a SpawnEntity,
    settings: Option<&(dyn Settings + Sync)>,
) -> Option<std::borrow::Cow<'a, Entity>> {
    match e {
        SpawnEntity::Decoded(d) => Some(std::borrow::Cow::Borrowed(d.as_ref())),
        raw => settings.and_then(|s| raw.decode(s)).map(std::borrow::Cow::Owned),
    }
}

/// The spawn layer from the level name on.
pub fn bake(input: &SpawnInput<'_>) -> Result<SpawnBake, ExportError> {
    let mut out = SpawnBake::default();
    let scene = input.scene;
    let level = input.level.as_str();
    // O_PHYS_S with an overridden visual is the vanilla recipe for a placed prop
    let section = input.section.as_deref().filter(|s| !s.is_empty()).unwrap_or("physic_object");
    let mode_line = input
        .mode
        .as_deref()
        .filter(|m| !m.is_empty())
        .map(|m| format!("mode      = {m}\r\n"))
        .unwrap_or_default();
    let mut fresh = String::new();
    let mut replaced: Vec<String> = Vec::new();
    // models staged by this bake: a later op naming one finds it there
    let mut staged_now: HashSet<String> = HashSet::new();

    // Sanitising collapses different scene names onto one key ("box 1" and "box-1" both become
    // "box_1"); without the suffix the later op would replace the earlier one and an object
    // would just not spawn.
    let unique_op = |replaced: &[String], scene_name: &str| -> String {
        let base = sanitize_op_name(scene_name);
        let mut op = base.clone();
        let mut dup = 2u32;
        while replaced.iter().any(|r| r.strip_prefix("obj:") == Some(op.as_str())) {
            op = format!("{}_{dup}", &base[..base.len().min(120)]);
            dup += 1;
        }
        op
    };
    let selected = |o: &Object| !input.selected_only || scene.is_selected(o.id);

    for o in scene.of_class(ObjClass::SceneObject) {
        if !selected(o) {
            continue;
        }
        // objects that came with an imported base level are the level, not the mod - shipping
        // them as adds would double every prop on the map
        if !o.marks.authored {
            continue;
        }
        let ObjData::SceneObject(so) = &o.data else {
            out.result.skipped += 1;
            continue;
        };
        if so.reference.is_empty() {
            out.result.skipped += 1;
            continue;
        }
        // a model asset: a static one is the level's geometry (the piece compiles it) and no
        // object of this layer; a dynamic one is spawned under the name the build publishes
        let asset = input.visuals.assets.get(so.reference.as_str());
        if crate::gather::is_model_asset(&so.reference) && !matches!(asset, Some(Some(_))) {
            continue;
        }
        // any other static object too, where the game loads the module's piece: the piece
        // carries its picture, its light and its collision
        if input.statics_in_piece && !so.visual_mode() && !matches!(asset, Some(Some(_))) {
            continue;
        }
        let op = unique_op(&replaced, &o.name);
        let visual = match asset {
            Some(Some(name)) => name.clone(),
            _ => config_visual(&so.reference),
        };
        let reachable = if staged_now.contains(&visual) || asset.is_some() {
            true
        } else {
            match input.visuals.props.get(so.reference.as_str()).unwrap_or(&Found::Missing) {
                Found::Staged | Found::Game => true,
                Found::Project(src) => {
                    out.stage.push((src.clone(), visual.clone()));
                    out.log.info(format!(
                        "[XMS] spawn export: staged '{}' -> gamedata\\meshes\\{visual}.ogf",
                        so.reference
                    ));
                    staged_now.insert(visual.clone());
                    true
                }
                Found::Sdk(_) | Found::Missing => false,
            }
        };
        if !reachable {
            out.warn(format!(
                "'{op}' is not baked: its model '{visual}' is neither in the game nor in the project, and an object without its model stops the game"
            ));
            out.result.missing += 1;
            out.result.skipped += 1;
            continue;
        }
        let xf = o.world();
        let angles = get_xyz(&xf);
        let s = o.scale();
        if !similar(s.x, 1.0) || !similar(s.y, 1.0) || !similar(s.z, 1.0) {
            out.result.scaled += 1;
        }
        let _ = write!(
            fresh,
            "[obj:{op}]\r\nop        = add\r\nsection   = {section}\r\nlevel     = {level}\r\nposition  = {}\r\ndirection = {}\r\nvisual    = {visual}\r\n",
            f3(xf.c()),
            f3(angles)
        );
        fresh.push_str(&mode_line);
        fresh.push_str("\r\n");
        replaced.push(format!("obj:{op}"));
        out.result.names.push(op);
        out.result.exported += 1;
    }

    // ---- gameplay objects: spawn elements travel with their REAL section ----
    for o in scene.of_class(ObjClass::SpawnPoint) {
        if !selected(o) {
            continue;
        }
        // an imported base level's spawns are the game's, not the mod's - unless the author took
        // one out: the engine drops it from new games of the module's mode
        if !o.marks.authored {
            if o.marks.removed {
                let header = format!("obj:base:{level}/{}", o.name);
                let _ = write!(fresh, "[{header}]\r\nop        = remove\r\n");
                fresh.push_str(&mode_line);
                fresh.push_str("\r\n");
                replaced.push(header);
                out.result.removed += 1;
            }
            continue;
        }
        let ObjData::Spawn(pt) = &o.data else {
            out.result.skipped += 1;
            continue;
        };
        let xf = o.world();
        let angles = get_xyz(&xf);

        // The module's Player Start. One per module - a second one is written too, so the
        // build's check sees both and names them.
        if pt.kind == PointKind::PlayerStart {
            let op = unique_op(&replaced, &o.name);
            let _ = write!(
                fresh,
                "[obj:{op}]\r\nop        = start\r\nlevel     = {level}\r\nposition  = {}\r\ndirection = {}\r\n",
                f3(xf.c()),
                f3(angles)
            );
            fresh.push_str(&mode_line);
            fresh.push_str("\r\n");
            replaced.push(format!("obj:{op}"));
            out.result.names.push(op);
            out.result.exported += 1;
            out.result.starts += 1;
            continue;
        }
        if pt.kind != PointKind::Spawn {
            out.result.skipped += 1;
            continue;
        }
        let Some(raw) = &pt.entity else {
            out.result.skipped += 1;
            continue;
        };
        let sp_section = raw.section().to_owned();
        if sp_section.is_empty() {
            out.result.skipped += 1;
            continue;
        }
        let Some(entity) = entity_of(raw, input.settings) else {
            // the configs that read the entity are not there: nothing of it can be baked safely
            out.warn(format!(
                "'{}' ({sp_section}) is not baked: it cannot be read without the linked game's configs",
                o.name
            ));
            out.result.skipped += 1;
            continue;
        };
        let op = unique_op(&replaced, &o.name);
        let object = entity.object();

        // The model is resolved BEFORE the op is written: a model the game cannot load skips
        // the WHOLE op - a physic entity that spawns without a visual crashes the client inside
        // net_Spawn.
        let vname = entity_visual(&entity, input.settings);
        let section_visual = input
            .settings
            .filter(|s| s.section_exist(&sp_section))
            .and_then(|s| s.r_string(&sp_section, "visual"));
        let mut bytes = None;
        if !vname.is_empty() {
            let info = input.visuals.spawns.get(&vname);
            let found = if staged_now.contains(&vname) {
                Found::Staged
            } else {
                info.map_or(Found::Missing, |i| i.found.clone())
            };
            match found {
                Found::Staged | Found::Game | Found::Project(_) => {}
                Found::Sdk(src) => {
                    // the chooser lists the editor's own meshes too: a model picked there is staged
                    // into the project, so the module carries it like any project mesh
                    out.stage.push((src, vname.clone()));
                    out.log.info(format!(
                        "[XMS] spawn export: staged an editor model -> gamedata\\meshes\\{vname}.ogf"
                    ));
                    staged_now.insert(vname.clone());
                }
                Found::Missing => {
                    out.warn(format!(
                        "'{op}' is not baked: its model '{vname}' is neither in the game, the project, nor the editor's data, and an object without its model stops the game"
                    ));
                    out.result.missing += 1;
                    out.result.skipped += 1;
                    continue;
                }
            }
            bytes = info.and_then(|i| i.bytes.clone());
        }
        // no model anywhere is the same crash spelled differently; a section with its own
        // default (the inventory_box ghost) is fine without a line
        if vname.is_empty() && section_visual.is_none() && object.and_then(|ob| ob.physic()).is_some() {
            out.warn(format!(
                "'{op}' ({sp_section}) is not baked: it has no model at all, and an object without one stops the game - give it one (`props.visual`)"
            ));
            out.result.missing += 1;
            out.result.skipped += 1;
            continue;
        }

        let _ = write!(
            fresh,
            "[obj:{op}]\r\nop        = add\r\nsection   = {sp_section}\r\nlevel     = {level}\r\nposition  = {}\r\ndirection = {}\r\n",
            f3(xf.c()),
            f3(angles)
        );
        if !vname.is_empty() {
            let _ = write!(fresh, "visual    = {vname}\r\n");
            // a collision-only model (named *collider*) is unseen by design, and so is the model
            // of a section whose own model is unseen (a stash's case)
            let own_way = section_visual
                .map(config_visual)
                .and_then(|own| input.visuals.spawns.get(&own))
                .and_then(|own| own.bytes.as_deref())
                .is_some_and(ogf_is_ghost);
            if !vname.contains("collider") && !own_way && bytes.as_deref().is_some_and(ogf_is_ghost) {
                out.result.invisible += 1;
                out.warn(format!(
                    "'{op}' wears '{vname}', a model the game draws nothing for: the object works and nobody sees it"
                ));
            }
        }
        let bones_of = |out: &mut SpawnBake, fixed: &str| -> String {
            if vname.is_empty() {
                return fixed.to_owned();
            }
            let names = bytes.as_deref().map(ogf_bone_names).unwrap_or_default();
            let (bones, bad) = checked_fixed_bones(&names, fixed);
            if !bad.is_empty() {
                out.warn(format!("'{op}': fixed bone(s) '{bad}' are not bones of '{vname}' - dropped, the game would assert on them"));
            }
            bones
        };

        // Fixed bones share the visual's fate: an op without the line spawns a FREE physics
        // body - the chest slides when pushed instead of standing where the author nailed it.
        if let Some(po) = object.and_then(|ob| ob.physic())
            && !po.fixed_bones.is_empty()
        {
            let bones = bones_of(&mut out, &po.fixed_bones);
            let _ = write!(fresh, "fixed_bones = {bones}\r\n");
        }
        // The description picked in the smart cover's properties is what makes it an animpoint.
        if let Some(sc) = object.and_then(|ob| ob.cover()) {
            if sc.description.is_empty() {
                out.warn(format!("'{op}' is a smart cover without a description - nobody can use it; pick one in its properties"));
            } else {
                let _ = write!(fresh, "smart_cover = {}\r\n", sc.description);
            }
        }
        // A hanging lamp's light is its own fields, not its section: without them the game
        // lights every lamp white, 10 m, as a spot along its local +Z.
        if let Some(l) = object.and_then(|ob| ob.lamp()) {
            let _ = write!(
                fresh,
                "lamp_flags = {}\r\nlamp_color = {:08x}\r\nlamp_light = {},{},{},{}\r\nlamp_bones = {},{}\r\nlamp_ambient = {},{},{}\r\nlamp_volumetric = {},{},{}\r\nlamp_glow = {},{}\r\nlamp_health = {}\r\n",
                u32::from(l.flags),
                l.color,
                f(l.brightness),
                f(l.range),
                f(l.spot_cone_angle),
                f(l.virtual_size),
                l.light_main_bone,
                l.light_ambient_bone,
                f(l.ambient_radius),
                f(l.ambient_power),
                l.ambient_texture,
                f(l.volumetric_quality),
                f(l.volumetric_intensity),
                f(l.volumetric_distance),
                f(l.glow_radius),
                l.glow_texture,
                f(l.health),
            );
            if !l.color_animator.is_empty() {
                let _ = write!(fresh, "lamp_animator = {}\r\n", l.color_animator);
            }
            if !l.light_texture.is_empty() {
                let _ = write!(fresh, "lamp_texture = {}\r\n", l.light_texture);
            }
            if !l.fixed_bones.is_empty() {
                let bones = bones_of(&mut out, &l.fixed_bones);
                let _ = write!(fresh, "fixed_bones = {bones}\r\n");
            }
        }
        // The zone's own geometry: a restrictor composed without it has an empty shape list,
        // so nothing is ever inside it. Baked here, not applied to the scene, because exporting
        // a mod must not rewrite the scene it is exporting.
        if let Some(a) = &pt.attached
            && let ObjData::Shape(shape) = &a.data
            && !shape.shapes.is_empty()
        {
            let _ = write!(fresh, "shapes    = {}\r\n", shape.shapes.len());
            let sc = a.xform.scale;
            let s_matrix = gz_scene::math::scale_matrix(sc);
            for (si, sh) in shape.shapes.iter().enumerate() {
                if sh.kind == CF_SPHERE {
                    let p = s_matrix.transform_tiny(sh.center());
                    let r = sh.radius() * sc.x;
                    let _ =
                        write!(fresh, "shape{si}    = sphere,{},{},{},{}\r\n", f(p.x), f(p.y), f(p.z), f(r));
                } else {
                    let b = sh.matrix().mul_a43(&s_matrix);
                    let (i, j, k, c) = (b.row(0), b.row(1), b.row(2), b.c());
                    let _ = write!(fresh, "shape{si}    = box,{},{},{},{}\r\n", f3(i), f3(j), f3(k), f3(c));
                }
            }
        }

        // logic: written as a module file so it stays multi-line ltx, and the op only carries
        // the @reference the engine resolves inside the module
        let mut cd = entity.custom_data().unwrap_or_default().to_owned();
        // the standing order rides in the same text: the game reads [xms_order] off the NPC's
        // spawn ini once it is online
        let order = &pt.order;
        if !order.is_empty() {
            if !pt.can_take_order() && !entity_takes_order(&entity) {
                out.warn(format!("'{op}' ({sp_section}) is no stalker - its order is left out"));
            } else if order.kind == 2 && order.route.is_empty() {
                out.warn(format!(
                    "'{op}' patrols without a route - its order is left out; pick one in Order\\Route"
                ));
            } else {
                if order.kind == 2
                    && scene.find(ObjClass::Way, &order.route).is_none()
                    && !input.game_patrols.contains(order.route.as_str())
                {
                    out.warn(format!("'{op}' patrols '{}', which is neither a way path of this level nor a path of the game", order.route));
                }
                if strip_ini_section(&mut cd, "xms_order") {
                    out.warn(format!("'{op}': the [xms_order] written into its custom data is replaced by its Order properties"));
                }
                if !cd.is_empty() && !cd.ends_with('\n') {
                    cd.push_str("\r\n");
                }
                cd.push_str(&order.section());
                out.result.orders += 1;
            }
        }
        // what a radio plays rides in it too: the game reads [xms_radio] off the radio's spawn ini
        if !pt.plays.is_empty() {
            bake_plays(&mut out, input.radio, &entity, &op, &sp_section, &pt.plays, &mut cd);
        }
        if !cd.is_empty() {
            let file = format!("{level}_{op}.ltx");
            let _ = write!(fresh, "custom_data = @spawn/custom_data/{file}\r\n");
            out.custom_data.push((file, cd));
        }
        fresh.push_str(&mode_line);
        fresh.push_str("\r\n");
        replaced.push(format!("obj:{op}"));
        out.result.names.push(op);
        out.result.exported += 1;
    }

    // ---- way paths: the module's own patrol paths ----
    // The engine registers them when the level loads, next to the level's own. The game's
    // names are the game's: a path named like one is left out rather than losing to it.
    for o in scene.of_class(ObjClass::Way) {
        if !selected(o) || !o.marks.authored {
            continue;
        }
        let ObjData::Way(way) = &o.data else {
            out.result.skipped += 1;
            continue;
        };
        let name = o.name.as_str();
        if !valid_path_name(name) {
            out.warn(format!(
                "way path '{name}' is left out - a path name takes a-z, 0-9, _ - . only; rename it"
            ));
            out.result.skipped += 1;
            continue;
        }
        if input.game_patrols.contains(name) {
            out.warn(format!(
                "way path '{name}' is left out - the game has a path of that name; rename yours"
            ));
            out.result.skipped += 1;
            continue;
        }
        if way.points.is_empty() {
            out.result.skipped += 1;
            continue;
        }
        let _ = write!(fresh, "[path:{name}]\r\nlevel  = {level}\r\npoints = {}\r\n", way.points.len());
        let mut links: Vec<(usize, u32, f32)> = Vec::new();
        for (i, p) in way.points.iter().enumerate() {
            let _ = write!(fresh, "p{i} = {}\r\n", f3(p.position));
            if p.flags != 0 {
                let _ = write!(fresh, "p{i}_flags = {}\r\n", p.flags);
            }
            if !p.name.is_empty() {
                let _ = write!(fresh, "p{i}_name = {}\r\n", p.name);
            }
            for (to, w) in &p.links {
                if (*to as usize) < way.points.len() {
                    links.push((i, *to, *w));
                }
            }
        }
        if !links.is_empty() {
            fresh.push_str("links  = ");
            for (n, (from, to, w)) in links.iter().enumerate() {
                let _ = write!(fresh, "{}{from}-{to}:{}", if n > 0 { ", " } else { "" }, g(*w));
            }
            fresh.push_str("\r\n");
        }
        fresh.push_str(&mode_line);
        fresh.push_str("\r\n");
        replaced.push(format!("path:{name}"));
        out.result.names.push(name.to_owned());
        out.result.paths += 1;
        out.result.exported += 1;
    }

    // base spawns taken out are not counted in `exported`, so a scene that only removes
    // base spawns is "empty": its mirror pass deletes the file and a merge export refuses.
    if out.result.exported == 0 {
        if input.mirror {
            out.log.info(format!("[XMS] spawn layer: scene is empty, {level} cleared"));
            out.custom_data.clear();
            out.stage.clear();
            return Ok(out);
        }
        return Err(ExportError::new(if input.selected_only {
            "none of the objects chosen is one the module adds: the spawn layer holds only those"
        } else {
            "the scene has no placeable objects"
        }));
    }
    out.text = Some(if input.mirror { fresh } else { merge_spawn_text(input.old_text, &replaced, &fresh) });
    Ok(out)
}

/// The visual a spawn element's op names: its model in the configs' form, empty when it has
/// none or keeps its section's own (the game re-derives that one, and a stash keeping its ghost
/// DEFAULT is the vanilla recipe, not a mistake).
#[must_use]
pub fn spawn_visual(e: &SpawnEntity, settings: Option<&(dyn Settings + Sync)>) -> String {
    entity_of(e, settings).map(|e| entity_visual(&e, settings)).unwrap_or_default()
}

fn entity_visual(entity: &Entity, settings: Option<&(dyn Settings + Sync)>) -> String {
    let mut vname = entity.visual_name().map(config_visual).unwrap_or_default();
    let section = entity.section();
    if let Some(def) =
        settings.filter(|s| s.section_exist(section)).and_then(|s| s.r_string(section, "visual"))
        && !vname.is_empty()
        && vname.eq_ignore_ascii_case(&config_visual(def))
    {
        vname.clear();
    }
    vname
}

/// A radio's `[xms_radio]` appended to its custom data, or why it is left out.
fn bake_plays(
    out: &mut SpawnBake,
    radio: Option<&RadioCx>,
    entity: &Entity,
    op: &str,
    section: &str,
    plays: &str,
    cd: &mut String,
) {
    let Some(r) = radio.filter(|r| r.radios.is_radio(entity)) else {
        out.warn(format!("'{op}' ({section}) is no radio - what it plays is left out"));
        return;
    };
    let text = match r.section(plays) {
        Ok(t) => t,
        Err(why) => {
            out.warn(format!("'{op}': {why} - what it plays is left out; pick it in Radio\\Plays"));
            return;
        }
    };
    if let Some(id) = r.missing(plays) {
        out.warn(format!(
            "'{op}' plays the playlist '{id}', which mod.ltx does not declare - the radio plays the player's station until it does"
        ));
    }
    if strip_ini_section(cd, "xms_radio") {
        out.warn(format!(
            "'{op}': the [xms_radio] written into its custom data is replaced by its Radio properties"
        ));
    }
    if !cd.is_empty() && !cd.ends_with('\n') {
        cd.push_str("\r\n");
    }
    cd.push_str(&text);
    out.result.radios += 1;
}

/// Whether a point can take an order, for a point whose entity was decoded for this bake only.
fn entity_takes_order(e: &Entity) -> bool {
    e.class_info().is_some_and(|c| c.clsid.trim() == "AI_STL_S")
}

/// `fsimilar(a, b)` with `EPS_L`.
fn similar(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.0001
}

/// The entries exported now rewritten, every other section kept verbatim -
/// hand-written ops in the same file survive a re-export.
#[must_use]
pub fn merge_spawn_text(old: &str, replaced: &[String], fresh: &str) -> String {
    let mut out = String::with_capacity(old.len() + fresh.len());
    let mut dropping = false;
    let mut pos = 0usize;
    loop {
        let eol = old[pos..].find('\n').map_or(old.len(), |i| pos + i);
        let line = &old[pos..eol];
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.starts_with('[') {
            let header = trimmed.find(']').map_or("", |close| &trimmed[1..close]);
            dropping = replaced.iter().any(|r| r == header);
        }
        if !dropping {
            out.push_str(line);
            if eol < old.len() {
                out.push('\n');
            }
        }
        if eol == old.len() {
            break;
        }
        pos = eol + 1;
    }
    while out.ends_with(['\n', '\r']) {
        out.pop();
    }
    if !out.is_empty() {
        out.push_str("\r\n\r\n");
    }
    out.push_str(fresh);
    out
}

/// The shapes of a zone as the entity holds them, in world-free form: what `bake` writes for an
/// attached shape. Exposed for the level build's restrictor checks.
#[must_use]
pub fn entity_shapes(scale: Vec3, shapes: &[gz_scene::objects::ShapeDef]) -> Vec<Shape> {
    gz_scene::objects::entity_shapes(scale, shapes)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gz_entities::MapSettings;
    use gz_scene::objects::{SceneObjectData, SpawnData, WayData, WayPoint, XmsOrder};

    use super::*;

    #[test]
    fn a_bake_logs_at_the_level_of_each_line() {
        use gz_base::Severity;
        let mut bake = SpawnBake::default();
        bake.warn("something was left out".to_owned());
        bake.log.info("staged a model");
        bake.log.error("the file could not be written");
        assert_eq!(
            bake.log.iter().map(|f| f.severity).collect::<Vec<_>>(),
            [Severity::Warning, Severity::Note, Severity::Error]
        );
        assert!(bake.log.iter().all(|f| !f.text.starts_with(['!', '~', '*'])));
        assert_eq!(bake.result.warnings, ["something was left out"]);
    }

    fn settings() -> MapSettings {
        MapSettings::new().section("script", &[("current_server_entity_version", "12")]).section(
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
        )
    }

    fn authored(mut o: Object, pos: Vec3) -> Object {
        o.xform.position = pos;
        o.marks.authored = true;
        o
    }

    /// A prop, a Player Start, a way path and a stalker who patrols it.
    fn scene(st: &MapSettings) -> SceneState {
        let mut s = SceneState::default();
        s.insert(authored(
            Object::new(
                ObjClass::SceneObject,
                "box 1",
                ObjData::SceneObject(SceneObjectData::library("meshes/props/box.ogf")),
            ),
            Vec3::new(1.0, 2.0, 3.0),
        ));
        s.insert(authored(
            Object::new(
                ObjClass::SpawnPoint,
                "start",
                ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::PlayerStart))),
            ),
            Vec3::new(10.0, 0.5, -4.25),
        ));
        let mut way = WayData {
            kind: 0,
            points: vec![
                WayPoint::new("wp00", Vec3::new(0.0, 0.0, 0.0)),
                WayPoint::new("", Vec3::new(5.0, 0.0, 0.0)),
            ],
        };
        way.points[0].flags = 1;
        way.points[0].links.push((1, 1.0));
        way.points[1].links.push((0, 0.5));
        s.insert(authored(Object::new(ObjClass::Way, "walk_1", ObjData::Way(way)), Vec3::ZERO));
        let entity = Entity::create("stalker", st).unwrap();
        let mut sd = SpawnData::of_kind(PointKind::Spawn);
        sd.entity = Some(SpawnEntity::Decoded(Arc::new(entity)));
        sd.order = XmsOrder { kind: 2, route: "walk_1".into(), flags: 1, ..XmsOrder::default() };
        s.insert(authored(
            Object::new(ObjClass::SpawnPoint, "guard", ObjData::Spawn(Box::new(sd))),
            Vec3::new(-1.0, 0.0, 2.0),
        ));
        s
    }

    fn dir(s: &SceneState, name: &str) -> String {
        let o = s.find_any(name).unwrap();
        f3(get_xyz(&o.world()))
    }

    /// A static model asset is the level's geometry and no object of the spawn layer, and its
    /// asset file is never staged as a model; a dynamic one is spawned under the name the build
    /// publishes it by.
    #[test]
    fn a_model_asset_is_never_a_raw_model_file() {
        let st = settings();
        let mut s = SceneState::default();
        for (name, reference) in [("hut", "models/hut.meshasset"), ("crate", "models/crate.meshasset")] {
            s.insert(authored(
                Object::new(
                    ObjClass::SceneObject,
                    name,
                    ObjData::SceneObject(SceneObjectData::library(reference)),
                ),
                Vec3::new(1.0, 2.0, 3.0),
            ));
        }
        let mut visuals = Visuals::default();
        visuals.assets.insert("models/hut.meshasset".into(), None);
        visuals.assets.insert("models/crate.meshasset".into(), Some("my_mod\\crate".into()));
        let patrols = HashSet::new();
        let input = SpawnInput {
            scene: &s,
            level: "l01_escape".into(),
            selected_only: false,
            section: None,
            mode: None,
            mirror: false,
            visuals: &visuals,
            settings: Some(&st),
            game_patrols: &patrols,
            radio: None,
            old_text: "",
            statics_in_piece: false,
        };
        let bake = bake(&input).unwrap();
        let text = bake.text.clone().unwrap_or_default();
        assert!(bake.stage.is_empty(), "nothing of an asset is staged as a model: {:?}", bake.stage);
        assert!(!text.contains("[obj:hut]"), "{text}");
        assert!(text.contains("[obj:crate]"), "{text}");
        assert!(text.contains("my_mod\\crate"), "{text}");
        assert!(!text.contains("meshasset"), "{text}");
        // an asset the gather did not see (no library at hand) is left out, never staged
        let bare = Visuals::default();
        let input = SpawnInput { visuals: &bare, mirror: true, ..input };
        let bake = super::bake(&input).unwrap();
        let text = bake.text.clone().unwrap_or_default();
        assert!(bake.stage.is_empty() && !text.contains("[obj:"), "{text}");
        // a game that loads the module's piece: a static object the piece compiles is no object of
        // this layer whatever its model is, a dynamic model asset still is
        let mut both = s.clone();
        both.insert(authored(
            Object::new(
                ObjClass::SceneObject,
                "box",
                ObjData::SceneObject(SceneObjectData::library("props_box.ogf")),
            ),
            Vec3::new(4.0, 2.0, 3.0),
        ));
        let mut found = visuals.clone();
        found.props.insert("props_box.ogf".into(), Found::Game);
        let with_piece =
            SpawnInput { scene: &both, visuals: &found, statics_in_piece: true, mirror: false, ..input };
        let text = super::bake(&with_piece).unwrap().text.unwrap_or_default();
        assert!(!text.contains("[obj:box]") && text.contains("[obj:crate]"), "{text}");
        let without = SpawnInput { statics_in_piece: false, ..with_piece };
        let text = super::bake(&without).unwrap().text.unwrap_or_default();
        assert!(text.contains("[obj:box]"), "a game without pieces gets the prop as an object: {text}");
        assert!(crate::gather::is_model_asset("Models/Hut.MeshAsset"));
        assert!(!crate::gather::is_model_asset("meshes/props/box.ogf"));
        assert!(!crate::gather::is_model_asset(".meshasset"));
        // a reference in other letters is no asset and no panic
        assert!(!crate::gather::is_model_asset("ящик/стол_01"));
        assert!(crate::gather::is_model_asset("модели/стол.meshasset"));
    }

    #[test]
    fn the_spawn_layer_is_written_in_its_fixed_form() {
        let st = settings();
        let s = scene(&st);
        let mut visuals = Visuals::default();
        visuals.props.insert("meshes/props/box.ogf".into(), Found::Game);
        let patrols = HashSet::new();
        let input = SpawnInput {
            scene: &s,
            level: "l01_escape".into(),
            selected_only: false,
            section: None,
            mode: Some("story".into()),
            mirror: true,
            visuals: &visuals,
            settings: Some(&st),
            game_patrols: &patrols,
            radio: None,
            old_text: "",
            statics_in_piece: false,
        };
        let bake = bake(&input).unwrap();
        let expected = format!(
            "[obj:box_1]\r\nop        = add\r\nsection   = physic_object\r\nlevel     = l01_escape\r\nposition  = 1.000000,2.000000,3.000000\r\ndirection = {}\r\nvisual    = props\\box\r\nmode      = story\r\n\r\n\
             [obj:start]\r\nop        = start\r\nlevel     = l01_escape\r\nposition  = 10.000000,0.500000,-4.250000\r\ndirection = {}\r\nmode      = story\r\n\r\n\
             [obj:guard]\r\nop        = add\r\nsection   = stalker\r\nlevel     = l01_escape\r\nposition  = -1.000000,0.000000,2.000000\r\ndirection = {}\r\ncustom_data = @spawn/custom_data/l01_escape_guard.ltx\r\nmode      = story\r\n\r\n\
             [path:walk_1]\r\nlevel  = l01_escape\r\npoints = 2\r\np0 = 0.000000,0.000000,0.000000\r\np0_flags = 1\r\np0_name = wp00\r\np1 = 5.000000,0.000000,0.000000\r\nlinks  = 0-1:1, 1-0:0.5\r\nmode      = story\r\n\r\n",
            dir(&s, "box 1"),
            dir(&s, "start"),
            dir(&s, "guard"),
        );
        assert_eq!(bake.text.as_deref(), Some(expected.as_str()));
        assert_eq!(
            bake.custom_data,
            // a new stalker's custom data is `CSE_ALifeTraderAbstract`'s "\r\n[game_info]\r\n",
            // which is written as it is before the order
            [(
                "l01_escape_guard.ltx".to_owned(),
                "\r\n[game_info]\r\n[xms_order]\r\norder = patrol\r\nroute = walk_1\r\nmove = run\r\n"
                    .to_owned()
            )]
        );
        let r = &bake.result;
        assert_eq!((r.exported, r.starts, r.paths, r.orders, r.skipped, r.missing), (4, 1, 1, 1, 0, 0));
        assert_eq!(r.names, ["box_1", "start", "guard", "walk_1"]);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn warnings_name_what_is_left_out() {
        let st = settings();
        let mut s = scene(&st);
        // a path the game already has, a route nobody knows, a model nowhere
        let patrols: HashSet<String> = ["walk_1".to_owned()].into();
        let guard = s.find_any("guard").unwrap().as_ref().clone();
        let mut g2 = guard.clone();
        if let ObjData::Spawn(sd) = &mut g2.data {
            sd.order.route = "nowhere".into();
        }
        s.replace(g2);
        let visuals = Visuals::default();
        let input = SpawnInput {
            scene: &s,
            level: "l01".into(),
            selected_only: false,
            section: None,
            mode: None,
            mirror: false,
            visuals: &visuals,
            settings: Some(&st),
            game_patrols: &patrols,
            radio: None,
            old_text: "[obj:hand_made]\r\nop        = add\r\n\r\n[obj:guard]\r\nop = add\r\n",
            statics_in_piece: false,
        };
        let bake = bake(&input).unwrap();
        let r = &bake.result;
        assert_eq!(r.missing, 1);
        assert_eq!(
            r.warnings,
            [
                "'box_1' is not baked: its model 'props\\box' is neither in the game nor in the project, and an object without its model stops the game",
                "'guard' patrols 'nowhere', which is neither a way path of this level nor a path of the game",
                "way path 'walk_1' is left out - the game has a path of that name; rename yours",
            ]
        );
        let text = bake.text.unwrap();
        // the hand-written op stays, the exported one is rewritten
        assert!(text.starts_with("[obj:hand_made]\r\nop        = add\r\n\r\n[obj:start]\r\n"));
        assert_eq!(text.matches("[obj:guard]").count(), 1);
    }

    /// An object whose model nobody sees is named in the bake's warnings - unless its section's
    /// own model is unseen too (a stash is an unseen case inside something drawn): that is the
    /// section's way and nothing is said.
    #[test]
    fn an_unseen_model_is_named_unless_it_is_the_sections_way() {
        fn chunk(id: u32, body: &[u8]) -> Vec<u8> {
            let mut v = id.to_le_bytes().to_vec();
            v.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
            v.extend_from_slice(body);
            v
        }
        fn model(texture: &[u8]) -> crate::visuals::SpawnVisual {
            let mut file = chunk(1, &[0u8; 8]);
            file.extend(chunk(9, &chunk(0, &chunk(2, texture))));
            crate::visuals::SpawnVisual { found: Found::Game, bytes: Some(Arc::from(file)) }
        }
        let sections = |crate_model: &'static str, stash_model: &'static str| {
            settings()
                .section("physic_object", &[("class", "O_PHYS_S"), ("visual", crate_model)])
                .section("inventory_box", &[("class", "S_INVBOX"), ("visual", stash_model)])
        };
        // both are placed wearing the second case, on sections whose own models differ
        let placed = sections("dynamics\\case_2.ogf", "dynamics\\case_2.ogf");
        let st = sections("dynamics\\barrel.ogf", "dynamics\\case_1.ogf");
        let mut s = SceneState::default();
        for (name, section) in [("crate", "physic_object"), ("stash", "inventory_box")] {
            let mut sd = SpawnData::of_kind(PointKind::Spawn);
            sd.entity = Some(SpawnEntity::Decoded(Arc::new(Entity::create(section, &placed).unwrap())));
            s.insert(authored(
                Object::new(ObjClass::SpawnPoint, name, ObjData::Spawn(Box::new(sd))),
                Vec3::ZERO,
            ));
        }
        let unseen: &[u8] = b"glow\\glow_ghost\0default\0";
        let mut visuals = Visuals::default();
        visuals.spawns.insert("dynamics\\case_1".into(), model(unseen));
        visuals.spawns.insert("dynamics\\case_2".into(), model(unseen));
        visuals.spawns.insert("dynamics\\barrel".into(), model(b"props\\barrel\0models\\model\0"));
        let patrols = HashSet::new();
        let input = SpawnInput {
            scene: &s,
            level: "l01".into(),
            selected_only: false,
            section: None,
            mode: None,
            mirror: true,
            visuals: &visuals,
            settings: Some(&st),
            game_patrols: &patrols,
            radio: None,
            old_text: "",
            statics_in_piece: false,
        };
        let bake = bake(&input).unwrap();
        let text = bake.text.clone().unwrap_or_default();
        assert!(text.contains("[obj:crate]") && text.contains("[obj:stash]"), "{text}");
        assert_eq!(bake.result.invisible, 1);
        assert_eq!(
            bake.result.warnings,
            [
                "'crate' wears 'dynamics\\case_2', a model the game draws nothing for: the object works and nobody sees it"
            ]
        );
    }

    /// A radio's Plays rides in its custom data as `[xms_radio]`, keyed by the module; a playlist
    /// `mod.ltx` lacks is said; a hand-written `[xms_radio]` gives way; a playlist on something
    /// that is no radio is left out with a warning.
    #[test]
    fn a_radio_bakes_what_it_plays() {
        use gz_entities::{PropContext, PropValue};
        use gz_scene::objects::radio::{RADIO_LOGIC, Radios};
        let st = settings().section(
            "physic_object",
            &[("class", "O_PHYS_S"), ("visual", "dynamics\\radio.ogf"), ("fixed_bones", "link")],
        );
        let mut s = SceneState::default();
        let ctx = PropContext::default();
        for (name, logic, plays) in [
            ("radio", format!("{RADIO_LOGIC}[xms_radio]\r\nplaylist = off\r\n"), "road"),
            ("lost", RADIO_LOGIC.to_owned(), "gone"),
            ("barrel", String::new(), "game"),
        ] {
            let mut e = Entity::create("physic_object", &st).unwrap();
            let path = e.properties(&ctx).into_iter().find(|p| p.key() == "custom_data").unwrap().path;
            e.set(&path, &PropValue::Text(logic), &ctx).unwrap();
            let mut sd = SpawnData::of_kind(PointKind::Spawn);
            sd.entity = Some(SpawnEntity::Decoded(Arc::new(e)));
            sd.plays = plays.into();
            s.insert(authored(
                Object::new(ObjClass::SpawnPoint, name, ObjData::Spawn(Box::new(sd))),
                Vec3::ZERO,
            ));
        }
        let radio = RadioCx {
            radios: Arc::new(Radios::unlinked()),
            module: "my_mod".into(),
            playlists: Arc::from(vec![gz_mod_playlist("road")]),
        };
        let (visuals, patrols) = (Visuals::default(), HashSet::new());
        let input = SpawnInput {
            scene: &s,
            level: "l01".into(),
            selected_only: false,
            section: None,
            mode: None,
            mirror: true,
            visuals: &visuals,
            settings: Some(&st),
            game_patrols: &patrols,
            radio: Some(&radio),
            old_text: "",
            statics_in_piece: false,
        };
        let bake = bake(&input).unwrap();
        let cd = |op: &str| {
            bake.custom_data.iter().find(|(f, _)| f == &format!("l01_{op}.ltx")).map(|(_, t)| t.clone())
        };
        assert_eq!(cd("radio"), Some(format!("{RADIO_LOGIC}[xms_radio]\r\nplaylist = my_mod.road\r\n")));
        assert_eq!(cd("lost"), Some(format!("{RADIO_LOGIC}[xms_radio]\r\nplaylist = my_mod.gone\r\n")));
        assert_eq!(cd("barrel"), None);
        assert_eq!(bake.result.radios, 2);
        assert_eq!(
            bake.result.warnings,
            [
                "'radio': the [xms_radio] written into its custom data is replaced by its Radio properties",
                "'lost' plays the playlist 'gone', which mod.ltx does not declare - the radio plays the player's station until it does",
                "'barrel' (physic_object) is no radio - what it plays is left out",
            ]
        );
    }

    fn gz_mod_playlist(id: &str) -> gz_scene::objects::radio::RadioPlaylist {
        gz_scene::objects::radio::RadioPlaylist { id: id.into(), title: String::new() }
    }

    #[test]
    fn op_names_are_made_unique() {
        let mut s = SceneState::default();
        for n in ["box 1", "box-1", "box_1"] {
            s.insert(authored(
                Object::new(
                    ObjClass::SceneObject,
                    n,
                    ObjData::SceneObject(SceneObjectData::library("props\\box")),
                ),
                Vec3::ZERO,
            ));
        }
        let mut visuals = Visuals::default();
        visuals.props.insert("props\\box".into(), Found::Staged);
        let patrols = HashSet::new();
        let input = SpawnInput {
            scene: &s,
            level: "l".into(),
            selected_only: false,
            section: Some("my_prop".into()),
            mode: None,
            mirror: true,
            visuals: &visuals,
            settings: None,
            game_patrols: &patrols,
            radio: None,
            old_text: "",
            statics_in_piece: false,
        };
        let b = bake(&input).unwrap();
        assert_eq!(b.result.names, ["box_1", "box-1", "box_1_2"]);
        assert!(b.text.unwrap().contains("section   = my_prop\r\n"));
    }

    #[test]
    fn merge_keeps_other_sections() {
        let old = "[obj:a]\r\nop = add\r\n\r\n[obj:b]\r\nop = add\r\n\r\n";
        let out = merge_spawn_text(old, &["obj:a".to_owned()], "[obj:a]\r\nop        = add\r\n\r\n");
        assert_eq!(out, "[obj:b]\r\nop = add\r\n\r\n[obj:a]\r\nop        = add\r\n\r\n");
    }
}
