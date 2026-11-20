//! The media assets in the module build (`docs/ASSETS.md` section 6): what `gz-build` asks of
//! them ([`MediaBuild`], set by [`crate::install`]). Every asset that ships - a texture, a sound,
//! a dynamic model - is baked through `gz_media::publish` into the project's `temp\media` and
//! handed to the build under the game's place for its name; the asset where it lies and every
//! file it is made of are sources, which the build never ships as they are - but a source that is
//! the game's container already, which is handed on itself (`gz_media::publish`). A release stages
//! its module through the same build, so it ships the same files.

use std::path::Path;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::RelPath;
use gz_build::{BuildFile, FileFrom, MediaPlan, MediaPublisher, Publication, Sources, source_key};
use gz_editor::{Editor, ProjectSession};
use gz_game::version::{LINKED_GAME, game_states, game_version, words};
use gz_media::{Index, Media, Payload};

/// The publisher the build asks.
pub struct MediaBuild;

/// The files the assets of `index` are made of, as the build's walk leaves them out
/// ([`gz_media::Entry::takes`]): a file of the module's `gamedata` only when its asset
/// publishes it at that very path - the game reads it where it lies, by its own name, and an
/// asset that publishes another name must not take it out of the module.
#[must_use]
pub fn sources_of(index: &Index) -> Sources {
    let mut out = Sources::default();
    for e in &index.entries {
        out.extend(
            e.source
                .iter()
                .chain(&e.uses)
                .chain(&e.companion)
                .filter(|r| e.takes(r.as_str()))
                .map(|r| source_key(r.as_str())),
        );
    }
    out
}

/// What `index` would publish, without a bake.
#[must_use]
pub fn plan_of(index: &Index) -> MediaPlan {
    MediaPlan { sources: sources_of(index), published: index.published_paths() }
}

/// The sentence the build says when the linked game does not read the descriptions: the data
/// still loads, what the descriptions say over it does not.
#[must_use]
pub fn no_descriptions_warning(version: Option<&str>) -> String {
    format!(
        "The linked game{} does not state {}: it loads the published textures, sounds and models, but not what their descriptions say over them (bump, detail and material of a texture, a sound's distances and type, a model's surfaces, bones and motions).",
        version.map(|v| format!(" ({v})")).unwrap_or_default(),
        words::ASSET_DESCRIPTIONS
    )
}

/// Bakes and publishes every asset of `index` that ships; `game` is the linked game's folder,
/// asked whether it reads the descriptions. Blocking: run it on the IO pool.
#[must_use]
pub fn publication(
    root: &Path,
    index: &Index,
    game: Option<&Path>,
    models: &gz_media::resolve::GameModel,
) -> Publication {
    let mut out = Publication { sources: sources_of(index), ..Publication::default() };
    // an asset that does not read would silently not ship: the build stops on it
    for e in index.entries.iter().filter(|e| !e.excluded) {
        if let Some(why) = &e.error {
            out.errors.push(format!("{}: {why}", e.path));
        }
    }
    let mut seen = rustc_hash::FxHashMap::<String, String>::default();
    for e in index.entries.iter().filter(|e| e.ships()) {
        for p in gz_media::publish::paths_of(e) {
            if let Some(first) = seen.insert(p.to_lowercase(), e.path.to_string()) {
                out.errors
                    .push(format!("{} and {first} both publish {p}: give one of them another name", e.path));
            }
        }
    }
    if !out.errors.is_empty() {
        return out;
    }
    let mut files: Vec<BuildFile> = Vec::new();
    let mut sink = |rel: &str, payload: Payload| -> Result<(), String> {
        let at = RelPath::new(&rel.replace('\\', "/")).map_err(|e| e.to_string())?;
        let file = match payload {
            Payload::File(path) => {
                let md = std::fs::metadata(&path)
                    .map_err(|e| format!("{}: {}", path.display(), gz_base::io_reason(&e)))?;
                BuildFile {
                    rel: at,
                    size: md.len(),
                    modified: md.modified().ok(),
                    from: FileFrom::Made(path),
                }
            }
            Payload::Bytes(bytes) => BuildFile {
                rel: at,
                size: bytes.len() as u64,
                modified: None,
                from: FileFrom::Bytes(Arc::from(bytes)),
            },
        };
        files.push(file);
        Ok(())
    };
    let done = gz_media::publish(root, index, models, &mut sink);
    out.errors.extend(done.problems);
    out.files = files;
    if let Some(game) = game
        && !out.files.is_empty()
        && !game_states(game, &LINKED_GAME, words::ASSET_DESCRIPTIONS)
    {
        out.warnings.push(no_descriptions_warning(game_version(game, &LINKED_GAME).as_deref()));
    }
    out
}

