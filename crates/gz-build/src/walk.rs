//! What a build ships, as ONE list made before anything is written.
//!
//! A clean build has to know what it will write to tell what in the target it would throw
//! away, and the author is told what the walk passed by: a folder that silently did not ship is
//! the next bug report. The list comes from the project's file tree; the tree never holds
//! tooling (`.git`, `Thumbs.db`), so the folders the walk enters are listed once more for those
//! names alone - that is what `left_out` says the build passed by.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gz_base::{RelPath, SmolStr};
use gz_mod::exclude_match;
use gz_project::rules::is_media_asset_name;
use gz_project::{
    Entry, EntryKind, FileTree, is_editor_only_entry, is_level_build_file, is_source_only_entry,
    is_tooling_entry,
};
use gz_task::Executors;
use rustc_hash::{FxHashMap, FxHashSet};

/// X-Ray cannot open a path longer than this (`string_path`), and neither can the copy.
pub const PATH_LIMIT: usize = 520;

/// Where a file the build writes comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FileFrom {
    /// The project's file at the same path.
    #[default]
    Project,
    /// A file made for the build (a media asset's bake in the project's `temp\media`).
    Made(PathBuf),
    /// Bytes made for the build (a copy of a media asset's text).
    Bytes(Arc<[u8]>),
}

/// One file the build writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFile {
    /// Where it goes, relative to the module's root; for a project's file also where it lies.
    pub rel: RelPath,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub from: FileFrom,
}

impl BuildFile {
    /// The file a copy reads, `None` for bytes.
    #[must_use]
    pub fn source(&self, root: &Path) -> Option<PathBuf> {
        match &self.from {
            FileFrom::Project => Some(self.rel.to_fs(root)),
            FileFrom::Made(p) => Some(p.clone()),
            FileFrom::Bytes(_) => None,
        }
    }
}

/// The project files a build never ships as they are: every file a media asset takes its data
/// from, as [`source_key`] writes them. The media assets themselves are told by their names.
pub type Sources = FxHashSet<String>;

/// A project path as [`Sources`] holds it: forward slashes, lower case.
#[must_use]
pub fn source_key(rel: &str) -> String {
    rel.replace('\\', "/").to_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftOutReason {
    /// Version control, IDE and OS litter.
    Tooling,
    /// A junction or a symbolic link: it may point anywhere, the project itself included.
    Link,
    /// The author's `[exclude]`.
    Excluded,
    /// An author key: whoever has it publishes updates in the author's name.
    KeyFile,
    /// A level build's own file (`build.prj`, `build.state`): what the compiler reads, never what
    /// the game does.
    LevelBuild,
    PathTooLong,
}

/// Where the walk stopped, project-relative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    pub rel: RelPath,
    pub reason: LeftOutReason,
}

