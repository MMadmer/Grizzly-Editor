//! The world's commands (`EditorWorld::McpGet` / `McpSet` / `McpTransitions`) and what the area
//! hangs on the editor.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Origin, Problem};
use gz_build::{BuildCheck, BuildChecks, BuildWalk};
use gz_editor::{Args, CmdError, Editor, ProjectSession, Registry, Reply};
use gz_mod::{Edit, Manifest, ManifestDoc, ManifestFields};
use gz_project::ProjectHooks;
use gz_screens::GameLevels;
use serde_json::{Value, json};

use crate::game::GameWorld;
use crate::model::{KitItem, MAX_PLAYER_CHOICES, RULES, StartKit, World, base_kit, rule_index, toggle};
use crate::service::WorldService;
use crate::starts::{Start, fixed3, starts_json};

const AREA: &str = "gz-world";

/// The game file the base kit comes from.
const FACTION_LOADOUTS: &str = "configs\\plugins\\faction_loadouts.ltx";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "world_get", world_get);
    reg.add(AREA, "world_set", world_set);
    reg.add(AREA, "world_transitions", world_transitions);
}

/// What the area hangs on the editor: the `world` field of `mod_manifest`, the levels
/// `gz-screens` checks `W407` against, the build's check, and the service made when a project
/// opens, so that `gz_nq`'s pickers know the level changers and the patrols.
pub fn install(editor: &Editor) {
    ManifestFields::of(editor).provide("world", |state| state.doc().map_or_else(|| World::default().json(), |d| World::parse(d).json()));
    GameLevels::of(editor).provide(|ed| {
        Box::pin(async move {
            let svc = WorldService::of(&ed).await.ok()?;
            let game = svc.game().await;
            game.ok.then(|| game.levels.clone())
        })
    });
    BuildChecks::of(editor).add(Arc::new(WorldCheck));
    ProjectHooks::of(editor).on_open(|ed, _session| {
        let ed = ed.clone();
        ed.exec.clone().spawn(async move {
            // nothing is read here: the game's world is read when it is first asked
            let _ = WorldService::of(&ed).await;
        });
    });
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

// ---- arguments the way GrizzlyMCP read them -------------------------------------------------

/// `ArgString`: present means given, even empty; a bare number or flag is its text.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(a) => Some(a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(",")),
        _ => None,
    }
}

/// `ArgBool`: `true` / `1` / `false` / `0` at the start of the value; absent otherwise.
fn arg_bool(args: &Args, key: &str) -> Option<bool> {
    match args.get(key)? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.to_string().chars().next() {
            Some('1') => Some(true),
            Some('0') => Some(false),
            _ => None,
        },
        Value::String(s) => {
            let t = s.trim_start_matches(' ');
            if t.starts_with("true") || t.starts_with('1') {
                Some(true)
            } else if t.starts_with("false") || t.starts_with('0') {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// `ArgInt`: a number (cut toward zero) or text that starts like one.
fn arg_int(args: &Args, key: &str) -> Option<i64> {
    match args.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|v| v as i64)),
        Value::String(s) => {
            let t = s.trim_start_matches([' ', '\t']);
            t.starts_with(|c: char| c == '-' || c == '+' || c.is_ascii_digit()).then(|| gz_nq::text::c_atoi(t))
        }
        _ => None,
    }
}

/// `ArgList`: a comma separated list, every name trimmed, empty ones dropped; `None` when the
/// argument is absent.
fn arg_list(args: &Args, key: &str) -> Option<Vec<String>> {
    arg(args, key).map(|t| gz_nq::text::split(&t, ','))
}

// ---- the answer -----------------------------------------------------------------------------

/// The mode the module targets: the first one it brings, or the one it is limited to.
fn mode_of(m: &Manifest) -> &str {
    m.provides_modes.first().map_or(m.mode.as_str(), |p| p.id.as_str())
}

fn game_json(g: &GameWorld) -> Value {
    json!({ "available": g.ok, "levels": g.levels.len(), "transitions": g.transitions.len(), "error": g.error })
}

/// The answer of `world_get` and `world_set` (`WorldAnswer`).
pub async fn world_answer(editor: &Editor, svc: &WorldService, m: &Manifest, doc: &ManifestDoc) -> Value {
    let w = World::parse(doc);
    let k = StartKit::parse(doc);
    let (starts, problems) = svc.problems(editor, &w, &k).await;
    let game = svc.game().await;
    json!({
        "module": m.id,
        "mode": mode_of(m),
        "target": m.target,
        "world": w.json(),
        "start_kit": k.json(),
        "starts": starts_json(&starts),
        "problems": problems,
        "game": game_json(&game),
    })
}

