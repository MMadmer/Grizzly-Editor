//! The one list of kinds: what each kind of text asset is (its extension, header key, current
//! version) and what it does (its migrations, its check, its reference, its diff rules). The
//! kind crates hang their entries here when they install; the commands, the read and save paths
//! and the build read the list.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Problem, RelPath};
use gz_editor::Editor;
use gz_luadata::{DiffRules, Limits, LuaDataError, LuaDoc};
use gz_project::AssetKind;
use parking_lot::RwLock;

use crate::file::AssetFile;

/// One step from version n to n + 1: edits the document in memory (the reader then writes the
/// new version into the header).
pub type Migration = fn(&mut LuaDoc) -> Result<(), LuaDataError>;

/// The problems of one file, positioned through its document. Reads snapshots, never blocks.
pub type CheckFn = fn(Arc<Editor>, Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>>;

/// The kind's reference as text: its format document and what the linked game adds.
pub type ReferenceFn = fn(Arc<Editor>) -> BoxFuture<'static, String>;

/// An open document of the kind takes the file's value of one conflict a rebase left (the
/// n-th; none: every one): answers how many conflicts are left.
pub type TakeTheirsFn = fn(Arc<Editor>, RelPath, Option<usize>) -> BoxFuture<'static, Result<usize, String>>;

/// A kind of text asset.
#[derive(Clone)]
pub struct KindOps {
    /// What the author calls it: `quest`.
    pub name: &'static str,
    /// Its extension, with the dot.
    pub ext: &'static str,
    /// The key that names the kind and its version.
    pub header: &'static str,
    /// The version this editor writes and the newest it reads.
    pub version: u32,
    /// What its reader accepts.
    pub limits: Limits,
    /// Steps from version n to n + 1, in order (the first lifts version 1); applied in memory
    /// on read.
    pub migrations: &'static [Migration],
    /// The keys each level of the kind knows, as patterns of [`DiffRules`] (`""` the top,
    /// `"nodes.*"` every node): any other key is the author's, kept and listed as a note.
    pub known: &'static [(&'static str, &'static [&'static str])],
    /// Where the format is written down, for `asset.kinds`.
    pub reference_doc: &'static str,
    pub check: CheckFn,
    pub reference: ReferenceFn,
    pub diff_rules: fn() -> DiffRules,
    /// How an open document of the kind takes the file's side of a conflict; none for a kind
    /// whose documents do not rebase.
    pub take_theirs: Option<TakeTheirsFn>,
}

impl KindOps {
    /// An entry for one of the editor's own kinds: its name, extension, header and version from
    /// the kind table, an asset's limits, no migration, nothing known, no check.
    #[must_use]
    pub fn of(kind: AssetKind) -> Self {
        Self {
            name: kind.name(),
            ext: kind.extension(),
            header: kind.header(),
            version: kind.version(),
            limits: Limits::asset(),
            migrations: &[],
            known: &[],
            reference_doc: "",
            check: |_, _| Box::pin(async { Vec::new() }),
            reference: |_| Box::pin(async { String::new() }),
            diff_rules: DiffRules::new,
            take_theirs: None,
        }
    }

    /// Whether a file name is of this kind (its extension, any case).
    #[must_use]
    pub fn owns(&self, name: &str) -> bool {
        name.len() > self.ext.len()
            && name.get(name.len() - self.ext.len()..).is_some_and(|e| e.eq_ignore_ascii_case(self.ext))
    }

    /// Whether `word` names this kind: its name, its extension (with or without the dot) or
    /// its header key.
    #[must_use]
    pub fn named(&self, word: &str) -> bool {
        let w = word.trim().trim_start_matches('.');
        w.eq_ignore_ascii_case(self.name)
            || w.eq_ignore_ascii_case(self.ext.trim_start_matches('.'))
            || w.eq_ignore_ascii_case(self.header)
            || (self.name.ends_with(" graph") && w.eq_ignore_ascii_case(self.name.trim_end_matches(" graph")))
    }
}

