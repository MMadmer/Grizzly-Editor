//! The project's text service: the index of the scripts, made once per state of the linked game
//! and of the project's scripts.
//!
//! The index is the closure given to `Query::fresh`: however many ask at once, one of them reads
//! the scripts (the game's in runs through its archives, the project's in parallel, all on the
//! IO pool) and parses them on the CPU pool; the texts go as each is parsed, the declarations
//! stay. Its revision is the game index's and a stamp of the project's scripts (the watcher's
//! count of their changes and their list, or - without a watcher - their sizes and times).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gz_base::{RelPath, Rev, RevCounter};
use gz_editor::{Editor, Executors, ProjectSession};
use gz_game::GameLink;
use gz_project::{EntryKind, FileTree, Filter, Project};
use gz_query::Query;
use gz_vfs::GameFiles;
use parking_lot::{Mutex, RwLock};

use crate::file::{TextFile, decode};
use crate::index::{DeclSource, ScriptDecls, ScriptIndex, declarations};

/// The game's script namespaces: the files right in `scripts\`.
const SCRIPTS: &str = "scripts";

/// A seed of the FNV-1a hash the stamps are made with.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Moves the revision when a game is linked or unlinked with the same project stamp.
const LINKED: Rev = Rev(0x6c69_6e6b);

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A script's namespace: its file's name without the extension, lower case.
fn namespace(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_ascii_lowercase()
}

/// The index of the scripts and the game's files a tab opens read-only.
pub struct TextService {
    exec: Executors,
    index: Query<(), ScriptIndex>,
    latest: RwLock<Option<Arc<ScriptIndex>>>,
    /// The project's stamp for a state of its tree, its manifest and its scripts' changes.
    memo: Mutex<Option<(Rev, Rev)>>,
    /// Moves with every change of a script the project's watcher reports.
    changes: Option<Arc<RevCounter>>,
}

/// A counter the project's watcher moves on every change of a script.
fn follow_scripts(editor: &Editor, project: &Arc<Project>) -> Arc<RevCounter> {
    let counter = Arc::new(RevCounter::new());
    let mut rx = project.tree.changes().subscribe(Filter::Extensions(vec!["script".into()]));
    let (weak, tree) = (Arc::downgrade(&counter), Arc::downgrade(project));
    editor.exec.spawn(async move {
        while rx.recv().await.is_some() {
            if let Some(p) = tree.upgrade() {
                p.tree.refresh().await;
            }
            match weak.upgrade() {
                Some(c) => {
                    c.bump();
                }
                None => break,
            }
        }
    });
    counter
}

/// What the index reads of the project: the scripts the build ships, and their stamp.
struct ProjectScripts {
    root: PathBuf,
    files: Vec<RelPath>,
    stamp: Rev,
}

impl TextService {
    pub fn of_session(editor: &Editor, session: &ProjectSession) -> Arc<Self> {
        session.services.get_or_init(|| TextService {
            exec: editor.exec.clone(),
            index: Query::new(editor.exec.clone()),
            latest: RwLock::new(None),
            memo: Mutex::new(None),
            changes: session.services.get::<Project>().map(|p| follow_scripts(editor, &p)),
        })
    }

    /// The `.script` files of the project the build ships, in path order.
    fn script_files(tree: &FileTree, rules: &[String]) -> Vec<RelPath> {
        tree.files_with_extension("script")
            .into_iter()
            .filter(|r| !gz_mod::outside_build(rules, r.as_str()))
            .collect()
    }

