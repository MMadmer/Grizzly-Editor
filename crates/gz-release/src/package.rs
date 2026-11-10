//! Package Release (`EditorMod::PackageRelease`): a job that stages the COMPLETE module through
//! the real build (`gz_build::run_build` into the project's `temp\release_stage_<pid>`), so a
//! release ships what a build ships and refuses what a build refuses; then writes into
//! `<project>\releases\<version>` the packages, the file index and - last, because it vouches
//! for the other two - the descriptor, with its `[card]` and, when the author key can sign,
//! its `[signature]`. Never a patch or a delta.
//!
//! Entries are compressed in parallel on the CPU pool and written in order; every file's
//! SHA-256 is taken while it is fed. The index states what is ON DISK: offsets, packed lengths
//! and methods are read back from the finished archive.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use gz_build::{BuildKind, BuildRequest, BuildStatus, Builds, run_build};
use gz_editor::{CmdError, Editor, Executors, JobCtx, JobError, JobId, ProjectSession};
use gz_mod::{Manifest, ManifestService};
use gz_project::Project;
use gz_task::Cancelled;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;

use crate::card::{CardFile, StringTables, card_section};
use crate::content::{Finding, Level, check_content, content_head, content_need};
use crate::crypto::{Sha256Stream, sha256_hex};
use crate::format::{
    ASSET_LIMIT, Asset, INDEX_LIMIT, IndexRecord, MAX_PACKAGES, ReleaseFacts, check_requires_game, descriptor_head, descriptor_name,
    index_name, index_text, is_release_file, package_name, package_path_ok, release_gate, resolve_release_dir, split_parts,
};
use crate::game_version::linked_game_version;
use crate::keys::AuthorKeys;
use crate::zip::{Packed, ZipWriter, deflate, read_layout};

/// What a packaging is asked.
#[derive(Clone, Debug, Default)]
pub struct PackageRequest {
    /// The output folder; empty = `<project>\releases\<version>`. Inside the project only
    /// `releases\` is accepted.
    pub out_dir: String,
    /// The lowest game version the release runs on, `MAJOR.MINOR.PATCH`. `None` = the linked
    /// game's version, `Some("")` = no requirement.
    pub requires_game: Option<String>,
}

/// What a packaging answers.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PackageResult {
    pub ok: bool,
    pub cancelled: bool,
    pub err: String,
    pub id: String,
    pub version: String,
    /// The module's display name.
    pub name: String,
    /// `<out_dir>\<id>.update.ltx`.
    pub descriptor: String,
    /// The descriptor as an asset: a publisher checks every uploaded file against these.
    pub descriptor_file: Asset,
    pub out_dir: String,
    pub index: Asset,
    pub packages: Vec<Asset>,
    pub files: usize,
    pub unpacked: u64,
    pub packed: u64,
    pub requires_game: String,
    /// What the scene's bake did.
    pub baked: String,
    pub warnings: String,
    pub signed: bool,
    pub key_id: String,
    /// What the build passed by.
    pub left_out: Vec<String>,
    /// The Mod Browser's content rules: what it refuses, and what its review looks at.
    pub catalog_block: Vec<String>,
    pub catalog_review: Vec<String>,
}

/// The Task Center's row of a packaging: the build's own row while the module stages.
const STEPS: [&str; 3] = ["Check", "Copy", "Pack"];
const STEP_PACK: usize = 2;
/// Input bytes compressed in memory at once; a file above `STREAM_ABOVE` is deflated as it is
/// read instead.
const BATCH_BYTES: u64 = 96 << 20;
const STREAM_ABOVE: u64 = 32 << 20;
const KEEP_RESULTS: usize = 8;

/// The packagings of the editor: the job, its row, the answers. Lives in `editor.services`.
#[derive(Default)]
pub struct Releases {
    results: Mutex<Vec<(JobId, Arc<PackageResult>)>>,
    last: Mutex<Option<Arc<PackageResult>>>,
    status: Mutex<Option<BuildStatus>>,
    /// The build's row stands for the Copy step.
    staging: AtomicBool,
}