async fn open(ed: &Editor) -> Result<Arc<WorldService>, CmdError> {
    ed.project()?;
    let svc = WorldService::of(ed).await?;
    let _ = svc.manifest().sync().await;
    Ok(svc)
}

async fn world_get(ed: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    let svc = open(&ed).await?;
    let snap = svc.manifest().snapshot();
    let (Some(doc), Some(m)) = (snap.doc(), snap.manifest()) else { return Err(refuse("mod.ltx not found in the project")) };
    Ok(world_answer(&ed, &svc, m, doc).await)
}

/// The changes of `world_set` to a world; `Err` refuses the whole call before anything is
/// written.
pub(crate) fn set_world(args: &Args, mut w: World) -> Result<World, String> {
    if let Some(preset) = arg(args, "preset") {
        match preset.as_str() {
            "stock" => w = World::default(),
            "empty" => w.keep = [false; 5],
            _ => return Err("preset must be stock (the game as it is) or empty (nothing of the original game)".into()),
        }
    }
    for (rule, keep) in RULES.iter().zip(w.keep.iter_mut()) {
        if let Some(v) = arg_bool(args, rule) {
            *keep = v;
        }
    }
    if let Some(list) = arg_list(args, "player_choice") {
        w.player_choice = [false; 5];
        for key in list {
            let Some(rule) = rule_index(&key) else {
                return Err(format!("player_choice: unknown rule '{key}' (original_npcs, random_stalkers, mutants, npc_quests, story_quests)"));
            };
            if let Some(c) = w.player_choice.get_mut(rule) {
                *c = true;
            }
        }
    }
    if w.choice_count() > MAX_PLAYER_CHOICES {
        return Err(format!("player_choice: the new-game screen has room for {MAX_PLAYER_CHOICES} rules"));
    }
    for (key, list) in [("levels", &mut w.levels), ("area", &mut w.area), ("locked_levels", &mut w.locked_levels), ("locked", &mut w.locked)] {
        if let Some(v) = arg_list(args, key) {
            *list = v;
        }
    }
    for (key, on) in [("lock", true), ("unlock", false)] {
        for n in arg_list(args, key).unwrap_or_default() {
            toggle(&mut w.locked, &n, on);
        }
    }
    for (key, on) in [("lock_level", true), ("unlock_level", false)] {
        for n in arg_list(args, key).unwrap_or_default() {
            toggle(&mut w.locked_levels, &n, on);
        }
    }
    for (key, on) in [("area_add", true), ("area_remove", false)] {
        for n in arg_list(args, key).unwrap_or_default() {
            toggle(&mut w.area, &n, on);
        }
    }
    Ok(w)
}

/// The changes of `world_set` to the start kit: any of its fields declares it,
/// `start_kit_remove` takes it out; `base` is the linked game's own kit when asked for.
pub(crate) fn set_kit(args: &Args, mut k: StartKit, base: Option<Option<StartKit>>) -> Result<StartKit, String> {
    if arg_bool(args, "start_kit_remove") == Some(true) {
        k = StartKit::default();
    }
    // the tab's Game's Base Kit: the linked game's own rows and money, the fields below on top
    if let Some(base) = base {
        let Some(mut base) = base else { return Err("start_kit_base: the linked game has no plugins\\faction_loadouts.ltx [shared] kit to read".into()) };
        base.perks = k.perks;
        k = base;
    }
    if let Some(text) = arg(args, "start_kit") {
        // "device_torch, wpn_knife, bandage=2": the whole list, like the other list fields
        k.items.clear();
        for row in gz_nq::text::split(&text, ',') {
            let (section, count) = match row.find('=') {
                Some(eq) => (gz_nq::text::trim(row.get(..eq).unwrap_or("")), crate::model::atoi(row.get(eq + 1..).unwrap_or(""))),
                None => (gz_nq::text::trim(&row), 1),
            };
            if section.is_empty() || count < 1 {
                return Err(format!("start_kit: '{row}' needs an item section and a count of at least 1 (section or section=N)"));
            }
            k.items.push(KitItem { section: section.to_owned(), count });
        }
        k.declared = true;
    }
    if let Some(money) = arg_int(args, "start_money") {
        k.money = if money < 0 { -1 } else { i32::try_from(money).unwrap_or(i32::MAX) };
        k.declared = true;
    }
    if let Some(perks) = arg_bool(args, "start_perks") {
        k.perks = perks;
        k.declared = true;
    }
    Ok(k)
}