/// A check of what the module manifest says about a kind (`[pda] layout` naming a file that is
/// not there): its problems are the manifest's.
pub type ManifestCheckFn = fn(Arc<Editor>) -> BoxFuture<'static, Vec<Problem>>;

/// A check of a text file that is no asset (a script, XML, JSON), by its path in the project:
/// its problems, in the file.
pub type TextCheckFn = fn(Arc<Editor>, RelPath) -> BoxFuture<'static, Vec<Problem>>;

/// Every kind, in the order the kind crates installed them. Lives in `editor.services`.
#[derive(Default)]
pub struct Kinds {
    list: RwLock<Vec<Arc<KindOps>>>,
    /// The build checks whose problems `asset.check` gives per file already (by their stage).
    covered: RwLock<Vec<&'static str>>,
    manifest: RwLock<Vec<ManifestCheckFn>>,
    /// The checks of text files that are no assets, by the extensions they read.
    text: RwLock<Vec<(&'static [&'static str], TextCheckFn)>>,
}

impl Kinds {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Kinds> {
        editor.services.get_or_init(Kinds::default)
    }

    /// Adds a kind; one added again for the same extension takes the place of the first.
    pub fn add(&self, ops: KindOps) {
        let mut list = self.list.write();
        list.retain(|k| !k.ext.eq_ignore_ascii_case(ops.ext));
        list.push(Arc::new(ops));
    }

    #[must_use]
    pub fn all(&self) -> Vec<Arc<KindOps>> {
        self.list.read().clone()
    }

    /// The kind of a file, by its extension.
    #[must_use]
    pub fn of_file(&self, name: &str) -> Option<Arc<KindOps>> {
        self.list.read().iter().find(|k| k.owns(name)).cloned()
    }

    /// The kind a word names (see [`KindOps::named`]).
    #[must_use]
    pub fn named(&self, word: &str) -> Option<Arc<KindOps>> {
        self.list.read().iter().find(|k| k.named(word)).cloned()
    }

    /// A build check whose problems `asset.check` already gives per file, by its stage: the
    /// project check runs every other one for the manifest.
    pub fn covers_build_stage(&self, stage: &'static str) {
        let mut c = self.covered.write();
        if !c.contains(&stage) {
            c.push(stage);
        }
    }

    #[must_use]
    pub fn covered(&self, stage: &str) -> bool {
        self.covered.read().contains(&stage)
    }

    /// Adds a check of what the manifest says about a kind; `asset.check` of the manifest runs
    /// it.
    pub fn add_manifest_check(&self, check: ManifestCheckFn) {
        self.manifest.write().push(check);
    }

    #[must_use]
    pub fn manifest_checks(&self) -> Vec<ManifestCheckFn> {
        self.manifest.read().clone()
    }

    /// Adds a check of the text files with these extensions (without the dot, any case):
    /// `asset.check` runs it for such a file that no kind owns. A check added again for an
    /// extension takes the place of the first.
    pub fn add_text_check(&self, exts: &'static [&'static str], check: TextCheckFn) {
        let mut text = self.text.write();
        text.retain(|(e, _)| !e.iter().any(|x| exts.iter().any(|y| x.eq_ignore_ascii_case(y))));
        text.push((exts, check));
    }

    /// The text check of a file, by its name: `None` for an asset of a kind, the manifest, or a
    /// file no text check reads.
    #[must_use]
    pub fn text_check(&self, name: &str) -> Option<TextCheckFn> {
        if self.of_file(name).is_some() || name.eq_ignore_ascii_case(crate::check::MANIFEST) {
            return None;
        }
        let ext = name.rsplit_once('.').map(|(_, e)| e)?;
        self.text
            .read()
            .iter()
            .find(|(exts, _)| exts.iter().any(|e| e.eq_ignore_ascii_case(ext)))
            .map(|(_, f)| *f)
    }
}