impl Releases {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Starts Package Release of the open project as the job `package` (group `build`: one
    /// build-like job at a time).
    pub fn start(editor: &Arc<Editor>, req: PackageRequest) -> Result<JobId, CmdError> {
        let session = editor.project()?;
        let me = Self::of(editor);
        let ed = editor.clone();
        editor
            .jobs
            .start("package", Some("build"), "Packaging Release", move |ctx| async move {
                let r = Arc::new(run_package(&ed, &session, &req, &ctx).await);
                me.finish(ctx.id, r.clone());
                if r.ok {
                    Ok(json!({ "descriptor": r.descriptor.replace('\\', "/"), "files": r.files }))
                } else if r.cancelled {
                    Err(JobError::from(Cancelled))
                } else {
                    Err(JobError::new(r.err.clone()))
                }
            })
            .map_err(|_| CmdError::busy("a build is running: wait for it to finish, or stop it"))
    }

    fn finish(&self, id: JobId, r: Arc<PackageResult>) {
        let mut results = self.results.lock();
        results.push((id, r.clone()));
        let excess = results.len().saturating_sub(KEEP_RESULTS);
        results.drain(..excess);
        *self.last.lock() = Some(r);
    }

    /// The answer of a packaging job that has ended.
    #[must_use]
    pub fn result(&self, id: JobId) -> Option<Arc<PackageResult>> {
        self.results.lock().iter().find(|(j, _)| *j == id).map(|(_, r)| r.clone())
    }

    /// The last packaging's answer.
    #[must_use]
    pub fn last(&self) -> Option<Arc<PackageResult>> {
        self.last.lock().clone()
    }

    /// The running packaging's row; `None` when none runs.
    #[must_use]
    pub fn status(&self, editor: &Editor) -> Option<BuildStatus> {
        let mut row = self.status.lock().clone()?;
        if self.staging.load(Ordering::Relaxed)
            && let Some(build) = Builds::of(editor).status()
        {
            // the module stages through the build: its Check and Copy are the packaging's
            row.step = build.step.min(1);
            row.stage = build.stage;
            row.file = build.file;
            row.done = build.done;
            row.total = build.total;
            row.files = build.files;
            row.files_total = build.files_total;
        }
        Some(row)
    }

    fn begin(&self, subtitle: String) {
        *self.status.lock() = Some(BuildStatus {
            title: "Packaging Release".to_owned(),
            subtitle,
            steps: STEPS.to_vec(),
            stop_label: "Stop Packaging",
            stop_tip: "Stops the packaging. Nothing is written to the release folder before the packages are done.",
            ..BuildStatus::default()
        });
    }

    fn end(&self) {
        self.staging.store(false, Ordering::Relaxed);
        *self.status.lock() = None;
    }
}

/// What the packaging tells its row and its job, from any thread.
#[derive(Clone)]
struct Feeder {
    releases: Arc<Releases>,
    ctx: JobCtx,
}

impl Feeder {
    fn stage(&self, text: &str, bytes: u64, files: usize) {
        if let Some(s) = self.releases.status.lock().as_mut() {
            s.step = STEP_PACK;
            s.stage = text.to_owned();
            s.file.clear();
            s.done = 0;
            s.total = bytes;
            s.files = 0;
            s.files_total = files;
        }
        self.ctx.stage(text);
    }

