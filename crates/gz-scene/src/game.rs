//! What the scene takes from the linked game: the configs spawn entities are made and decoded
//! with, and the Spawn tool's list. The one place this crate meets `gz-game`.

use std::borrow::Cow;
use std::sync::Arc;

use gz_editor::Editor;
use gz_entities::Settings;
use gz_game::{Bundled, GameLink, SpawnSection};
use gz_ltx::Ltx;
use gz_vfs::GameFiles;

/// The game's settings over the linked game's `system.ltx` tree (or the editor's own copy).
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

    /// Every line of a section in its order (`r_line`), a line without a value as `(key, "")`.
    #[must_use]
    pub fn lines(&self, section: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Ok(Some((k, v))) = self.ltx.r_line(section, i) {
            out.push((k.to_owned(), v.unwrap_or("").to_owned()));
            i += 1;
        }
        out
    }

    /// The Details sheet's lists that come from the configs: the story ids, and the graph
    /// points' locations of `game.ltx`. Blocking: reads `game.ltx`.
    #[must_use]
    pub fn prop_context(&self) -> gz_entities::PropContext {
        let game = self.game_ltx();
        let lines = |i: usize| {
            let name = format!("location_{i}");
            let mut out = Vec::new();
            if let Some(ltx) = &game {
                let mut k = 0;
                while let Ok(Some((key, value))) = ltx.r_line(&name, k) {
                    out.push((key.to_owned(), value.unwrap_or("").to_owned()));
                    k += 1;
                }
            }
            out
        };
        gz_entities::PropContext::default()
            .with_story_names(self.lines("story_ids"), self.lines("spawn_story_ids"))
            .with_locations([lines(0), lines(1), lines(2), lines(3)])
    }

    /// `$game_config$\game.ltx` and what it includes.
    fn game_ltx(&self) -> Option<Ltx> {
        use gz_ltx::tree::{Dialect, ParsedFile, include_path, parse_file};
        let mut files: rustc_hash::FxHashMap<String, ParsedFile> = rustc_hash::FxHashMap::default();
        let mut queue = vec!["game.ltx".to_owned()];
        while let Some(path) = queue.pop() {
            if files.contains_key(&path) || files.len() > 256 {
                continue;
            }
            let Some(text) = self.config_text(&path) else { continue };
            let parsed = parse_file(&text, Dialect::Editor);
            queue.extend(parsed.includes().iter().map(|inc| include_path(&path, &inc.name)));
            files.insert(path, parsed);
        }
        if !files.contains_key("game.ltx") {
            return None;
        }
        Ltx::assemble_map("game.ltx", &files, Dialect::Editor).ok().map(|l| l.ltx)
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

/// The Spawn tool's list (`game.lookup type=spawn_section`).
pub async fn spawn_sections(editor: &Editor) -> Arc<[SpawnSection]> {
    gz_game::spawn_roster(editor).await
}

/// What the radio rows, the audit and the bake read, current: the game's radios read again when
/// the configs changed, the module's playlists from its manifest.
pub async fn radio_cx(editor: &Arc<Editor>) -> Arc<crate::objects::radio::RadioCx> {
    let Ok(session) = editor.project() else { return Arc::new(crate::objects::radio::RadioCx::unlinked()) };
    let svc = crate::SceneService::of_session(editor, &session);
    svc.refresh_radios(editor, settings(editor).await).await;
    // made on first use: the playlists are its answer
    let _ = gz_mod::ManifestService::of_session(editor, &session).await;
    svc.radio_cx_now(&session)
}
