//! What the media area keeps while a project is open: the [`Index`], made once per state of the
//! files and kept fresh by the project's watcher, and what the checks read of the linked game,
//! once per game revision. The watcher also tells the scene's library which models to make
//! again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gz_base::{RelPath, Rev, RevCounter};
use gz_editor::{CmdError, Editor, Executors};
use gz_game::GameLink;
use gz_mod::ManifestService;
use gz_project::asset_kind::MediaKind;
use gz_project::{ChangeKind, EntryKind, FileTree, Filter, Project};
use gz_query::Query;
use gz_vfs::GameFiles;
use parking_lot::{Mutex, RwLock};
use rustc_hash::FxHashSet;

use crate::files::{self, FNV0, fnv};
use crate::index::{self, Index};
use crate::read::name_key;
use crate::resolve;
use crate::sound::SoundAsset;

/// What the checks read of the linked game.
#[derive(Clone, Debug, Default)]
pub struct GameLists {
    pub linked: bool,
    /// Lower case.
    pub shaders: FxHashSet<String>,
    pub compiler_shaders: FxHashSet<String>,
    pub materials: FxHashSet<String>,
    /// The game's own list of copyrighted sounds ([`crate::sound::RIGHTS_FILE`]): names and masks,
    /// keyed by [`name_key`]. Empty when the game has no such file.
    pub rights: Vec<String>,
    pub files: Option<Arc<GameFiles>>,
}

impl GameLists {
    /// Whether the game has a texture of this name.
    #[must_use]
    pub fn has_texture(&self, name: &str) -> bool {
        self.files.as_ref().is_some_and(|f| f.exists(&format!("textures\\{}.dds", name_key(name))))
    }

    /// Whether the game has a sound of this name (`sounds\<name>.ogg`).
    #[must_use]
    pub fn has_sound(&self, name: &str) -> bool {
        self.files.as_ref().is_some_and(|f| f.exists(&format!("sounds\\{}.ogg", name_key(name))))
    }
}

/// The media assets of the open project (`session.services`).
pub struct Media {
    exec: Executors,
    root: PathBuf,
    project: Arc<Project>,
    manifest: Arc<ManifestService>,
    index: Query<(), Index>,
    memo: Mutex<Option<(Rev, Rev)>>,
    latest: RwLock<Option<Arc<Index>>>,
    lists: Query<(), GameLists>,
    /// Moves with every change of a media asset or a file one may take its data from.
    changes: Arc<RevCounter>,
}

/// The extensions whose changes the index and the library hear about.
fn watched(name: &str) -> bool {
    gz_project::rules::is_media_asset_name(name)
        || files::kind_of_source(name).is_some()
        || name
            .rsplit_once('.')
            .is_some_and(|(_, e)| e.eq_ignore_ascii_case("bin") || e.eq_ignore_ascii_case("mtl"))
}

impl Media {
    /// The media of the open project, made on first use.
    pub async fn of(editor: &Arc<Editor>) -> Result<Arc<Self>, CmdError> {
        let (session, project) = Project::of(editor)?;
        if let Some(found) = session.services.get::<Self>() {
            return Ok(found);
        }
        let manifest = ManifestService::of_session(editor, &session).await?;
        let mut made = false;
        let svc = session.services.get_or_init(|| {
            made = true;
            Self {
                exec: editor.exec.clone(),
                root: PathBuf::from(project.root()),
                manifest,
                index: Query::new(editor.exec.clone()),
                memo: Mutex::new(None),
                latest: RwLock::new(None),
                lists: Query::new(editor.exec.clone()),
                changes: Arc::new(RevCounter::new()),
                project,
            }
        });
        if made {
            follow(editor, &svc);
        }
        Ok(svc)
    }