    fn file(&self, rel: &str, bytes: u64) {
        if let Some(s) = self.releases.status.lock().as_mut() {
            s.file = rel.to_owned();
            s.files += 1;
            s.done += bytes;
            if s.total > 0 {
                self.ctx.fraction((s.done as f64 / s.total as f64) as f32);
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.ctx.cancel.is_cancelled()
    }
}

fn cancelled() -> String {
    "cancelled".to_owned()
}

/// Runs a packaging to its end inside a job: what Package Release and Publish Release call.
pub async fn run_package(editor: &Arc<Editor>, session: &Arc<ProjectSession>, req: &PackageRequest, ctx: &JobCtx) -> PackageResult {
    let releases = Releases::of(editor);
    releases.begin(String::new());
    let feeder = Feeder { releases: releases.clone(), ctx: ctx.clone() };
    let mut out = PackageResult::default();
    match package(editor, session, req, &feeder, &mut out).await {
        Ok(()) => out.ok = true,
        Err(err) => {
            let keep = PackageResult { err, warnings: std::mem::take(&mut out.warnings), ..PackageResult::default() };
            out = keep;
        }
    }
    out.cancelled = !out.ok && out.err == "cancelled";
    releases.end();
    out
}

/// A job of a project that closed meanwhile has nothing left to package.
fn still_open(editor: &Editor, session: &Arc<ProjectSession>, ctx: &JobCtx) -> Result<(), String> {
    let open = editor.project_opt().is_some_and(|p| Arc::ptr_eq(&p, session));
    if !open || ctx.check().is_err() { Err(cancelled()) } else { Ok(()) }
}

fn add_warning(to: &mut String, line: &str) {
    if line.is_empty() {
        return;
    }
    if !to.is_empty() && !to.ends_with('\n') {
        to.push('\n');
    }
    to.push_str(line);
}

async fn package(editor: &Arc<Editor>, session: &Arc<ProjectSession>, req: &PackageRequest, feeder: &Feeder, out: &mut PackageResult) -> Result<(), String> {
    let ctx = &feeder.ctx;
    let project = session.services.get::<Project>().ok_or_else(|| "no active project".to_owned())?;
    let manifest = ManifestService::of_session(editor, session).await.map_err(|e| e.message)?;
    manifest.sync().await.map_err(|e| e.message)?;
    let m = manifest.snapshot().manifest().ok_or_else(|| "mod.ltx not found in the project".to_owned())?.clone();
    if let Some(s) = feeder.releases.status.lock().as_mut() {
        s.subtitle = format!("{} {}", m.title(), m.version);
    }
    (out.id, out.version, out.name) = (m.id.clone(), m.version.clone(), m.title().to_owned());

    release_gate(&m)?;
    let requires_game = match &req.requires_game {
        Some(v) => v.clone(),
        None => {
            let game = project.game_root();
            editor.exec.io(move || linked_game_version(&game)).await
        }
    };
    check_requires_game(&requires_game)?;
    let root = session.root.clone();
    let out_dir = resolve_release_dir(&root, &req.out_dir, &m.version)?;
    still_open(editor, session, ctx)?;

    // temp\ is editor-only scratch that no export ever reads, so the stage can sit inside the
    // project; the pid keeps two editors on one project apart
    let stage = root.join("temp").join(format!("release_stage_{}", std::process::id()));
    let (stage_io, temp) = (stage.clone(), root.join("temp"));
    let lock = editor
        .exec
        .io(move || {
            remove_dead_stages(&temp, &stage_io);
            let _ = std::fs::create_dir_all(&stage_io);
            if !stage_io.is_dir() {
                return Err("can't create the staging folder in the project's temp".to_owned());
            }
            gz_build::install::wipe_tree(&stage_io);
            Ok(StageLock::take(&stage_io))
        })
        .await?;

    let staged = stage_and_pack(editor, session, feeder, out, &m, &requires_game, &out_dir, &stage).await;
    let stage_io = stage.clone();
    editor
        .exec
        .io(move || {
            drop(lock);
            gz_build::install::remove_tree(&stage_io);
        })
        .await;
    let draft = staged?;
    still_open(editor, session, ctx)?;
    finish_release(editor, &root, &m, &draft, out).await
}

/// What the files of the release left for the descriptor.
struct Draft {
    manifest_ref: CardFile,
    thumbnail_ref: Option<CardFile>,
}

#[allow(clippy::too_many_arguments)]
async fn stage_and_pack(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    feeder: &Feeder,
    out: &mut PackageResult,
    m: &Manifest,
    requires_game: &str,
    out_dir: &str,
    stage: &Path,
) -> Result<Draft, String> {
    // The real module build does the staging: what the release ships is what a build ships.
    feeder.releases.staging.store(true, Ordering::Relaxed);
    let build = run_build(
        editor,
        session,
        &BuildRequest { kind: BuildKind::ToFolder, target: stage.to_string_lossy().into_owned(), clean: true, remember_target: false },
        &feeder.ctx,
    )
    .await;
    feeder.releases.staging.store(false, Ordering::Relaxed);
    out.warnings.clone_from(&build.warnings);
    if !build.ok {
        return Err(if build.cancelled { cancelled() } else { build.err });
    }
    out.left_out = build.left_out;
    out.baked = build.bake_note;
    still_open(editor, session, &feeder.ctx)?;

    let job = PackJob {
        module_dir: PathBuf::from(&build.path),
        stage: stage.to_path_buf(),
        out_dir: out_dir.to_owned(),
        id: m.id.clone(),
        version: m.version.clone(),
        thumbnail: m.thumbnail.clone(),
        exec: editor.exec.clone(),
        feeder: feeder.clone(),
    };
    let packed = editor.exec.io(move || job.run()).await;
    add_warning(&mut out.warnings, &packed.warnings);
    let done = packed.result?;
    out.catalog_block = done.catalog_block;
    out.catalog_review = done.catalog_review;
    out.out_dir = out_dir.to_owned();
    out.requires_game = requires_game.to_owned();
    out.files = done.files;
    out.unpacked = done.unpacked;
    out.packed = done.packed;
    out.index = done.index;
    out.packages = done.packages;
    Ok(done.draft)
}

/// The descriptor, the job's last step: it goes last, because it is the file that vouches for
/// the other two kinds (`FinishRelease`).
async fn finish_release(editor: &Arc<Editor>, root: &Path, m: &Manifest, draft: &Draft, out: &mut PackageResult) -> Result<(), String> {
    let facts = ReleaseFacts { id: &m.id, version: &m.version, requires_game: &out.requires_game, files: out.files, unpacked: out.unpacked };
    let mut text = descriptor_head(&facts, &out.index, &out.packages);
    let root_io = root.to_path_buf();
    let tables = editor.exec.io(move || StringTables::read(&root_io)).await;
    text.push_str(&card_section(m, &tables, &draft.manifest_ref, draft.thumbnail_ref.as_ref()));

    // signed with the author key when there is one to sign with: one signature covers the card,
    // the index hash and the package hashes - every byte of the release
    let keys = AuthorKeys::of(editor);
    let path = PathBuf::from(&out.out_dir).join(descriptor_name(&m.id));
    let (body, path_io) = (text, path.clone());
    let (written, warning, key_id) = editor
        .exec
        .io(move || {
            let key = keys.state();
            let (text, warning, key_id) = if key.usable() {
                let signed = keys.sign(&body).map_err(|e| format!("the release could not be signed: {e}"))?;
                (signed, String::new(), Some(key.key_id))
            } else {
                let warning = if !key.exists {
                    "the release is not signed - the Mod Browser takes signed releases only (Mod > Submit to Mod Browser sets up the author key)\n".to_owned()
                } else if key.locked {
                    "the release is not signed: the author key is locked - give its password first\n".to_owned()
                } else {
                    format!("the release is not signed: {}\n", key.error)
                };
                (body, warning, None)
            };
            std::fs::write(&path_io, text.as_bytes()).map_err(|_| format!("can't write {}", path_io.display()))?;
            Ok::<_, String>((text, warning, key_id))
        })
        .await?;
    add_warning(&mut out.warnings, &warning);
    if let Some(id) = key_id {
        out.signed = true;
        out.key_id = id;
    }
    // a publisher checks every uploaded asset against these, the descriptor included
    out.descriptor_file = Asset { name: descriptor_name(&m.id), size: written.len() as u64, sha256: sha256_hex(written.as_bytes()) };
    out.descriptor = path.to_string_lossy().into_owned();
    editor.log.info(format!(
        "* [XMS] release {} {}: {} file(s), {} byte(s) in {} package(s) -> {}",
        m.id,
        m.version,
        out.files,
        out.packed,
        out.packages.len(),
        out.out_dir
    ));
    Ok(())
}

/// The stage's mark of a live editor: a file held open without sharing while the packaging
/// runs. A crashed packaging's stage - a full copy of the module plus the zips - is found by a
/// mark nobody holds and removed; one whose mark is held belongs to an editor at work.
struct StageLock {
    _held: Option<File>,
}

const LOCK_NAME: &str = "owner.lock";

impl StageLock {
    fn take(stage: &Path) -> Self {
        Self { _held: open_exclusive(&stage.join(LOCK_NAME)) }
    }
}

#[cfg(windows)]
fn open_exclusive(path: &Path) -> Option<File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create(true).truncate(false).share_mode(0).open(path).ok()
}

#[cfg(not(windows))]
fn open_exclusive(path: &Path) -> Option<File> {
    std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(path).ok()
}

/// Stages whose editor is gone go (`RemoveDeadStages`). A stage without the mark - the C++
/// editor's, which judges by its process id - is left to it.
// oracle: the C++ asks whether the pid in the folder name is alive; std has no such question, so
// the mark stands in for it
fn remove_dead_stages(temp: &Path, ours: &Path) {
    let Ok(listing) = std::fs::read_dir(temp) else { return };
    for entry in listing.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let is_stage = name.to_str().is_some_and(|n| n.starts_with("release_stage_"));
        if !is_stage || path == ours || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let mark = path.join(LOCK_NAME);
        if !mark.is_file() {
            continue;
        }
        // nobody holds it: its editor is gone
        if let Some(held) = open_exclusive(&mark) {
            drop(held);
            gz_build::install::remove_tree(&path);
        }
    }
}

/// One file of the staged module and its index record.
#[derive(Clone, Debug, Default)]
struct Staged {
    /// Below the staged module folder, as found on disk (`\`).
    rel: String,
    /// The same as the index states it: forward slashes, UTF-8.
    path: String,
    /// The zip entry name: `modules/<id>/` + path.
    entry: String,
    /// The lowercased entry - the order of packages and index alike.
    key: String,
    size: u64,
    written: Option<SystemTime>,
    rec: IndexRecord,
}

fn list_staged(dir: &Path, rel_dir: &str, out: &mut Vec<Staged>) -> Result<(), String> {
    let Ok(listing) = std::fs::read_dir(dir) else { return Ok(()) };
    for entry in listing.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(format!("file name the game's updater refuses (reserved name, trailing dot or space, control character): {}", entry.path().display()));
        };
        let rel = if rel_dir.is_empty() { name.to_owned() } else { format!("{rel_dir}\\{name}") };
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            list_staged(&entry.path(), &rel, out)?;
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        let md = entry.metadata().ok();
        out.push(Staged { rel, size: md.as_ref().map_or(0, std::fs::Metadata::len), written: md.and_then(|m| m.modified().ok()), ..Staged::default() });
    }
    Ok(())
}