async fn world_set(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let svc = open(&ed).await?;
    let snap = svc.manifest().snapshot();
    let Some(doc) = snap.doc() else { return Err(refuse("mod.ltx not found in the project")) };
    let before_world = World::parse(doc);
    let before_kit = StartKit::parse(doc);
    let w = set_world(&args, before_world.clone()).map_err(refuse)?;
    let base = if arg_bool(&args, "start_kit_base") == Some(true) { Some(game_base_kit(&ed).await) } else { None };
    let k = set_kit(&args, before_kit.clone(), base).map_err(refuse)?;

    // only what changed is written: the author's own spelling of an untouched section stays
    let mut edits: Vec<Edit> = Vec::new();
    if w != before_world {
        edits.push(w.edit());
    }
    if k != before_kit
        && let Some(e) = k.edit(doc)
    {
        edits.push(e);
    }
    if !edits.is_empty() {
        svc.manifest().edit(edits, origin).await.map_err(|e| if e.code == "io" { refuse("can't write mod.ltx") } else { e })?;
    }
    let snap = svc.manifest().snapshot();
    let (Some(doc), Some(m)) = (snap.doc(), snap.manifest()) else { return Err(refuse("mod.ltx not found in the project")) };
    Ok(world_answer(&ed, &svc, m, doc).await)
}

/// The linked game's own base kit (`GameBaseKit`); `None` when the game has none.
async fn game_base_kit(ed: &Editor) -> Option<StartKit> {
    let link = gz_game::GameLink::of(ed).ok()?;
    let files = link.files().await.ok()?;
    let text = ed.exec.io(move || files.read(FACTION_LOADOUTS).ok()).await?;
    base_kit(&gz_base::text::cp1251_to_utf8(&text))
}

async fn world_transitions(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let svc = open(&ed).await?;
    let game = svc.game().await;
    if !game.ok {
        return Err(refuse(format!("the linked game's transitions are not available: {}", game.error)));
    }
    let w = svc.manifest().snapshot().doc().map(World::parse).unwrap_or_default();
    let level = arg(&args, "level").unwrap_or_default();
    let captions = svc.captions(&ed).await;
    let caption = |l: &str| captions.get(l).cloned().unwrap_or_else(|| l.to_owned());

    let levels: Vec<Value> = game
        .levels
        .iter()
        .map(|l| json!({ "name": l, "caption": captions.get(l).cloned().unwrap_or_default(), "transitions": game.transitions_on(l) }))
        .collect();
    let transitions: Vec<Value> = game
        .transitions
        .iter()
        .filter(|t| level.is_empty() || t.level == level)
        .map(|t| {
            let by = w.closed_by(&t.name, &t.level, &t.to_level);
            json!({
                "name": t.name,
                "level": t.level,
                "to": t.to_level,
                "to_caption": caption(&t.to_level),
                "closed": !by.is_empty(),
                "closed_by": by,
                "position": fixed3(t.position, 2),
            })
        })
        .collect();
    let mut out = serde_json::Map::new();
    out.insert("levels".into(), Value::Array(levels));
    out.insert("transitions".into(), Value::Array(transitions));
    // the game's own patrol paths, by a part of the name: routes an order may walk, and names a
    // module path must not take
    if let Some(filter) = arg(&args, "patrols") {
        let limit = usize::try_from(arg_int(&args, "limit").unwrap_or(200)).unwrap_or(0);
        let patrols: Vec<&String> = game.patrols.iter().filter(|p| filter.is_empty() || p.contains(filter.as_str())).take(limit).collect();
        out.insert("patrols".into(), json!(patrols));
        out.insert("patrols_total".into(), json!(game.patrols.len()));
    }
    Ok(Value::Object(out))
}

// ---- the build ------------------------------------------------------------------------------

/// A coded line as a problem: its code, the rest its message.
fn as_problem(line: &str) -> Problem {
    let (code, message) = line.split_once(' ').unwrap_or((line, ""));
    if code.starts_with('E') { Problem::error(code, message) } else { Problem::warning(code, message) }
}

/// A second Player Start or a fourth new-game choice is a decision the game would make for the
/// author (the first start wins, the fourth row never shows) - refused; a level or transition the
/// game lacks is a warning.
struct WorldCheck;

impl BuildCheck for WorldCheck {
    fn stage(&self) -> &'static str {
        "Checking the world rules"
    }

    fn check(&self, editor: Arc<Editor>, _session: Arc<ProjectSession>, _walk: Arc<BuildWalk>) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            let Ok(svc) = WorldService::of(&editor).await else { return Vec::new() };
            let snap = svc.manifest().snapshot();
            let Some(doc) = snap.doc() else { return Vec::new() };
            let (w, k) = (World::parse(doc), StartKit::parse(doc));
            let (_, problems): (Vec<Start>, Vec<String>) = svc.problems(&editor, &w, &k).await;
            problems.iter().map(|p| as_problem(p)).collect()
        })
    }
}