impl LeftOut {
    /// What `left_out` says on the wire: the path with backslashes, and why when it is not
    /// plain.
    #[must_use]
    pub fn text(&self) -> String {
        let rel = self.rel.to_backslash();
        match self.reason {
            LeftOutReason::KeyFile => format!("{rel} (a key file never ships)"),
            LeftOutReason::LevelBuild => format!("{rel} (the level build's own file)"),
            LeftOutReason::PathTooLong => format!("{rel} (path too long)"),
            _ => rel,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkScope {
    /// Everything the author made: a module build.
    Module,
    /// `gamedata` alone, filtered exactly like a module build: the flat overlay.
    Gamedata,
}

/// The tooling names of the folders the walk enters: `(name, is a folder)`.
pub type Tooling = FxHashMap<RelPath, Vec<(SmolStr, bool)>>;

/// The build's list.
#[derive(Clone, Debug, Default)]
pub struct BuildWalk {
    pub root: PathBuf,
    /// In walk order: folder by folder, names in the order the folder lists them.
    pub files: Vec<BuildFile>,
    pub left_out: Vec<LeftOut>,
    /// The size of every file together.
    pub bytes: u64,
    /// How many of `files` the media assets publish.
    pub published: usize,
}

enum Seen<'a> {
    Tree(&'a Entry),
    Tooling(&'a str, bool),
}

impl Seen<'_> {
    fn name(&self) -> &str {
        match self {
            Seen::Tree(e) => &e.name,
            Seen::Tooling(n, _) => n,
        }
    }
}

struct Walker<'a> {
    tree: &'a FileTree,
    rules: &'a [String],
    root_len: usize,
    tooling: Option<&'a Tooling>,
    sources: &'a Sources,
    out: BuildWalk,
    /// The folders entered, for the tooling listing.
    entered: Vec<RelPath>,
}

impl Walker<'_> {
    fn skip(&mut self, rel: &RelPath, seen: &Seen<'_>, at_root: bool) -> bool {
        let name = seen.name();
        // the manifest is what makes the folder a module - no rule takes it out
        if at_root && name.eq_ignore_ascii_case(gz_mod::FILE) && matches!(seen, Seen::Tree(e) if e.is_file())
        {
            return false;
        }
        // the editor's scratch and the sources never ship, and the author knows it
        if at_root && (is_editor_only_entry(name) || is_source_only_entry(name)) {
            return true;
        }
        // authoring help beside the patches, not content
        if name.eq_ignore_ascii_case("_readme.txt")
            && rel.segments().next().is_some_and(|s| s.eq_ignore_ascii_case("patch"))
            && rel.depth() > 1
        {
            return true;
        }
        let folder = match seen {
            Seen::Tree(e) => e.kind == EntryKind::Dir,
            Seen::Tooling(_, dir) => *dir,
        };
        if !folder && rel.has_extension("xkey") {
            self.out.left_out.push(LeftOut { rel: rel.clone(), reason: LeftOutReason::KeyFile });
            return true;
        }
        // a media asset where it lies and what it is made of are sources: the build publishes
        // the asset under its name, and passing them by is no news for the author
        if !folder
            && matches!(seen, Seen::Tree(_))
            && (is_media_asset_name(name) || self.sources.contains(&source_key(rel.as_str())))
        {
            return true;
        }
        if !folder && is_level_build_file(rel.as_str()) {
            self.out.left_out.push(LeftOut { rel: rel.clone(), reason: LeftOutReason::LevelBuild });
            return true;
        }
        let reason = match seen {
            Seen::Tooling(..) => Some(LeftOutReason::Tooling),
            Seen::Tree(e) if e.kind == EntryKind::Link => Some(LeftOutReason::Link),
            Seen::Tree(_) if exclude_match(self.rules, rel.as_str()).is_some() => {
                Some(LeftOutReason::Excluded)
            }
            Seen::Tree(_) if is_tooling_entry(name) => Some(LeftOutReason::Tooling),
            Seen::Tree(_) => None,
        };
        if let Some(reason) = reason {
            self.out.left_out.push(LeftOut { rel: rel.clone(), reason });
            return true;
        }
        if self.root_len + rel.as_str().len() + 2 >= PATH_LIMIT {
            self.out.left_out.push(LeftOut { rel: rel.clone(), reason: LeftOutReason::PathTooLong });
            return true;
        }
        false
    }

    fn walk(&mut self, dir_rel: &RelPath) {
        self.entered.push(dir_rel.clone());
        let Some(dir) = self.tree.dir(dir_rel) else { return };
        let dir = dir.clone();
        let extra: &[(SmolStr, bool)] = self.tooling.and_then(|t| t.get(dir_rel)).map_or(&[], Vec::as_slice);
        let mut items: Vec<Seen<'_>> = dir
            .entries
            .iter()
            .map(Seen::Tree)
            .chain(extra.iter().map(|(n, d)| Seen::Tooling(n, *d)))
            .collect();
        items.sort_by_cached_key(|s| s.name().to_ascii_lowercase());
        let at_root = dir_rel.is_root();
        for seen in items {
            let Ok(rel) = dir_rel.join(seen.name()) else { continue };
            if self.skip(&rel, &seen, at_root) {
                continue;
            }
            let Seen::Tree(entry) = seen else { continue };
            match entry.kind {
                EntryKind::Dir => self.walk(&rel),
                EntryKind::File { size, modified, .. } => {
                    self.out.bytes += size;
                    self.out.files.push(BuildFile { rel, size, modified, from: FileFrom::Project });
                }
                EntryKind::Link => {}
            }
        }
    }
}

impl BuildWalk {
    fn run(
        tree: &FileTree,
        rules: &[String],
        scope: WalkScope,
        tooling: Option<&Tooling>,
        sources: &Sources,
    ) -> (BuildWalk, Vec<RelPath>) {
        let root_len = tree.root().as_os_str().len();
        let mut w = Walker {
            tree,
            rules,
            root_len,
            tooling,
            sources,
            out: BuildWalk { root: tree.root().to_path_buf(), ..BuildWalk::default() },
            entered: Vec::new(),
        };
        match scope {
            WalkScope::Module => w.walk(&RelPath::root()),
            WalkScope::Gamedata => {
                if let Ok(gamedata) = RelPath::new("gamedata") {
                    if exclude_match(rules, "gamedata").is_some() {
                        w.out.left_out.push(LeftOut { rel: gamedata, reason: LeftOutReason::Excluded });
                    } else {
                        w.walk(&gamedata);
                    }
                }
            }
        }
        (w.out, w.entered)
    }