/// MS-DOS date and time of a file's last write in local time - `FileTimeToLocalFileTime` takes
/// the bias in force now, not the one of that date, and so does this. 1980-01-01 when the time
/// does not fit (minizip reads 0 as "no date given").
fn dos_time(written: Option<SystemTime>, offset: time::UtcOffset) -> u32 {
    const NONE: u32 = 0x0021_0000;
    let Some(t) = written else { return NONE };
    let local = time::OffsetDateTime::from(t).to_offset(offset);
    let year = local.year();
    if !(1980..=2107).contains(&year) {
        return NONE;
    }
    let date = (((year - 1980) as u32) << 9) | (u32::from(u8::from(local.month())) << 5) | u32::from(local.day());
    let clock = (u32::from(local.hour()) << 11) | (u32::from(local.minute()) << 5) | (u32::from(local.second()) / 2);
    (date << 16) | clock
}

/// The file work of a packaging, on the IO pool.
struct PackJob {
    module_dir: PathBuf,
    stage: PathBuf,
    out_dir: String,
    id: String,
    version: String,
    /// `[module] thumbnail` as written.
    thumbnail: String,
    exec: Executors,
    feeder: Feeder,
}

struct Packaged {
    files: usize,
    unpacked: u64,
    packed: u64,
    index: Asset,
    packages: Vec<Asset>,
    catalog_block: Vec<String>,
    catalog_review: Vec<String>,
    draft: Draft,
}

