//! What a Text tab holds of its file and does with it off the UI thread: the read (and its
//! encoding and line endings), the follow through the project's watcher and the merge of a change
//! on disk into unsaved edits, the save in the file's own bytes, and the checks of the text.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gz_base::{Problem, RelPath};
use gz_editor::{CmdError, Editor};
use gz_project::{Filter, Project};

use gz_text::{Encoding, Eol, TextFile, TextService};
use gz_ui::code::CodeLang;
use parking_lot::Mutex;

/// Where a tab's file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file of the open project, project-relative with `/`.
    Project(String),
    /// A file of the linked game, by its path there (`scripts\xr_logic.script`): read-only.
    Game(String),
}

impl Source {
    /// What a tab's request names: `game:<path>` is the game's, anything else the project's.
    #[must_use]
    pub fn parse(path: &str, game: bool) -> Self {
        match path.strip_prefix("game:") {
            Some(p) => Self::Game(p.replace('/', "\\")),
            None if game => Self::Game(path.replace('/', "\\")),
            None => Self::Project(path.replace('\\', "/").trim_start_matches('/').to_owned()),
        }
    }

    /// The tab's id: `text:<rel>`, `text:game:<path>`.
    #[must_use]
    pub fn tab_id(&self) -> String {
        match self {
            Self::Project(rel) => format!("text:{rel}"),
            Self::Game(p) => format!("text:game:{}", p.replace('\\', "/")),
        }
    }

    /// What the session reopens it from.
    #[must_use]
    pub fn restore(&self) -> String {
        match self {
            Self::Project(rel) => rel.clone(),
            Self::Game(p) => format!("game:{p}"),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        let p = match self {
            Self::Project(rel) => rel.as_str(),
            Self::Game(p) => p.as_str(),
        };
        p.rsplit(['/', '\\']).next().unwrap_or(p)
    }

    #[must_use]
    pub fn shown(&self) -> String {
        match self {
            Self::Project(rel) => rel.clone(),
            Self::Game(p) => p.replace('\\', "/"),
        }
    }

    #[must_use]
    pub fn read_only(&self) -> bool {
        matches!(self, Self::Game(_))
    }
}

/// A tab's file and its text.
pub struct Model {
    pub source: Source,
    pub lang: CodeLang,
    /// The file as it was read or written last (encoding, line endings, its lines).
    pub file: Option<TextFile>,
    /// The text as the file holds it: what a merge starts from.
    pub disk: String,
    /// The text the tab shows and edits.
    pub text: String,
    /// The encoding and the line endings a save writes (the file's until More changes them).
    pub encoding: Encoding,
    pub eol: Eol,
    pub loading: bool,
    /// Why the file could not be read.
    pub read_error: Option<String>,
    /// What the file on disk did that the tab's edits keep out of view.
    pub disk_note: String,
    /// The file is gone from the disk.
    pub gone: bool,
    /// The tab is to close (its file went, with nothing unsaved).
    pub closed: bool,
    pub problems: Vec<Problem>,
    /// The hash of the text the problems are of.
    pub checked: u64,
    pub checking: bool,
    pub save_error: Option<CmdError>,
    pub saving: bool,
}

impl Model {
    #[must_use]
    pub fn new(source: Source) -> Self {
        let lang = lang_of(source.name());
        Self {
            source,
            lang,
            file: None,
            disk: String::new(),
            text: String::new(),
            encoding: Encoding::Utf8,
            eol: Eol::Lf,
            loading: true,
            read_error: None,
            disk_note: String::new(),
            gone: false,
            closed: false,
            problems: Vec::new(),
            checked: 0,
            checking: false,
            save_error: None,
            saving: false,
        }
    }

    /// Whether a save would write something: the text or what it is written as changed.
    #[must_use]
    pub fn dirty(&self) -> bool {
        if self.source.read_only() || self.file.is_none() {
            return false;
        }
        let f = self.file.as_ref();
        self.text != self.disk
            || f.is_some_and(|f| f.encoding != self.encoding || f.eol != self.eol)
            || (self.gone && !self.text.is_empty())
    }
}

/// The language of a file by its name: Markdown, Lua, LTX, ...; a project's `mod.ltx` is LTX.
#[must_use]
pub fn lang_of(name: &str) -> CodeLang {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    CodeLang::from_extension(ext)
}

/// The hash of a text: what the problems and the caches of a text are kept by.
#[must_use]
pub fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    text.hash(&mut h);
    h.finish()
}

pub type Shared = Arc<Mutex<Model>>;

/// Asks the window for a frame: what arrived off the UI thread is drawn.
fn wake(editor: &Editor) {
    editor.exec.wake().ping();
}