    /// The walk over `tree` with the tooling names `tooling` says the folders hold. Pure.
    #[must_use]
    pub fn from_tree(
        tree: &FileTree,
        rules: &[String],
        scope: WalkScope,
        tooling: &Tooling,
        sources: &Sources,
    ) -> Self {
        Self::run(tree, rules, scope, Some(tooling), sources).0
    }

    /// The folders a walk enters.
    #[must_use]
    pub fn folders(tree: &FileTree, rules: &[String], scope: WalkScope) -> Vec<RelPath> {
        Self::run(tree, rules, scope, None, &Sources::default()).1
    }

    /// The walk, with the folders it enters listed for tooling in parallel on the IO pool. The
    /// media assets where they lie are left out; their sources only when [`Self::scan_with`]
    /// names them. Blocking: call it on the IO pool.
    #[must_use]
    pub fn scan(tree: &FileTree, rules: &[String], scope: WalkScope, exec: &Executors) -> Self {
        Self::scan_with(tree, rules, scope, exec, &Sources::default())
    }

    /// [`Self::scan`] with the files the media assets are made of left out. Blocking.
    #[must_use]
    pub fn scan_with(
        tree: &FileTree,
        rules: &[String],
        scope: WalkScope,
        exec: &Executors,
        sources: &Sources,
    ) -> Self {
        let folders = Self::folders(tree, rules, scope);
        let root = tree.root().to_path_buf();
        let listed = exec.par_io(folders, |rel| {
            let names = tooling_names(&rel.to_fs(&root));
            (rel, names)
        });
        let tooling: Tooling = listed.into_iter().filter(|(_, n)| !n.is_empty()).collect();
        Self::from_tree(tree, rules, scope, &tooling, sources)
    }

    /// Adds what the media assets publish. A published file takes the place of a project file
    /// at the same path; those paths are answered (backslashes), for a warning.
    pub fn publish(&mut self, published: Vec<BuildFile>) -> Vec<String> {
        let keys: FxHashSet<String> = published.iter().map(|f| f.rel.as_str().to_lowercase()).collect();
        let mut replaced = Vec::new();
        self.files.retain(|f| {
            let taken = keys.contains(&f.rel.as_str().to_lowercase());
            if taken {
                replaced.push(f.rel.to_backslash());
            }
            !taken
        });
        self.published += published.len();
        self.files.extend(published);
        self.bytes = self.files.iter().map(|f| f.size).sum();
        replaced
    }

    /// The files' project-relative paths as the stale count compares them: backslashes, lower
    /// case.
    #[must_use]
    pub fn keys(&self) -> rustc_hash::FxHashSet<String> {
        self.files.iter().map(|f| f.rel.to_backslash().to_lowercase()).collect()
    }