struct PackOutcome {
    result: Result<Packaged, String>,
    /// Lines for the author, each ending with a line break.
    warnings: String,
}

impl PackJob {
    fn run(self) -> PackOutcome {
        let mut warnings = String::new();
        let result = self.pack(&mut warnings);
        PackOutcome { result, warnings }
    }

    fn pack(&self, warnings: &mut String) -> Result<Packaged, String> {
        let mut files = Vec::new();
        list_staged(&self.module_dir, "", &mut files)?;
        let prefix = format!("modules/{}/", self.id);
        let manifest_key = format!("{prefix}mod.ltx");
        let mut has_manifest = false;
        let mut unpacked = 0u64;
        for f in &mut files {
            let rel = f.rel.replace('\\', "/");
            if !package_path_ok(&rel) {
                return Err(format!("file name the game's updater refuses (reserved name, trailing dot or space, control character): {}", f.rel));
            }
            // an index record is space-separated and the path is the rest of its line
            if rel.starts_with(' ') {
                return Err(format!("file name starts with a space - the file index cannot state it safely: {}", f.rel));
            }
            f.entry = format!("{prefix}{rel}");
            f.key = f.entry.to_ascii_lowercase();
            f.path = rel;
            has_manifest |= f.key == manifest_key;
            unpacked += f.size;
        }
        if !has_manifest {
            return Err("the staged module has no mod.ltx".to_owned());
        }

        self.feeder.stage("Reading the content the way the Mod Browser does", 0, 0);
        let (catalog_block, catalog_review) = self.check_content(&files);
        // the author learns it here, not from a refused submission
        for text in catalog_block.iter().take(10) {
            warnings.push_str(&format!("the Mod Browser refuses {text}\n"));
        }
        if catalog_block.len() > 10 {
            warnings.push_str(&format!("the Mod Browser refuses {} more file(s)\n", catalog_block.len() - 10));
        }
        if let Some(first) = catalog_review.first() {
            warnings.push_str(&format!("the Mod Browser's review looks at {} place(s) in the scripts, first: {first}\n", catalog_review.len()));
        }

        // the same input always yields the same package order
        files.sort_by(|a, b| a.key.cmp(&b.key));
        let sizes: Vec<u64> = files.iter().map(|f| f.size).collect();
        let parts = split_parts(&sizes);
        if parts.len() > MAX_PACKAGES {
            return Err(format!("the module needs {} packages - a release holds {MAX_PACKAGES} at most", parts.len()));
        }

        let packages_dir = self.stage.join("packages");
        let _ = std::fs::create_dir_all(&packages_dir);
        self.feeder.stage("Packing the release", unpacked, files.len());
        let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let mut packages: Vec<Asset> = Vec::with_capacity(parts.len());
        let mut packed = 0u64;
        for (p, &(first, last)) in parts.iter().enumerate() {
            let name = package_name(&self.id, &self.version, p + 1, parts.len());
            let path = packages_dir.join(&name);
            let slice = files.get_mut(first..last).ok_or_else(|| "the package plan is damaged".to_owned())?;
            if let Err(e) = self.write_package(&path, slice, offset) {
                let _ = std::fs::remove_file(&path);
                return Err(e);
            }
            read_back(&path, slice, (p + 1) as u32)?;
            let (sha256, size) = hash_file(&path, &self.feeder)?;
            if size >= ASSET_LIMIT {
                let biggest = slice.iter().max_by_key(|f| f.size).map_or("", |f| f.rel.as_str());
                return Err(format!("package {name} is {size} bytes - GitHub refuses a release asset of 2 GiB and more (largest file inside: {biggest})"));
            }
            packed += size;
            packages.push(Asset { name, size, sha256 });
        }

        // The file index (2.2)
        let names: Vec<String> = packages.iter().map(|p| p.name.clone()).collect();
        let records: Vec<IndexRecord> = files.iter().map(|f| IndexRecord { path: f.path.clone(), size: f.size, ..f.rec.clone() }).collect();
        let index = index_text(&names, &records);
        if index.len() as u64 > INDEX_LIMIT {
            return Err(format!("the file index is {} bytes - the game reads 64 MiB at most", index.len()));
        }
        let index_asset = Asset { name: index_name(&self.id, &self.version), size: index.len() as u64, sha256: sha256_hex(index.as_bytes()) };

        self.feeder.stage("Writing the release", 0, 0);
        self.write_out(&packages_dir, &packages, &index_asset.name, &index)?;

        // The Mod Browser's card pins the manifest and the thumbnail by their hashes
        let thumbnail_path = self.thumbnail.replace('\\', "/");
        let mut draft = Draft { manifest_ref: CardFile::default(), thumbnail_ref: None };
        for f in &files {
            let card = || CardFile { path: f.path.clone(), size: f.size, sha256: f.rec.sha256.clone() };
            if f.path == "mod.ltx" {
                draft.manifest_ref = card();
            } else if !thumbnail_path.is_empty() && f.path.eq_ignore_ascii_case(&thumbnail_path) {
                draft.thumbnail_ref = Some(card());
            }
        }
        if !self.thumbnail.is_empty() {
            if draft.thumbnail_ref.is_none() {
                warnings.push_str(&format!("the thumbnail {} is not in the build - the Mod Browser shows no picture\n", self.thumbnail));
            } else {
                let picture = self.module_dir.join(self.thumbnail.replace('/', "\\"));
                let valid = std::fs::metadata(&picture).is_ok_and(|md| md.len() <= 1 << 20) && std::fs::read(&picture).is_ok_and(|b| crate::browser::valid_thumbnail(&b));
                if !valid {
                    warnings.push_str(&format!("the thumbnail {} is not a 256 x 256 DXT1/DXT5 DDS - the Mod Browser shows no picture\n", self.thumbnail));
                    draft.thumbnail_ref = None;
                }
            }
        }
        Ok(Packaged { files: files.len(), unpacked, packed, index: index_asset, packages, catalog_block, catalog_review, draft })
    }

