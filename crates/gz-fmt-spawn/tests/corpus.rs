//! The game's `all.spawn`: read, written back to the same bytes, indexed.

use std::sync::Arc;
use std::time::Instant;

use gz_corpus::finish;
use gz_fmt_spawn::{SpawnFile, SpawnIndex};

#[test]
fn all_spawn_writes_back_the_bytes_it_read() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let bytes: Arc<[u8]> = Arc::from(game.read_path("spawns/all.spawn").unwrap());
    let mut failed = Vec::new();
    let file = SpawnFile::parse(bytes.clone()).unwrap();
    if file.to_bytes().as_slice() != &*bytes {
        failed.push("all.spawn: written back, the bytes differ".to_owned());
    }
    let index = SpawnIndex::build(&file);
    println!(
        "all.spawn: {} bytes, {} entities, {} patrols, game graph {}",
        bytes.len(),
        file.vertices.len(),
        file.patrols.len(),
        file.game_graph.is_some()
    );
    // the game's world: thousands of entities on its levels, the patrols they walk
    assert!(file.vertices.len() > 1000, "{}", file.vertices.len());
    assert!(!file.patrols.is_empty());
    assert!(file.game_graph.is_some());
    // every packet's head reads, and each level an entity stands on is a level of the graph
    assert_eq!(index.entities.iter().filter(|e| e.damaged).count(), 0);
    let names: Vec<&str> = index.levels.iter().map(|l| l.name.as_str()).collect();
    println!("levels: {names:?}");
    assert!(index.entities.iter().filter_map(|e| e.level).all(|l| index.level_name(l).is_some()));
    assert!(names.contains(&"l01_escape"));
    finish("all.spawn", 1, started, &failed);
}