    /// `left_out` as the wire names it.
    #[must_use]
    pub fn left_out_texts(&self) -> Vec<String> {
        self.left_out.iter().map(LeftOut::text).collect()
    }
}

/// The tooling entries of one folder.
fn tooling_names(dir: &Path) -> Vec<(SmolStr, bool)> {
    let Ok(listing) = std::fs::read_dir(dir) else { return Vec::new() };
    listing
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !is_tooling_entry(&name) {
                return None;
            }
            let folder = e.file_type().is_ok_and(|t| t.is_dir());
            Some((SmolStr::new(name), folder))
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn walks_what_ships_and_says_what_not() {
        let dir = std::env::temp_dir().join(format!("gz-build-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for d in [
            ".git\\objects",
            "gamedata\\textures\\qa",
            "gamedata\\.git",
            "gamedata\\levels\\l01\\.lc_new",
            "design",
            "temp",
            "rawdata",
            "patch",
            "docs",
        ] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in [
            "mod.ltx",
            "LICENSE",
            ".gitignore",
            "gamedata\\textures\\qa\\a.dds",
            "gamedata\\textures\\qa\\Thumbs.db",
            "gamedata\\configs.ltx",
            "gamedata\\levels\\l01\\level.ai",
            "gamedata\\levels\\l01\\build.prj",
            "gamedata\\levels\\l01\\build.state",
            "design\\story.md",
            "art.psd",
            "temp\\x",
            "rawdata\\scene.level",
            "patch\\_readme.txt",
            "patch\\fix.ltxp",
            "author.xkey",
            "project.ltx",
            "docs\\notes.md",
        ] {
            fs::write(dir.join(f), "x").unwrap();
        }
        let tree = FileTree::scan(&dir);
        let rules = vec!["design".to_owned(), "*.psd".to_owned(), "docs/".to_owned()];
        let exec_rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 2 })
                .unwrap();
        let exec = exec_rt.exec();
        let walk = BuildWalk::scan(&tree, &rules, WalkScope::Module, &exec);
        let files: Vec<&str> = walk.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            files,
            [
                "gamedata/configs.ltx",
                "gamedata/levels/l01/level.ai",
                "gamedata/textures/qa/a.dds",
                "LICENSE",
                "mod.ltx",
                "patch/fix.ltxp"
            ]
        );
        assert_eq!(walk.bytes, 6);
        let left = walk.left_out_texts();
        assert_eq!(
            left,
            [
                ".git",
                ".gitignore",
                "art.psd",
                "author.xkey (a key file never ships)",
                "design",
                "docs",
                "gamedata\\.git",
                "gamedata\\levels\\l01\\.lc_new",
                "gamedata\\levels\\l01\\build.prj (the level build's own file)",
                "gamedata\\levels\\l01\\build.state (the level build's own file)",
                "gamedata\\textures\\qa\\Thumbs.db"
            ]
        );

        let flat = BuildWalk::scan(&tree, &rules, WalkScope::Gamedata, &exec);
        assert_eq!(flat.files.len(), 3);
        let none = BuildWalk::scan(&tree, &["gamedata".to_owned()], WalkScope::Gamedata, &exec);
        assert!(none.files.is_empty());
        assert_eq!(none.left_out_texts(), ["gamedata"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_media_asset_and_its_source_are_sources_and_its_publication_ships() {
        let dir = std::env::temp_dir().join(format!("gz-build-media-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("art")).unwrap();
        fs::create_dir_all(dir.join("gamedata\\textures\\od")).unwrap();
        for f in [
            "mod.ltx",
            "art\\wall.texasset",
            "art\\wall.png",
            "art\\Crate.MeshAsset",
            "art\\notes.png",
            "gamedata\\textures\\od\\floor.dds",
        ] {
            fs::write(dir.join(f), "x").unwrap();
        }
        let tree = FileTree::scan(&dir);
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 2 })
                .unwrap();
        let exec = rt.exec();
        let sources: Sources = [source_key("art\\WALL.png")].into_iter().collect();
        let mut walk = BuildWalk::scan_with(&tree, &[], WalkScope::Module, &exec, &sources);
        let files: Vec<&str> = walk.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(files, ["art/notes.png", "gamedata/textures/od/floor.dds", "mod.ltx"]);
        assert!(walk.left_out.is_empty(), "{:?}", walk.left_out_texts());
        // without the sources named, only the assets themselves stay home
        let plain = BuildWalk::scan(&tree, &[], WalkScope::Module, &exec);
        assert_eq!(plain.files.len(), 4);

        let made = |rel: &str, size: u64| BuildFile {
            rel: RelPath::new(rel).unwrap(),
            size,
            modified: None,
            from: FileFrom::Bytes(Arc::from(vec![b'x'; size as usize])),
        };
        let replaced = walk.publish(vec![
            made("gamedata/textures/od/floor.dds", 3),
            made("gamedata/textures/od/floor.texasset", 2),
        ]);
        assert_eq!(replaced, ["gamedata\\textures\\od\\floor.dds"]);
        assert_eq!(walk.published, 2);
        assert_eq!(walk.files.len(), 4);
        assert_eq!(walk.bytes, 1 + 1 + 3 + 2);
        assert!(walk.keys().contains("gamedata\\textures\\od\\floor.texasset"));
        assert_eq!(walk.files.last().unwrap().source(&dir), None);
        assert_eq!(walk.files[0].source(&dir), Some(dir.join("art").join("notes.png")));
        drop(rt);
        fs::remove_dir_all(&dir).unwrap();
    }
}