    /// MOD_CATALOG 8 and 10.3 over what the release ships, read the way the catalog reads a
    /// release: a script whole, anything else by what it starts with. In the order the files
    /// were found.
    fn check_content(&self, files: &[Staged]) -> (Vec<String>, Vec<String>) {
        let items: Vec<(PathBuf, String, u64)> = files.iter().map(|f| (self.module_dir.join(&f.rel), f.path.clone(), f.size)).collect();
        let found: Vec<Vec<Finding>> = self.exec.par_io(items, |(src, path, size)| {
            let mut out = Vec::new();
            let Ok(mut file) = File::open(&src) else { return out };
            let head = content_head(&path, size);
            let mut data = Vec::new();
            if file.by_ref().take(head).read_to_end(&mut data).is_err() {
                return out;
            }
            let need = content_need(&data, size);
            if need > data.len() as u64 {
                let more = need - data.len() as u64;
                let _ = file.take(more).read_to_end(&mut data);
            }
            check_content(&path, &data, &mut out);
            out
        });
        let (mut block, mut review) = (Vec::new(), Vec::new());
        for f in found.into_iter().flatten() {
            if f.level == Level::Block { block.push(f.text) } else { review.push(f.text) }
        }
        (block, review)
    }

    /// Files of one package into one zip, every file hashed on its way in - the index wants the
    /// SHA-256 of the content, and the content is read once.
    fn write_package(&self, zip_path: &Path, files: &mut [Staged], offset: time::UtcOffset) -> Result<(), String> {
        let zip_text = zip_path.display().to_string();
        let cant_write = || format!("can't write {zip_text}");
        let mut w = ZipWriter::create(zip_path).map_err(|_| format!("can't create {zip_text}"))?;
        let mut i = 0usize;
        while i < files.len() {
            if self.feeder.cancelled() {
                return Err(cancelled());
            }
            let Some(f) = files.get_mut(i) else { break };
            if f.size > STREAM_ABOVE {
                let src = self.module_dir.join(&f.rel);
                let mut file = File::open(&src).map_err(|_| format!("can't read {}", f.rel))?;
                let mut sha = Sha256Stream::default();
                let feeder = &self.feeder;
                let read = w
                    .add_stream(&f.entry, dos_time(f.written, offset), f.size, &mut file, &mut |piece| {
                        sha.update(piece);
                        if feeder.cancelled() { Err(std::io::Error::other("cancelled")) } else { Ok(()) }
                    })
                    .map_err(|e| if self.feeder.cancelled() { cancelled() } else if e.kind() == std::io::ErrorKind::Other && e.to_string().contains("grew") { format!("size changed while packaging: {}", f.rel) } else { cant_write() })?;
                if read != f.size {
                    return Err(format!("size changed while packaging: {}", f.rel));
                }
                f.rec.sha256 = sha.finish_hex();
                self.feeder.file(&f.rel, f.size);
                i += 1;
                continue;
            }
            // a batch of small files: read on the IO pool, deflated on the CPU pool, written in order
            let mut j = i;
            let mut bytes = 0u64;
            while let Some(g) = files.get(j) {
                if g.size > STREAM_ABOVE || (j > i && bytes + g.size > BATCH_BYTES) {
                    break;
                }
                bytes += g.size;
                j += 1;
            }
            let batch = files.get_mut(i..j).unwrap_or(&mut []);
            let reads: Vec<(PathBuf, String, u64)> = batch.iter().map(|f| (self.module_dir.join(&f.rel), f.rel.clone(), f.size)).collect();
            let contents: Vec<Result<Vec<u8>, String>> = self.exec.par_io(reads, |(src, rel, size)| {
                let data = std::fs::read(&src).map_err(|_| format!("can't read {rel}"))?;
                if data.len() as u64 != size {
                    return Err(format!("size changed while packaging: {rel}"));
                }
                Ok(data)
            });
            let contents: Vec<Vec<u8>> = contents.into_iter().collect::<Result<_, _>>()?;
            if self.feeder.cancelled() {
                return Err(cancelled());
            }
            let jobs: Vec<(Vec<u8>, String, u32)> = contents.into_iter().zip(batch.iter()).map(|(data, f)| (data, f.entry.clone(), dos_time(f.written, offset))).collect();
            let packed: Vec<Result<(Packed, String), String>> = self.exec.par_cpu(jobs, |(data, entry, dos)| {
                let sha = sha256_hex(&data);
                let mut crc = crc32fast::Hasher::new();
                crc.update(&data);
                let compressed = deflate(&data).map_err(|_| format!("can't add {entry} to the package"))?;
                Ok((Packed { name: entry, dos_time: dos, crc: crc.finalize(), size: data.len() as u64, data: compressed }, sha))
            });
            for (f, item) in batch.iter_mut().zip(packed) {
                let (entry, sha) = item?;
                w.add(&entry).map_err(|_| cant_write())?;
                f.rec.sha256 = sha;
                self.feeder.file(&f.rel, f.size);
            }
            i = j;
        }
        w.finish().map_err(|_| cant_write())
    }

