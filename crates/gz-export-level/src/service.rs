//! What the export area reads of the linked game: its patrol names, from `gz-world`'s reading of
//! `all.spawn` (read once per game revision there; the game is only ever read).

use std::collections::HashSet;
use std::sync::Arc;

use gz_editor::Editor;
use gz_world::WorldService;

/// The export area's view of the linked game.
pub struct ExportService;

impl ExportService {
    /// The patrol names of the linked game's
    /// `spawns\all.spawn`. Empty without a game or a readable spawn file.
    pub async fn game_patrols(editor: &Editor) -> Arc<HashSet<String>> {
        match WorldService::of(editor).await {
            Ok(svc) => Arc::new(svc.game().await.patrols.iter().cloned().collect()),
            Err(_) => Arc::default(),
        }
    }
}