    fn stamp_of(&self, project: &Project, tree: &FileTree, files: &[RelPath], rules: &[String]) -> Rev {
        let mut h = FNV_OFFSET;
        for rule in rules {
            h = fnv(h, rule.as_bytes());
            h = fnv(h, b"\n");
        }
        let live = project.tree.changes().live() && self.changes.is_some();
        if let Some(c) = self.changes.as_ref().filter(|_| live) {
            h = fnv(h, &c.get().0.to_le_bytes());
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

    /// The project's scripts as its tree and its manifest have them now; none without a project.
    async fn project_scripts(&self, editor: &Editor) -> Option<ProjectScripts> {
        let (session, project) = Project::of(editor).ok()?;
        let manifest = gz_mod::ManifestService::of_session(editor, &session).await.ok();
        let (rules, manifest_rev) = manifest.as_ref().map_or((Vec::new(), Rev::ZERO), |m| {
            (m.snapshot().manifest().map(|x| x.exclude.clone()).unwrap_or_default(), m.rev())
        });
        let tree = project.tree.snapshot();
        let files = Self::script_files(&tree, &rules);
        let cheap = tree.rev().mix(manifest_rev).mix(self.changes.as_ref().map_or(Rev::ZERO, |c| c.get()));
        let memo = *self.memo.lock();
        let stamp = match memo {
            Some((key, stamp)) if key == cheap => stamp,
            _ => {
                let stamp = self.stamp_of(&project, &tree, &files, &rules);
                *self.memo.lock() = Some((cheap, stamp));
                stamp
            }
        };
        Some(ProjectScripts { root: PathBuf::from(project.root()), files, stamp })
    }

    /// The index as the game and the project's scripts are now (made, or waited for, once per
    /// state of them).
    pub async fn index(&self, editor: &Arc<Editor>) -> Arc<ScriptIndex> {
        let game = match GameLink::of(editor) {
            Ok(link) => link.snapshot().await.ok(),
            Err(_) => None,
        };
        let project = self.project_scripts(editor).await;
        let mut rev = project.as_ref().map_or(Rev::ZERO, |p| p.stamp);
        if let Some((_, game_rev)) = &game {
            rev = rev.mix(*game_rev).mix(LINKED);
        }
        if let Some(made) = self.index.peek_at(&(), rev) {
            *self.latest.write() = Some(made.clone());
            return made;
        }
        let exec = self.exec.clone();
        let files = game.map(|(f, _)| f);
        let made = self
            .index
            .fresh(&(), rev, move || {
                build_index(
                    &exec,
                    files.as_deref(),
                    project.as_ref().map(|p| (p.root.as_path(), p.files.as_slice())),
                )
            })
            .await;
        *self.latest.write() = Some(made.clone());
        made
    }

    /// The index made last, without waiting: what the window completes with while a newer one is
    /// made.
    #[must_use]
    pub fn index_now(&self) -> Option<Arc<ScriptIndex>> {
        self.latest.read().clone()
    }

    /// Where a script is, by its namespace (`xr_logic`): the project's when it has one, else the
    /// linked game's.
    pub async fn script_source(&self, editor: &Arc<Editor>, script: &str) -> Option<DeclSource> {
        self.index(editor).await.source_of(script).cloned()
    }

    /// A text file of the linked game (a script Go to Definition opens read-only), by its path
    /// there (`scripts\xr_logic.script`).
    pub async fn read_game(&self, editor: &Arc<Editor>, path: &str) -> Result<TextFile, String> {
        let link = GameLink::of(editor).map_err(|e| e.to_string())?;
        let files = link.files().await.map_err(|e| e.to_string())?;
        let path = path.trim().replace('/', "\\");
        let name = path.rsplit('\\').next().unwrap_or(&path).to_owned();
        self.exec
            .io(move || {
                let size =
                    files.get(&path).map(|f| f.size).ok_or_else(|| format!("the game has no file {path}"))?;
                if size > crate::file::MAX_BYTES {
                    return Err(crate::file::ReadError::TooLarge(size).to_string());
                }
                let bytes = files.read(&path).map_err(|e| format!("{path} could not be read: {e}"))?;
                decode(&bytes, &name).map_err(|e| e.to_string())
            })
            .await
    }
}

/// The text of a script for its declarations: as it decodes, or nothing for one that does not.
fn script_text(bytes: &[u8], name: &str) -> Option<String> {
    decode(bytes, name).ok().map(|f| f.text)
}

/// The index of the game's scripts and the project's (`root` and its files): read on the IO
/// pool, parsed on the CPU pool. Blocking: runs on a pool.
fn build_index(
    exec: &Executors,
    game: Option<&GameFiles>,
    project: Option<(&Path, &[RelPath])>,
) -> ScriptIndex {
    let mut scripts: Vec<ScriptDecls> = Vec::new();
    if let Some(files) = game {
        let paths: Vec<&str> = files
            .find(SCRIPTS, "script")
            .into_iter()
            .map(|f| f.path)
            .filter(|p| p.get(SCRIPTS.len() + 1..).is_some_and(|rest| !rest.contains('/')))
            .collect();
        let bytes = exec.io_pool().install(|| files.read_many(&paths));
        let read: Vec<(String, Vec<u8>)> =
            paths.iter().zip(bytes).filter_map(|(p, b)| b.ok().map(|b| (p.replace('/', "\\"), b))).collect();
        scripts.extend(exec.par_cpu(read, |(path, bytes)| {
            let script = namespace(&path);
            let source = DeclSource::Game(path);
            let decls =
                script_text(&bytes, &script).map_or_else(Vec::new, |t| declarations(&script, &t, &source));
            ScriptDecls { script, source, decls }
        }));
    }
    if let Some((root, files)) = project {
        let read: Vec<(RelPath, Option<Vec<u8>>)> = exec.par_io(files.to_vec(), |rel| {
            let bytes = std::fs::read(rel.to_fs(root)).ok();
            (rel, bytes)
        });
        scripts.extend(exec.par_cpu(read, |(rel, bytes)| {
            let script = namespace(rel.as_str());
            let text = bytes.and_then(|b| script_text(&b, rel.file_name()));
            let source = DeclSource::Project(rel);
            let decls = text.map_or_else(Vec::new, |t| declarations(&script, &t, &source));
            ScriptDecls { script, source, decls }
        }));
    }
    ScriptIndex::of_scripts(scripts)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    use gz_task::{ExecOptions, Runtime};

    #[test]
    fn a_namespace_is_the_files_name() {
        assert_eq!(namespace("scripts\\XR_Logic.script"), "xr_logic");
        assert_eq!(namespace("gamedata/scripts/_g.script"), "_g");
    }

    #[test]
    fn the_index_of_a_folder_of_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("gamedata/scripts")).unwrap();
        std::fs::write(root.join("gamedata/scripts/my_mod.script"), "function start(npc)\nend\n").unwrap();
        std::fs::write(root.join("gamedata/scripts/_g.script"), "function printf(fmt, ...) end\n").unwrap();
        let rt = Runtime::start(ExecOptions { async_threads: 1, cpu_threads: 2, io_threads: 2 }).unwrap();
        let exec = rt.exec();
        let files = vec![
            RelPath::new("gamedata/scripts/_g.script").unwrap(),
            RelPath::new("gamedata/scripts/my_mod.script").unwrap(),
            RelPath::new("gamedata/scripts/missing.script").unwrap(),
        ];
        let made = rt.block_on({
            let exec = exec.clone();
            let root = root.to_path_buf();
            async move { exec.clone().cpu(move || build_index(&exec, None, Some((&root, &files)))).await }
        });
        assert_eq!(made.scripts(), 3);
        assert_eq!(made.lookup("my_mod.start")[0].params, "(npc)");
        assert_eq!(made.lookup("printf")[0].source.word(), "project");
    }