    /// Only now is the output folder touched. What an earlier packaging of this id and version
    /// left there goes first - a 3-part build followed by a 1-part one must not leave part2 and
    /// part3 beside the new release. A release is rebuilt from the module every time, so the
    /// files it replaces are deleted outright.
    fn write_out(&self, packages_dir: &Path, packages: &[Asset], index_name: &str, index: &str) -> Result<(), String> {
        let out = Path::new(&self.out_dir);
        let _ = std::fs::create_dir_all(out);
        if !out.is_dir() {
            return Err(format!("can't create the output folder {}", self.out_dir));
        }
        if let Ok(listing) = std::fs::read_dir(out) {
            for entry in listing.flatten() {
                let is_file = entry.file_type().is_ok_and(|t| !t.is_dir());
                let name = entry.file_name();
                if is_file && name.to_str().is_some_and(|n| is_release_file(n, &self.id, &self.version)) {
                    delete_outright(&entry.path());
                }
            }
        }
        for p in packages {
            if self.feeder.cancelled() {
                return Err(cancelled());
            }
            let (from, to) = (packages_dir.join(&p.name), out.join(&p.name));
            let cant = || if self.feeder.cancelled() { cancelled() } else { format!("can't write {}", to.display()) };
            if std::fs::rename(&from, &to).is_err() {
                std::fs::copy(&from, &to).map_err(|_| cant())?;
                let _ = std::fs::remove_file(&from);
            }
        }
        let path = out.join(index_name);
        std::fs::write(&path, index.as_bytes()).map_err(|_| format!("can't write {}", path.display()))
    }
}

/// The files an earlier packaging of this id and version left in the release folder - the ones
/// the next packaging replaces (`ListPackagedFiles`): what the window asks about before it
/// packages again. Over MCP they are simply replaced. Blocking: call it on the IO pool.
#[must_use]
pub fn packaged_files(project_root: &Path, out_dir: &str, id: &str, version: &str) -> Vec<String> {
    if !gz_mod::validate_id(id) || !gz_mod::validate_version(version) {
        return Vec::new();
    }
    let Ok(dir) = resolve_release_dir(project_root, out_dir, version) else { return Vec::new() };
    let Ok(listing) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = listing
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| is_release_file(n, id, version))
        .collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names
}