/// The media of the open project with its index as the files are now; `None` without a project.
async fn fresh(editor: &Arc<Editor>) -> Option<Arc<Index>> {
    let media = Media::of(editor).await.ok()?;
    Some(media.fresh_index().await)
}

impl MediaPublisher for MediaBuild {
    fn plan(&self, editor: Arc<Editor>, _session: Arc<ProjectSession>) -> BoxFuture<'static, MediaPlan> {
        Box::pin(async move { fresh(&editor).await.map(|i| plan_of(&i)).unwrap_or_default() })
    }

    fn publish(&self, editor: Arc<Editor>, session: Arc<ProjectSession>) -> BoxFuture<'static, Publication> {
        Box::pin(async move {
            let Some(index) = fresh(&editor).await else { return Publication::default() };
            let game = gz_project::Project::of(&editor).map(|(_, p)| p.game_root()).unwrap_or_default();
            let root = session.root.clone();
            let models = gz_media::GameModels::reader(&editor, &root);
            editor
                .exec
                .io(move || {
                    let game = (!game.trim().is_empty()).then(|| std::path::PathBuf::from(&game));
                    publication(&root, &index, game.as_deref(), &models)
                })
                .await
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use gz_build::install::install_build;
    use gz_build::{BuildWalk, WalkScope};
    use gz_project::FileTree;

    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("gz-release-media-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }

        fn put(&self, rel: &str, bytes: impl AsRef<[u8]>) {
            let p = self.0.join(rel.replace('/', "\\"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, bytes).unwrap();
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// An uncompressed 32-bit TGA, top-left first.
    pub(crate) fn tga(w: u16, h: u16, bgra: [u8; 4]) -> Vec<u8> {
        let mut out = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&[32, 0x28]);
        for _ in 0..u32::from(w) * u32::from(h) {
            out.extend_from_slice(&bgra);
        }
        out
    }

    /// A tenth of a second of a tone: RIFF, PCM, 16 bits, mono, 22050 Hz.
    fn wav() -> Vec<u8> {
        let samples: Vec<i16> = (0..2205).map(|i| ((i as f32 * 0.1).sin() * 8000.0) as i16).collect();
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&22050u32.to_le_bytes());
        out.extend_from_slice(&(22050u32 * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    const CUBE_OBJ: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
        f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n";

    fn project(d: &Dir) {
        d.put("mod.ltx", "[module]\nid = od\n[exclude]\nwip\n");
        d.put("art/wall.tga", tga(4, 4, [30, 60, 120, 255]));
        d.put("art/wall.texasset", "return { tex = 1 }\n");
        d.put("sfx/hum.wav", wav());
        d.put("sfx/hum.sndasset", "return { snd = 1, max_distance = 30 }\n");
        d.put("models/crate.obj", CUBE_OBJ);
        d.put("models/crate.meshasset", "return { mesh = 1, kind = \"dynamic\" }\n");
        d.put("models/wall.meshasset", "return { mesh = 1, parts = { { shape = \"box\" } } }\n");
        d.put("wip/old.tga", tga(2, 2, [0, 0, 0, 255]));
        d.put("wip/old.texasset", "return { tex = 1 }\n");
        d.put("gamedata/configs/od.ltx", "[od]\n");
    }

    fn noop(_: gz_build::install::Feed<'_>) {}

    #[test]
    fn a_build_publishes_the_media_and_ships_none_of_the_sources() {
        let d = Dir::new("build");
        project(&d);
        let index = gz_media::scan(&d.0);
        let p = publication(&d.0, &index, None, &gz_media::resolve::no_game());
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert!(p.warnings.is_empty());
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 2 })
                .unwrap();
        let exec = rt.exec();
        let tree = FileTree::scan(&d.0);
        let rules = vec!["wip".to_owned()];
        let mut walk = BuildWalk::scan_with(&tree, &rules, WalkScope::Module, &exec, &p.sources);
        assert!(walk.publish(p.files).is_empty());
        let mut files: Vec<&str> = walk.files.iter().map(|f| f.rel.as_str()).collect();
        files.sort_unstable();
        assert_eq!(
            files,
            [
                "gamedata/configs/od.ltx",
                "gamedata/meshes/od/crate.meshasset",
                "gamedata/meshes/od/crate.ogf",
                "gamedata/sounds/od/hum.ogg",
                "gamedata/sounds/od/hum.sndasset",
                "gamedata/textures/od/wall.dds",
                "gamedata/textures/od/wall.texasset",
                "mod.ltx",
            ]
        );
        assert_eq!(walk.published, 6);
        // the excluded folder is what the walk passed by; the sources are no news
        assert_eq!(walk.left_out_texts(), ["wip"]);
        assert_eq!(plan_of(&index).published.len(), 6);

        // installed twice: the second build reuses every file of the first
        let (ctx, _gate) = gz_build_test_ctx(&rt);
        let game = std::env::temp_dir().join(format!("gz-release-media-game-{}", std::process::id()));
        let dst = game.join("modules").join("od");
        let installed = install_build(&walk, &dst, true, &exec, &ctx, &noop);
        assert_eq!(installed.as_ref().map_err(|r| r.err.as_str()), Ok(&8));
        let dds = dst.join("gamedata\\textures\\od\\wall.dds");
        assert!(fs::read(&dds).unwrap().starts_with(b"DDS "));
        assert_eq!(
            fs::read(dst.join("gamedata\\textures\\od\\wall.texasset")).unwrap(),
            fs::read(d.0.join("art\\wall.texasset")).unwrap()
        );
        let first = fs::metadata(&dds).unwrap().modified().unwrap();
        let again = publication(&d.0, &gz_media::scan(&d.0), None, &gz_media::resolve::no_game());
        let mut walk2 =
            BuildWalk::scan_with(&FileTree::scan(&d.0), &rules, WalkScope::Module, &exec, &again.sources);
        walk2.publish(again.files);
        let unchanged = walk2.files.iter().filter(|f| gz_build::install::same_file_there(f, &dst)).count();
        assert_eq!(unchanged, 8, "every file of the second build is the first's");
        // a clean build without the question: nothing in the target is a stale file
        assert_eq!(install_build(&walk2, &dst, false, &exec, &ctx, &noop), Ok(8));
        assert_eq!(fs::metadata(&dds).unwrap().modified().unwrap(), first);
        let _ = fs::remove_dir_all(&game);
    }

    #[test]
    fn an_asset_that_cannot_be_baked_stops_the_build_by_name() {
        let d = Dir::new("broken");
        d.put("mod.ltx", "[module]\nid = od\n");
        d.put("art/gone.texasset", "return { tex = 1, source = \"gone.png\" }\n");
        d.put("art/bad.sndasset", "return { snd = ");
        let p = publication(&d.0, &gz_media::scan(&d.0), None, &gz_media::resolve::no_game());
        assert_eq!(p.errors.len(), 1, "{:?}", p.errors);
        assert!(p.errors[0].starts_with("art/bad.sndasset"), "{:?}", p.errors);
        let d2 = Dir::new("unbaked");
        d2.put("mod.ltx", "[module]\nid = od\n");
        d2.put("art/gone.texasset", "return { tex = 1, source = \"gone.png\" }\n");
        let p = publication(&d2.0, &gz_media::scan(&d2.0), None, &gz_media::resolve::no_game());
        assert!(p.errors.iter().any(|e| e.starts_with("art/gone.texasset")), "{:?}", p.errors);
        assert!(p.files.is_empty());
    }

    #[test]
    fn a_game_that_does_not_read_the_descriptions_gets_a_warning() {
        let d = Dir::new("game");
        project(&d);
        let game = d.0.join("game");
        fs::create_dir_all(&game).unwrap();
        let p = publication(&d.0, &gz_media::scan(&d.0), Some(&game), &gz_media::resolve::no_game());
        assert_eq!(p.warnings, [no_descriptions_warning(None)]);
        assert!(p.warnings[0].contains("asset-descriptions"));
        assert!(no_descriptions_warning(Some("1.6.4")).starts_with("The linked game (1.6.4) does not state"));
    }

    /// A job context of a real job that stays open while the test holds the gate.
    fn gz_build_test_ctx(rt: &gz_task::Runtime) -> (gz_task::JobCtx, std::sync::mpsc::Sender<()>) {
        let jobs = gz_task::Jobs::new(rt.exec());
        let (give, take) = std::sync::mpsc::channel();
        let (hold, wait) = std::sync::mpsc::channel::<()>();
        let started = jobs.start("test", None, "test", move |ctx| async move {
            let _ = give.send(ctx);
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = wait.recv();
                let _ = tx.send(());
            });
            let _ = rx.await;
            Ok(serde_json::Value::Null)
        });
        assert!(started.is_ok());
        (take.recv().unwrap(), hold)
    }
}
