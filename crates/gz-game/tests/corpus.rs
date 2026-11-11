//! What the editor reads of the linked game, from the real game: its modes, its config tree with
//! the items the Spawn tool lists, its communities and ranks, its trade categories and the level
//! changers of its `all.spawn` - non-empty and holding values the game's files show.

use std::sync::Arc;
use std::time::Instant;

use gz_corpus::finish;
use gz_game::configs::load_tree;
use gz_game::{SYSTEM_LTX, modes, spawn};
use gz_vfs::{GameFiles, MountOptions};

#[test]
fn the_games_lists_come_out_with_their_known_values() {
    let Some(root) = gz_corpus::game_root() else { return };
    let started = Instant::now();
    let files = GameFiles::mount(&root, &MountOptions { cache_dir: None }).unwrap();
    let mut failed = Vec::new();
    let mut want = |ok: bool, what: &str| {
        if !ok {
            failed.push(what.to_owned());
        }
    };

    // the new-game screen's modes: the plain story is one of them
    let scan = modes::scan(&files);
    want(scan.error.is_none(), "the modes scan failed");
    want(!scan.modes.is_empty(), "no modes");
    println!("modes: {:?}", scan.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>());

    // the config tree and the sections the Spawn tool lists
    let tree = load_tree(&files, SYSTEM_LTX, None, "no system.ltx").unwrap();
    let ltx = &tree.ltx;
    want(ltx.section_count() > 5000, "a small config tree");
    let items = spawn::sections(ltx);
    for known in ["wpn_ak74", "bread", "medkit", "stalker"] {
        want(items.iter().any(|s| s.section == known), &format!("no spawn section {known}"));
    }
    want(items.iter().any(|s| s.class == "AI_STL_S"), "no stalker class among the spawn sections");

    // communities and ranks: `[game_relations]` of game_relations.ltx
    let communities = ltx.r_string("game_relations", "communities").unwrap_or_default().to_owned();
    for known in ["stalker", "bandit", "dolg", "freedom", "army"] {
        want(communities.split(',').any(|c| c.trim() == known), &format!("no community {known}"));
    }
    let ranks = ltx.r_string("game_relations", "rating").unwrap_or_default().to_owned();
    // Dead Air counts ranks to 14000 in eight names: `novice, 499, ..., master, 13999, legend`
    let parts: Vec<&str> = ranks.split(',').map(str::trim).collect();
    want(
        parts.iter().filter_map(|r| r.parse::<i64>().ok()).max() == Some(13999),
        "ranks do not end at 13999",
    );
    want(parts.iter().filter(|r| r.parse::<i64>().is_err()).count() == 8, "not eight rank names");
    want(parts.last() == Some(&"legend"), "the last rank is not legend");

    // the trade presets the Characters tab offers as categories
    let presets = files
        .read("configs/misc/trade/trade_presets.ltx")
        .map(|b| gz_base::text::cp1251_to_utf8(&b).into_owned());
    want(presets.as_ref().is_ok_and(|t| t.matches("\n[").count() > 3), "no trade presets");

    // the level changers of all.spawn: class LVL_CHNG, on the levels of the game graph
    let spawn = gz_fmt_spawn::SpawnFile::parse(Arc::from(files.read("spawns/all.spawn").unwrap())).unwrap();
    let index = gz_fmt_spawn::SpawnIndex::build(&spawn);
    let changers = index
        .entities
        .iter()
        .filter(|e| {
            ltx.r_string(&e.section.to_ascii_lowercase(), "class").is_ok_and(|c| c.trim() == "LVL_CHNG")
        })
        .count();
    println!("level changers: {changers}");
    want(changers > 20, "fewer than 20 level changers");

    finish("game lists", 1, started, &failed);
}