fn delete_outright(path: &Path) {
    if let Ok(md) = std::fs::metadata(path) {
        let mut perms = md.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    let _ = std::fs::remove_file(path);
}

/// Where every file of package `pack` really sits, asked of the FINISHED archive: the data
/// offset from the local header found there, the packed length and the method from its central
/// directory (`ReadPackageLayout`). Nothing is assumed about what the writer did.
fn read_back(zip_path: &Path, files: &mut [Staged], pack: u32) -> Result<(), String> {
    let shown = zip_path.display().to_string();
    let layout = read_layout(zip_path).map_err(|_| format!("can't read back {shown}"))?;
    let mut found = 0usize;
    for stored in &layout {
        let key = stored.name.to_ascii_lowercase();
        let at = files.partition_point(|f| f.key < key);
        let Some(f) = files.get_mut(at).filter(|f| f.entry == stored.name && f.rec.pack == 0) else {
            return Err(format!("package does not read back as written: {}", stored.name));
        };
        let encrypted = stored.flags & 1 != 0;
        if encrypted || !matches!(stored.method, 0 | 8) || stored.size != f.size || stored.data_offset == 0 {
            return Err(format!("package does not read back as written: {}", f.rel));
        }
        f.rec.offset = stored.data_offset;
        f.rec.packed = stored.packed;
        f.rec.method = u32::from(stored.method);
        f.rec.pack = pack;
        found += 1;
    }
    if found != files.len() {
        return Err(format!("package does not read back as written: {shown}"));
    }
    Ok(())
}

/// The SHA-256 and the size of a file as the disk holds it.
fn hash_file(path: &Path, feeder: &Feeder) -> Result<(String, u64), String> {
    let cant = || format!("can't hash {}", path.display());
    let mut file = File::open(path).map_err(|_| cant())?;
    let mut sha = Sha256Stream::default();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        if feeder.cancelled() {
            return Err(cancelled());
        }
        let n = file.read(&mut buf).map_err(|_| cant())?;
        if n == 0 {
            break;
        }
        sha.update(buf.get(..n).unwrap_or(&[]));
        size += n as u64;
    }
    Ok((sha.finish_hex(), size))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn feeder(rt: &gz_task::Runtime) -> Feeder {
        let jobs = gz_task::Jobs::new(rt.exec());
        let (tx, rx) = std::sync::mpsc::channel();
        jobs.start("t", None, "t", move |ctx| async move {
            let _ = tx.send(ctx);
            futures::future::pending::<()>().await;
            Ok(json!({}))
        })
        .unwrap();
        let ctx = rx.recv().unwrap();
        Feeder { releases: Arc::new(Releases::default()), ctx }
    }

    /// Three files into one package: the index states what the archive holds.
    #[test]
    fn three_files_read_back_as_the_index_says() {
        let rt = gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 2, io_threads: 2 }).unwrap();
        let base = std::env::temp_dir().join(format!("gz-release-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let module = base.join("stage").join("modules").join("od");
        std::fs::create_dir_all(module.join("gamedata").join("scripts")).unwrap();
        std::fs::write(module.join("mod.ltx"), "[module]\r\nid = od\r\nversion = 1.0.0\r\n").unwrap();
        std::fs::write(module.join("gamedata").join("scripts").join("od.script"), "function main()\n  os.execute('x')\nend\n".repeat(50)).unwrap();
        std::fs::write(module.join("\u{43c}\u{438}\u{440}.txt"), vec![7u8; 4000]).unwrap();
        let out = base.join("out");
        let job = PackJob {
            module_dir: module.clone(),
            stage: base.join("stage"),
            out_dir: out.to_string_lossy().into_owned(),
            id: "od".into(),
            version: "1.0.0".into(),
            thumbnail: String::new(),
            exec: rt.exec(),
            feeder: feeder(&rt),
        };
        let outcome = job.run();
        let done = outcome.result.unwrap();
        assert_eq!(done.files, 3);
        assert_eq!(done.packages.len(), 1);
        assert_eq!(done.packages[0].name, "od-1.0.0.zip");
        assert!(done.catalog_block.iter().any(|t| t.contains("od.script:2 uses os.execute")));
        assert!(outcome.warnings.contains("the Mod Browser refuses gamedata/scripts/od.script:2 uses os.execute\n"));

        let zip = out.join("od-1.0.0.zip");
        let bytes = std::fs::read(&zip).unwrap();
        assert_eq!(done.packages[0].sha256, sha256_hex(&bytes));
        let index = std::fs::read_to_string(out.join("od-1.0.0.files")).unwrap();
        assert_eq!(done.index.sha256, sha256_hex(index.as_bytes()));
        let mut lines = index.lines();
        assert_eq!(lines.next(), Some("xms-files 1"));
        assert_eq!(lines.next(), Some("pack od-1.0.0.zip"));
        let layout = read_layout(&zip).unwrap();
        let mut paths = Vec::new();
        for line in lines {
            let f: Vec<&str> = line.splitn(8, ' ').collect();
            assert_eq!(f[0], "file");
            let (sha, size, pack, offset, packed, method, path) = (f[1], f[2].parse::<u64>().unwrap(), f[3], f[4].parse::<u64>().unwrap(), f[5].parse::<u64>().unwrap(), f[6], f[7]);
            assert_eq!(pack, "1");
            assert_eq!(method, "8");
            // the offsets the archive itself says
            let stored = layout.iter().find(|s| s.name == format!("modules/od/{path}")).unwrap();
            assert_eq!((stored.data_offset, stored.packed, stored.size), (offset, packed, size));
            let data = &bytes[offset as usize..(offset + packed) as usize];
            let mut plain = Vec::new();
            flate2::read::DeflateDecoder::new(data).read_to_end(&mut plain).unwrap();
            assert_eq!(sha256_hex(&plain), sha);
            paths.push(path.to_owned());
        }
        // package order: the lowercased entry names
        assert_eq!(paths, ["gamedata/scripts/od.script", "mod.ltx", "\u{43c}\u{438}\u{440}.txt"]);
        assert_eq!(done.draft.manifest_ref.path, "mod.ltx");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn dos_times() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        // 2023-11-14 22:13:20 UTC
        let d = dos_time(Some(t), time::UtcOffset::UTC);
        assert_eq!(d >> 16, (43 << 9) | (11 << 5) | 14);
        assert_eq!(d & 0xFFFF, (22 << 11) | (13 << 5) | 10);
        assert_eq!(dos_time(Some(SystemTime::UNIX_EPOCH), time::UtcOffset::UTC), 0x0021_0000);
        assert_eq!(dos_time(None, time::UtcOffset::UTC), 0x0021_0000);
    }
}
