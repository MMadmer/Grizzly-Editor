//! What the game area keeps while the editor runs.
//!
//! - [`GameLink`] (project scope, in `session.services`): the open project's linked game - the
//!   index `gz-project` mounts, and everything derived from it as [`Query`]s keyed by the
//!   index's revision, computed on first use.
//! - [`Bundled`] (editor scope): the copies of game files the editor ships under
//!   `<editor root>\gamedata`, and the tree its own `system.ltx` makes - what the editor reads
//!   when no game is linked.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::Rev;
use gz_editor::{Editor, Executors};
use gz_fmt_xml::sheets::TextureSheets;
use gz_fmt_xml::strings::{self, Mode, StringTable};
use gz_ltx::Ltx;
use gz_project::Project;
use gz_query::Query;
use gz_vfs::GameFiles;

use crate::configs::{self, ConfigTree, DiskFolder, SYSTEM_LTX};
use crate::modes::{self, ModeScan};
use crate::spawn::{self, SpawnSection};
use crate::version;

/// Why the game cannot answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GameError {
    #[error("no project is open")]
    NoProject,
    /// No game is linked, or its folder is no game any more (the reason).
    #[error("{0}")]
    NotLinked(String),
    /// The game's files do not make what was asked for (the reason).
    #[error("{0}")]
    Read(String),
    /// Neither the linked game nor the editor's bundled copy holds the file.
    #[error("neither the linked game nor the editor has {0}")]
    Missing(String),
}

/// Where a file that may come from either place came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// The linked game.
    Game,
    /// The editor's bundled copy.
    Bundled,
}

const NO_SYSTEM_LTX: &str = "the linked game has no configs\\system.ltx";
const NO_CONFIGS: &str = "the linked game has no configs";

type Derived<V> = Query<(), Result<Arc<V>, String>>;

/// The open project's linked game.
pub struct GameLink {
    exec: Executors,
    project: Arc<Project>,
    configs: Derived<ConfigTree>,
    strings: Query<(String, bool), Result<Arc<StringTable>, String>>,
    language: Query<(), String>,
    sheets: Derived<TextureSheets>,
    modes: Query<(), ModeScan>,
    spawn: Derived<[SpawnSection]>,
    version: Query<(), Option<String>>,
}

impl GameLink {
    /// The game of the open project, made on first use.
    pub fn of(editor: &Editor) -> Result<Arc<GameLink>, GameError> {
        let (session, project) = Project::of(editor).map_err(|_| GameError::NoProject)?;
        let exec = editor.exec.clone();
        Ok(session.services.get_or_init(|| GameLink {
            configs: Query::new(exec.clone()),
            strings: Query::new(exec.clone()),
            language: Query::new(exec.clone()),
            sheets: Query::new(exec.clone()),
            modes: Query::new(exec.clone()),
            spawn: Query::new(exec.clone()),
            version: Query::new(exec.clone()),
            exec,
            project,
        }))
    }

    /// The linked game's folder as `project.ltx` names it (`""`: none).
    #[must_use]
    pub fn root(&self) -> String {
        self.project.game_root()
    }

    /// The revision of the game index in place: it moves when the link changes or a refresh is
    /// asked, and every derived value is keyed by it.
    #[must_use]
    pub fn rev(&self) -> Rev {
        self.project.game_rev()
    }

    /// The index and its revision, mounted on first need.
    pub async fn snapshot(&self) -> Result<(Arc<GameFiles>, Rev), GameError> {
        let p = self.project.clone();
        self.exec.io(move || p.game_snapshot()).await.map_err(GameError::NotLinked)
    }

    /// The linked game's files.
    pub async fn files(&self) -> Result<Arc<GameFiles>, GameError> {
        self.snapshot().await.map(|(files, _)| files)
    }

    /// Checks the game against the disk again; what is derived from it is computed anew.
    pub async fn refresh(&self) -> Result<Arc<GameFiles>, GameError> {
        let p = self.project.clone();
        self.exec.io(move || p.refresh_game()).await.map(|(files, _)| files).map_err(GameError::NotLinked)
    }