fn abs_of(editor: &Editor, rel: &str) -> Option<PathBuf> {
    let session = editor.project_opt()?;
    let rel = RelPath::new(rel).ok()?;
    Some(session.fs(&rel))
}

/// Reads a file of the project as text: its size checked first, its bytes decoded.
fn read_project(abs: &Path, name: &str) -> Result<TextFile, String> {
    let meta = std::fs::metadata(abs)
        .map_err(|e| format!("{} could not be read: {}", abs.display(), gz_base::text::io_reason(&e)))?;
    if meta.len() > gz_text::MAX_BYTES {
        return Err(gz_text::ReadError::TooLarge(meta.len()).to_string());
    }
    let bytes = std::fs::read(abs)
        .map_err(|e| format!("{} could not be read: {}", abs.display(), gz_base::text::io_reason(&e)))?;
    gz_text::decode(&bytes, name).map_err(|e| e.to_string())
}

/// The tab's first read, and each read again when the file changed with nothing unsaved.
pub async fn load(editor: &Arc<Editor>, m: &Shared) {
    let (source, name) = {
        let g = m.lock();
        (g.source.clone(), g.source.name().to_owned())
    };
    let read = match &source {
        Source::Project(rel) => {
            let Some(abs) = abs_of(editor, rel) else {
                let mut g = m.lock();
                g.loading = false;
                g.read_error = Some(format!("{rel} is outside the project"));
                return;
            };
            editor.exec.io(move || read_project(&abs, &name)).await
        }
        Source::Game(path) => match editor.project() {
            Ok(session) => TextService::of_session(editor, &session).read_game(editor, path).await,
            Err(e) => Err(e.message),
        },
    };
    {
        let mut g = m.lock();
        g.loading = false;
        match read {
            Ok(f) => {
                g.read_error = None;
                g.gone = false;
                g.disk_note.clear();
                g.disk.clone_from(&f.text);
                g.text.clone_from(&f.text);
                g.encoding = f.encoding;
                g.eol = f.eol;
                g.file = Some(f);
            }
            Err(e) => g.read_error = Some(e),
        }
    }
    wake(editor);
}

/// Follows the tab's file through the project's watcher while `generation` is the tab's.
pub fn follow(editor: &Arc<Editor>, m: &Shared, generation: Arc<AtomicU64>) {
    let Source::Project(rel) = m.lock().source.clone() else { return };
    let Ok((_, project)) = Project::of(editor) else { return };
    let Ok(rel) = RelPath::new(&rel) else { return };
    let mine = generation.fetch_add(1, Ordering::AcqRel) + 1;
    let mut rx = project.tree.changes().subscribe(Filter::Path(rel));
    let (ed, weak) = (editor.clone(), Arc::downgrade(m));
    let _task = editor.exec.spawn(async move {
        while rx.recv().await.is_some() {
            let Some(m) = weak.upgrade() else { break };
            if generation.load(Ordering::Acquire) != mine {
                break;
            }
            changed_on_disk(&ed, &m).await;
        }
    });
}

/// The file changed on disk: read again when the tab holds nothing unsaved, merged line by line
/// into the tab's edits otherwise; gone, the tab closes or keeps its edits for Save.
async fn changed_on_disk(editor: &Arc<Editor>, m: &Shared) {
    let (rel, name) = {
        let g = m.lock();
        let Source::Project(rel) = g.source.clone() else { return };
        (rel, g.source.name().to_owned())
    };
    let Some(abs) = abs_of(editor, &rel) else { return };
    let probe = abs.clone();
    let now =
        editor.exec.io(move || if probe.is_file() { Some(read_project(&probe, &name)) } else { None }).await;
    // what the change means, decided under the lock; a merge is made after it
    let merge = {
        let mut g = m.lock();
        match now {
            None => {
                if g.dirty() || g.text != g.disk {
                    g.gone = true;
                    g.disk_note = format!("{} is gone from the disk. Save writes it again.", g.source.name());
                } else {
                    g.closed = true;
                }
                None
            }
            Some(Err(_)) => None,
            Some(Ok(f)) if f.text == g.disk => {
                // the editor's own write, or a touch that changed nothing
                g.gone = false;
                None
            }
            Some(Ok(f)) if g.text == g.disk => {
                g.gone = false;
                g.disk.clone_from(&f.text);
                g.text.clone_from(&f.text);
                g.encoding = f.encoding;
                g.eol = f.eol;
                g.file = Some(f);
                g.disk_note.clear();
                None
            }
            Some(Ok(f)) => {
                g.gone = false;
                Some((g.disk.clone(), g.text.clone(), f))
            }
        }
    };
    if let Some((base, mine, f)) = merge {
        // merged off the lock: the UI thread draws the text under it
        let (theirs, typed) = (f.text.clone(), mine.clone());
        let (merged, conflicts) = editor.exec.cpu(move || gz_text::merge3(&base, &typed, &theirs)).await;
        let mut g = m.lock();
        // the author typed meanwhile: the next change merges again
        if g.text != mine {
            return;
        }
        g.text = merged;
        g.disk.clone_from(&f.text);
        g.file = Some(f);
        g.disk_note = if conflicts == 0 {
            String::new()
        } else {
            // the words of the catalog's template, so the window shows it translated
            format!(
                "Changed on disk too: {conflicts} {}. The tab keeps its own; Take Theirs takes the file's.",
                if conflicts == 1 { "place" } else { "places" }
            )
        };
    }
    wake(editor);
}