    /// The media of the open project when they were made already: what a reader that cannot
    /// wait asks (the viewport's loader).
    #[must_use]
    pub fn get(editor: &Editor) -> Option<Arc<Self>> {
        editor.project_opt()?.services.get::<Self>()
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn rules(&self) -> Vec<String> {
        self.manifest.snapshot().manifest().map(|m| m.exclude.clone()).unwrap_or_default()
    }

    fn module(&self) -> String {
        self.manifest.snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default()
    }

    fn files(tree: &FileTree) -> Vec<RelPath> {
        let mut out = Vec::new();
        for kind in MediaKind::ALL {
            out.extend(tree.files_with_extension(kind.extension().trim_start_matches('.')));
        }
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out
    }

    fn stamp_of(&self, tree: &FileTree, files: &[RelPath], rules: &[String], module: &str) -> Rev {
        let mut h = fnv(FNV0, module.as_bytes());
        for r in rules {
            h = fnv(fnv(h, r.as_bytes()), b"\n");
        }
        let live = self.project.tree.changes().live();
        if live {
            h = fnv(h, &self.changes.get().0.to_le_bytes());
        }
        for rel in files {
            h = fnv(h, rel.key().as_bytes());
            if live {
                continue;
            }
            if let Some(EntryKind::File { size, modified, .. }) = tree.entry(rel).map(|e| e.kind) {
                h = fnv(h, &size.to_le_bytes());
                let t = modified
                    .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                h = fnv(h, &t.to_le_bytes());
            }
        }
        Rev(h)
    }

    /// The stamp of the files now, worked out once per state of the tree and the manifest: what
    /// a cache of something made from the media assets is keyed by.
    pub fn current_stamp(&self) -> Rev {
        let tree = self.project.tree.snapshot();
        let cheap = tree.rev().mix(self.manifest.rev()).mix(self.changes.get());
        if let Some((key, stamp)) = *self.memo.lock()
            && key == cheap
        {
            return stamp;
        }
        let stamp = self.stamp_of(&tree, &Self::files(&tree), &self.rules(), &self.module());
        *self.memo.lock() = Some((cheap, stamp));
        stamp
    }

    /// The index of the files as the tree has them now, made once per state of them on the IO
    /// pool.
    pub async fn index(&self) -> Arc<Index> {
        let stamp = self.current_stamp();
        if let Some(i) = self.index.peek_at(&(), stamp) {
            return i;
        }
        let tree = self.project.tree.snapshot();
        let rules = self.rules();
        let (root, module, exec) = (self.root.clone(), self.module(), self.exec.clone());
        // the query's own computation: however many ask at once - a check of a folder asks once
        // per file - the files are walked and read once
        let index = self
            .index
            .fresh(&(), stamp, move || {
                let files: Vec<(RelPath, bool)> = Self::files(&tree)
                    .into_iter()
                    .map(|r| (r.clone(), gz_mod::outside_build(&rules, r.as_str())))
                    .collect();
                exec.io_pool().install(|| index::build(&root, files, module, stamp))
            })
            .await;
        *self.latest.write() = Some(index.clone());
        index
    }

    /// The index once the file tree holds every change so far: what a command reads after a
    /// file was written.
    pub async fn fresh_index(&self) -> Arc<Index> {
        self.project.tree.refresh().await;
        self.index().await
    }

    /// The newest index made, without waiting.
    #[must_use]
    pub fn latest(&self) -> Option<Arc<Index>> {
        self.latest.read().clone()
    }

    /// The picture of the project's texture asset that publishes `name`, and its extension:
    /// what a loader reads before its old sources. From the newest index; blocking (a look at
    /// the asset's file).
    #[must_use]
    pub fn texture_file(&self, name: &str) -> Option<(PathBuf, String)> {
        let index = self.latest()?;
        let e = index.by_name(MediaKind::Texture, name)?;
        let source = e.source.as_ref()?.to_fs(&self.root);
        let ext = source.extension()?.to_str()?.to_ascii_lowercase();
        Some((source, ext))
    }

    /// The project's texture asset that publishes `name`: its picture decoded and its
    /// description. Blocking.
    #[must_use]
    pub fn texture(&self, name: &str) -> Option<resolve::Texture> {
        let index = self.latest()?;
        let e = index.by_name(MediaKind::Texture, name)?;
        resolve::texture(&self.root, e.path.as_str(), &e.name).ok()
    }

    /// The project's sound asset that publishes `name`: its description and its source file.
    /// Blocking.
    #[must_use]
    pub fn sound(&self, name: &str) -> Option<(SoundAsset, PathBuf)> {
        let index = self.latest()?;
        let e = index.by_name(MediaKind::Sound, name)?;
        resolve::sound(&self.root, e.path.as_str()).ok().map(|(a, p, _)| (a, p))
    }

    /// What the checks read of the linked game now (nothing without one).
    pub async fn game_lists(&self, editor: &Editor) -> Arc<GameLists> {
        let Ok(link) = GameLink::of(editor) else { return Arc::new(GameLists::default()) };
        let Ok((files, rev)) = link.snapshot().await else { return Arc::new(GameLists::default()) };
        if let Some(l) = self.lists.peek_at(&(), rev) {
            return l;
        }
        let f = files.clone();
        let read = self
            .exec
            .io(move || {
                ["shaders.xr", "shaders_xrlc.xr", "gamemtl.xr", crate::sound::RIGHTS_FILE]
                    .map(|n| f.read(n).ok())
            })
            .await;
        self.lists
            .fresh(&(), rev, move || {
                let lower =
                    |v: Vec<String>| v.into_iter().map(|s| s.to_lowercase()).collect::<FxHashSet<_>>();
                let [sh, xrlc, mtl, rights] = read;
                GameLists {
                    linked: true,
                    shaders: lower(
                        sh.and_then(|b| gz_fmt_xr::Shaders::read(&b).ok())
                            .map(|s| s.blenders.iter().map(|b| b.name.to_string()).collect())
                            .unwrap_or_default(),
                    ),
                    compiler_shaders: lower(
                        xrlc.and_then(|b| gz_fmt_xr::ShadersXrlc::read(&b).ok())
                            .map(|s| s.shaders.iter().map(gz_fmt_xr::ShaderXrlc::name).collect())
                            .unwrap_or_default(),
                    ),
                    materials: lower(
                        mtl.and_then(|b| gz_fmt_xr::GameMaterials::read(&b).ok())
                            .map(|m| m.materials.iter().map(|x| x.name.to_string()).collect())
                            .unwrap_or_default(),
                    ),
                    // the game's own text, cp1251 where it is not ASCII
                    rights: rights
                        .map(|b| crate::sound::parse_rights(&gz_base::text::cp1251_to_utf8(&b)))
                        .unwrap_or_default(),
                    files: Some(files),
                }
            })
            .await
    }
}

/// The project's watcher moves the index and tells the scene's library which models to make
/// again: an asset that changed, every model whose source or a file it uses changed, every
/// model when a texture asset changed (a surface may take its texture), all of them after a
/// rescan.
fn follow(editor: &Arc<Editor>, svc: &Arc<Media>) {
    let mut rx = svc.project.tree.changes().subscribe(Filter::All);
    let (me, ed) = (Arc::downgrade(svc), Arc::downgrade(editor));
    editor.exec.spawn(async move {
        while let Some(change) = rx.recv().await {
            let Some(me) = me.upgrade() else { break };
            let name = change.path.file_name().to_owned();
            let rescan = change.kind == ChangeKind::Rescan;
            if !rescan && !watched(&name) && change.kind != ChangeKind::Removed {
                continue;
            }
            me.changes.bump();
            let latest = me.latest();
            let mut forget: Vec<String> = Vec::new();
            if let Some(index) = &latest {
                let models = index.of_kind(MediaKind::Model);
                if rescan || MediaKind::of_file(&name) == Some(MediaKind::Texture) {
                    forget.extend(models.map(|e| e.path.as_str().to_owned()));
                } else {
                    forget.extend(
                        index
                            .users_of(change.path.as_str())
                            .iter()
                            .filter(|e| e.kind == MediaKind::Model)
                            .map(|e| e.path.as_str().to_owned()),
                    );
                }
            }
            if MediaKind::of_file(&name) == Some(MediaKind::Model) {
                forget.push(change.path.as_str().to_owned());
            }
            if forget.is_empty() {
                continue;
            }
            let Some(ed) = ed.upgrade() else { break };
            if let Some(scene) = ed.project_opt().and_then(|s| s.services.get::<gz_scene::SceneService>()) {
                for r in forget {
                    scene.library.forget(&r);
                }
            }
        }
    });
}

/// The provider of a project's model assets for the scene's library: a reference ending in
/// `.meshasset` is a project path, made into its model when it is first asked for.
#[must_use]
pub fn provider(root: PathBuf) -> gz_scene::library::Provider {
    Arc::new(move |key: &str| {
        if !key.ends_with(MediaKind::Model.extension()) {
            return None;
        }
        resolve::library_object(&root, key).ok()
    })
}