    /// A derived value of the index: computed on the CPU pool (reads through the IO pool) once
    /// per revision.
    async fn derive<K, V>(
        &self,
        query: &Query<K, Result<Arc<V>, String>>,
        key: K,
        compute: impl FnOnce(&GameFiles, &Executors) -> Result<Arc<V>, String> + Send + 'static,
    ) -> Result<Arc<V>, GameError>
    where
        K: Eq + std::hash::Hash + Clone + Send + 'static,
        V: Send + Sync + 'static + ?Sized,
    {
        let (files, rev) = self.snapshot().await?;
        let exec = self.exec.clone();
        let value = query.fresh(&key, rev, move || compute(&files, &exec)).await;
        value.as_ref().clone().map_err(GameError::Read)
    }

    /// The game's `system.ltx` tree, as the editor's `CInifile` reads it, with its warnings.
    pub async fn config_tree(&self) -> Result<Arc<ConfigTree>, GameError> {
        self.derive(&self.configs, (), |files, exec| {
            if files.find("configs", "").is_empty() {
                return Err(NO_CONFIGS.to_owned());
            }
            configs::load_tree(files, SYSTEM_LTX, Some(exec.io_pool()), NO_SYSTEM_LTX).map(Arc::new)
        })
        .await
    }

    /// The game's `system.ltx` tree.
    pub async fn configs(&self) -> Result<Arc<Ltx>, GameError> {
        self.config_tree().await.map(|t| t.ltx.clone())
    }

    /// The language of `configs\localization.ltx` (`rus` without one).
    pub async fn language(&self) -> Result<String, GameError> {
        let (files, rev) = self.snapshot().await?;
        let value = self
            .language
            .fresh(&(), rev, move || {
                let text = files.read("configs/localization.ltx").map(|b| gz_base::text::cp1251_to_utf8(&b).into_owned()).unwrap_or_default();
                strings::language(&text)
            })
            .await;
        Ok(value.as_ref().clone())
    }

    /// The string table of `lang` as the editor's pickers read it ([`Mode::Editor`]).
    pub async fn strings(&self, lang: &str) -> Result<Arc<StringTable>, GameError> {
        self.strings_as(lang, Mode::Editor).await
    }

    /// The string table of `lang` read the editor's way or the game's.
    pub async fn strings_as(&self, lang: &str, mode: Mode) -> Result<Arc<StringTable>, GameError> {
        let lang_owned = lang.to_owned();
        self.derive(&self.strings, (lang.to_owned(), mode == Mode::Game), move |files, exec| {
            let mut names: Vec<String> = files
                .find(&format!("configs/text/{lang_owned}"), "xml")
                .into_iter()
                .map(|f| f.path.replace('/', "\\"))
                .filter(|p| strings::is_table_file(p, &lang_owned))
                .collect();
            strings::order(&mut names);
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let bytes = exec.io_pool().install(|| files.read_many(&refs));
            let texts: Vec<(String, String)> =
                names.iter().zip(bytes).filter_map(|(n, b)| b.ok().map(|b| (n.clone(), gz_fmt_xml::decode(&b).into_owned()))).collect();
            let pairs: Vec<(&str, &str)> = texts.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            Ok(Arc::new(StringTable::build(&pairs, mode).0))
        })
        .await
    }

    /// The UI texture descriptions (`configs\ui\textures_descr\*.xml`): `id -> (sheet, rect)`.
    pub async fn texture_sheets(&self) -> Result<Arc<TextureSheets>, GameError> {
        self.derive(&self.sheets, (), |files, exec| {
            let paths: Vec<&str> = files.find("configs/ui/textures_descr", "xml").into_iter().map(|f| f.path).collect();
            let bytes = exec.io_pool().install(|| files.read_many(&paths));
            let texts: Vec<String> = bytes.into_iter().filter_map(Result::ok).map(|b| gz_fmt_xml::decode(&b).into_owned()).collect();
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            Ok(Arc::new(TextureSheets::build(&refs)))
        })
        .await
    }

    /// The game modes a module can target (`EditorGameModes::Scan`). Not linked: no modes and
    /// the reason.
    pub async fn modes(&self) -> Arc<ModeScan> {
        let (files, rev) = match self.snapshot().await {
            Ok(s) => s,
            Err(e) => return Arc::new(ModeScan { modes: Vec::new(), error: Some(e.to_string()) }),
        };
        let exec = self.exec.clone();
        self.modes.fresh(&(), rev, move || exec.io_pool().install(|| modes::scan(&files))).await
    }

    /// The spawnable sections, grouped as the Spawn tool lists them.
    pub async fn spawn_sections(&self) -> Result<Arc<[SpawnSection]>, GameError> {
        let tree = self.config_tree().await?;
        let (_, rev) = self.snapshot().await?;
        let value = self.spawn.fresh(&(), rev, move || Ok(Arc::from(spawn::sections(&tree.ltx)))).await;
        value.as_ref().clone().map_err(GameError::Read)
    }