    /// What the index of the user's game costs: run by hand (`--ignored`), never by a suite. It
    /// reads the game and writes nothing.
    #[test]
    #[ignore = "reads the installed game"]
    fn the_index_of_the_linked_game() {
        let game = Path::new("D:\\Games\\Dead Air");
        let rt = Runtime::start(ExecOptions::default()).unwrap();
        let exec = rt.exec();
        let t = std::time::Instant::now();
        let files = Arc::new(GameFiles::mount(game, &gz_vfs::MountOptions::default()).unwrap());
        let mounted = t.elapsed();
        let t = std::time::Instant::now();
        let made = rt.block_on({
            let (exec, files) = (exec.clone(), files.clone());
            async move { exec.clone().cpu(move || build_index(&exec, Some(&files), None)).await }
        });
        let built = t.elapsed();
        eprintln!(
            "mounted in {mounted:?}; {} scripts, {} declarations in {built:?}",
            made.scripts(),
            made.decls().len()
        );
        assert!(made.scripts() > 100);
        assert!(!made.lookup("xr_logic.pick_section_from_condlist").is_empty());
        // every script the game runs compiles here (`lua_help` is the engine's list of what it
        // binds, no script)
        let t = std::time::Instant::now();
        let paths: Vec<String> = files
            .find(SCRIPTS, "script")
            .into_iter()
            .map(|f| f.path.to_owned())
            .filter(|p| !p.ends_with("/lua_help.script"))
            .collect();
        let failed: Vec<String> = exec
            .par_cpu(paths, |path| {
                let text = files.read(&path).ok().and_then(|b| script_text(&b, &path)).unwrap_or_default();
                crate::check::check_text(&path, &text)
                    .into_iter()
                    .filter(gz_base::Problem::is_error)
                    .map(|p| format!("{path}: {}", p.message))
                    .collect::<Vec<_>>()
            })
            .into_iter()
            .flatten()
            .collect();
        eprintln!("checked in {:?}; errors: {failed:#?}", t.elapsed());
        assert!(failed.is_empty());
        // and every XML file the game reads
        let paths: Vec<String> =
            files.find("configs", "xml").into_iter().map(|f| f.path.to_owned()).collect();
        let count = paths.len();
        let failed: Vec<String> = exec
            .par_cpu(paths, |path| {
                let text = files.read(&path).ok().and_then(|b| script_text(&b, &path)).unwrap_or_default();
                crate::check::check_text(&path, &text)
                    .into_iter()
                    .map(|p| format!("{path}: {}", p.message))
                    .collect::<Vec<_>>()
            })
            .into_iter()
            .flatten()
            .collect();
        // what the game's own reader would refuse too, were the file loaded
        eprintln!("{count} XML files; problems: {failed:#?}");
    }
}
