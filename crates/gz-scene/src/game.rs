//! What the scene takes from the linked game: the configs spawn entities are made and decoded
//! with (`pSettings`), and the Spawn tool's list. The one place this crate meets `gz-game`.

use std::borrow::Cow;
use std::sync::Arc;

use gz_editor::Editor;
use gz_entities::Settings;
use gz_game::{Bundled, GameLink, SpawnSection};
use gz_ltx::Ltx;
use gz_vfs::GameFiles;

/// `pSettings` over the linked game's `system.ltx` tree (or the editor's own copy).
pub struct GameSettings {
    ltx: Arc<Ltx>,
    files: Option<Arc<GameFiles>>,
}

impl GameSettings {
    #[must_use]
    pub fn new(ltx: Arc<Ltx>, files: Option<Arc<GameFiles>>) -> Self {
        Self { ltx, files }
    }

    /// The tree itself.
    #[must_use]
    pub fn ltx(&self) -> &Ltx {
        &self.ltx
    }
}

impl Settings for GameSettings {
    fn section_exist(&self, section: &str) -> bool {
        self.ltx.section_exist(&section.to_ascii_lowercase())
    }

    fn r_string(&self, section: &str, key: &str) -> Option<&str> {
        let s = section.to_ascii_lowercase();
        if !self.ltx.line_exist(&s, key) {
            return None;
        }
        Some(self.ltx.r_string_raw(&s, key).ok().flatten().unwrap_or(""))
    }

    /// A file under `$game_config$`. Blocking: entities are made on the IO pool.
    fn config_text(&self, path: &str) -> Option<Cow<'_, str>> {
        let files = self.files.as_ref()?;
        let bytes = files.read(&format!("configs\\{path}")).ok()?;
        Some(Cow::Owned(gz_base::text::decode_guess(&bytes).into_owned()))
    }
}

/// The configs of the linked game, or the editor's own when no game is linked; `None` when
/// neither loads.
pub async fn settings(editor: &Editor) -> Option<Arc<GameSettings>> {
    if let Ok(link) = GameLink::of(editor)
        && let Ok(ltx) = link.configs().await
    {
        let files = link.files().await.ok();
        return Some(Arc::new(GameSettings::new(ltx, files)));
    }
    match Bundled::of(editor).config_tree().await {
        Ok(tree) => Some(Arc::new(GameSettings::new(tree.ltx.clone(), None))),
        Err(_) => None,
    }
}

/// The Spawn tool's list (`list_spawn_sections`).
pub async fn spawn_sections(editor: &Editor) -> Arc<[SpawnSection]> {
    gz_game::spawn_roster(editor).await
}