    /// The version of the linked game's `xrEngine.exe` (`EditorMod::LinkedGameVersion`).
    pub async fn game_version(&self) -> Option<String> {
        let root = self.root();
        if root.is_empty() {
            return None;
        }
        let exec = self.exec.clone();
        let value = self
            .version
            .fresh(&(), self.rev(), move || {
                let exe = std::path::Path::new(&root).join("xrEngine.exe");
                exec.io_pool().install(|| std::fs::read(exe).ok()).and_then(|bytes| version::exe_version(&bytes))
            })
            .await;
        value.as_ref().clone()
    }
}

/// The game files the editor ships, and the tree of its own `system.ltx`.
pub struct Bundled {
    exec: Executors,
    gamedata: PathBuf,
    configs: Derived<ConfigTree>,
    spawn: Derived<[SpawnSection]>,
}

impl Bundled {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Bundled> {
        editor.services.get_or_init(|| Bundled {
            exec: editor.exec.clone(),
            gamedata: editor.paths.root.join("gamedata"),
            configs: Query::new(editor.exec.clone()),
            spawn: Query::new(editor.exec.clone()),
        })
    }

    /// Where the bundled copy of a game path is (`configs\nq\catalog.ltx` ->
    /// `<editor root>\gamedata\configs\nq\catalog.ltx`).
    #[must_use]
    pub fn path(&self, rel: &str) -> PathBuf {
        self.gamedata.join(gz_vfs::normalize(rel))
    }

    /// The bundled copy of a game path, when the editor has one.
    pub async fn read(&self, rel: &str) -> Option<Vec<u8>> {
        let path = self.path(rel);
        self.exec.io(move || std::fs::read(path).ok()).await
    }

    /// The editor's own `system.ltx` tree: the configs the editor reads without a linked game.
    pub async fn config_tree(&self) -> Result<Arc<ConfigTree>, GameError> {
        let source = DiskFolder(self.gamedata.clone());
        let exec = self.exec.clone();
        let value = self
            .configs
            .fresh(&(), Rev::ZERO, move || {
                configs::load_tree(&source, SYSTEM_LTX, Some(exec.io_pool()), "the editor has no gamedata\\configs\\system.ltx").map(Arc::new)
            })
            .await;
        value.as_ref().clone().map_err(GameError::Read)
    }

    /// The spawnable sections of the editor's own configs.
    pub async fn spawn_sections(&self) -> Result<Arc<[SpawnSection]>, GameError> {
        let tree = self.config_tree().await?;
        let value = self.spawn.fresh(&(), Rev::ZERO, move || Ok(Arc::from(spawn::sections(&tree.ltx)))).await;
        value.as_ref().clone().map_err(GameError::Read)
    }
}

/// A game file from the linked game, or - when no game is linked or it lacks the file - from the
/// editor's bundled copy (the NQ catalog, the PDA reader).
pub async fn read_game_or_bundled(editor: &Editor, rel: &str) -> Result<(Vec<u8>, Provenance), GameError> {
    if let Ok(link) = GameLink::of(editor)
        && let Ok(files) = link.files().await
        && files.exists(rel)
    {
        let key = rel.to_owned();
        if let Ok(bytes) = editor.exec.io(move || files.read(&key)).await {
            return Ok((bytes, Provenance::Game));
        }
    }
    match Bundled::of(editor).read(rel).await {
        Some(bytes) => Ok((bytes, Provenance::Bundled)),
        None => Err(GameError::Missing(rel.replace('/', "\\"))),
    }
}

/// The spawn roster of the linked game, or the editor's own when no game is linked or its
/// configs do not load - the editor's `pSettings` stayed its own then.
pub async fn spawn_roster(editor: &Editor) -> Arc<[SpawnSection]> {
    if let Ok(link) = GameLink::of(editor) {
        match link.spawn_sections().await {
            Ok(s) => return s,
            Err(GameError::NotLinked(_)) => {}
            Err(e) => editor.log.warn(format!("the linked game's configs did not load: {e}")),
        }
    }
    match Bundled::of(editor).spawn_sections().await {
        Ok(s) => s,
        Err(e) => {
            editor.log.warn(format!("the editor's own configs did not load: {e}"));
            Arc::from(Vec::new())
        }
    }
}