/// Take Theirs: the tab's text is the file's again.
pub fn take_theirs(m: &Shared) -> String {
    let mut g = m.lock();
    g.text = g.disk.clone();
    g.disk_note.clear();
    if let Some((e, l)) = g.file.as_ref().map(|f| (f.encoding, f.eol)) {
        g.encoding = e;
        g.eol = l;
    }
    g.text.clone()
}

/// Writes the text as it is now in the file's encoding and line endings (or the ones More picked);
/// the tab is clean once its file holds it. What refuses the save is the save's answer.
pub async fn save(editor: &Arc<Editor>, m: &Shared) -> Result<(), CmdError> {
    let (rel, text, file, encoding, eol, name) = {
        let mut g = m.lock();
        let Source::Project(rel) = g.source.clone() else {
            return Err(CmdError::new("refused", "A file of the game: the editor never writes it."));
        };
        g.saving = true;
        g.save_error = None;
        let file = g.file.clone().unwrap_or_else(|| TextFile::new_file(g.source.name()));
        let name = g.source.name().to_owned();
        // an ASCII file that gains its first other character goes in the encoding the game reads it in
        let encoding =
            if file.encoding == g.encoding { file.encoding_for(&name, &g.text) } else { g.encoding };
        (rel, g.text.clone(), file, encoding, g.eol, name)
    };
    let written = async {
        let bytes = file.encode(&text, encoding, eol).map_err(|e| {
            CmdError::new(
                "encoding",
                format!("{e}: save it in UTF-8 (More > Encoding) or write the character another way."),
            )
        })?;
        let path =
            RelPath::new(&rel).map_err(|_| CmdError::invalid(format!("{rel} is no path of the project")))?;
        gz_asset::write_bytes(editor, &path, bytes.clone()).await?;
        gz_text::decode(&bytes, &name).map_err(|e| CmdError::internal(e.to_string()))
    }
    .await;
    let mut g = m.lock();
    g.saving = false;
    match written {
        Ok(f) => {
            g.disk.clone_from(&text);
            g.gone = false;
            g.disk_note.clear();
            g.encoding = f.encoding;
            g.eol = f.eol;
            g.file = Some(f);
            drop(g);
            wake(editor);
            Ok(())
        }
        Err(e) => {
            g.save_error = Some(e.clone());
            drop(g);
            wake(editor);
            Err(e)
        }
    }
}

/// The problems of the text as it is now, worked out on the CPU pool; dropped when the text moved
/// on meanwhile.
pub async fn check(editor: &Arc<Editor>, m: &Shared) {
    let (text, name, want, rel) = {
        let mut g = m.lock();
        g.checking = true;
        let rel = match &g.source {
            Source::Project(rel) => RelPath::new(rel).ok(),
            Source::Game(_) => None,
        };
        (g.text.clone(), g.source.name().to_owned(), hash(&g.text), rel)
    };
    let game = rel.is_none();
    let found = editor.exec.cpu(move || gz_text::check_text(&name, &text)).await;
    let mut g = m.lock();
    g.checking = false;
    if hash(&g.text) == want {
        let mut problems: Vec<Problem> = Vec::new();
        for p in found {
            // a file of the game is no code of the module: only what does not read is said of it
            if game && p.severity != gz_base::Severity::Error {
                continue;
            }
            // the scan finds a thing twice on a line where it is written twice
            if problems.iter().any(|q| q.line == p.line && q.message == p.message) {
                continue;
            }
            problems.push(match &rel {
                Some(r) if p.file.is_none() => p.in_file(r.clone()),
                _ => p,
            });
        }
        g.problems = problems;
        g.checked = want;
    }
    drop(g);
    wake(editor);
}
