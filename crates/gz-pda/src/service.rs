//! What the PDA area keeps while a project is open: the game's reader, the layouts it read (a
//! `Query` keyed by the text, the reader's revision and the inputs), the textures W320 asks
//! about - and the project-side operations every surface shares: the layouts of the project,
//! `[pda] layout`, the build's question.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Origin, Problem, RelPath, Rev, SmolStr};
use gz_build::{BuildCheck, BuildChecks, BuildWalk};
use gz_editor::{CmdError, Editor, Executors, ProjectSession};
use gz_fmt_xml::sheets::TextureSheets;
use gz_game::{Bundled, GameLink};
use gz_lua::LuaHost;
use gz_mod::{Edit, ManifestService};
use gz_project::{AssetTemplates, ChangeKind, Filter, Project, ProjectHooks, TemplateKind};
use gz_query::Query;
use gz_vfs::GameFiles;

use crate::layout::Layout;
use crate::read::{
    BUNDLED_READER, GAME_READER, ReadError, Reader, base_name, evaluate, is_pda_path, template,
};

/// The bundled texture description of Refined's PDA sheet, for a game without it.
const BUNDLED_SHEET: &str = "configs\\pda\\ui_xms_pda.xml";

/// What a texture id may be: an id of a texture description of the game, of Refined's sheet or
/// of the project, or a file under `textures\` of the project or the game
/// (`EditorPdaInternal::TextureKnown`).
pub struct KnownTextures {
    game: Arc<GameFiles>,
    game_sheets: Option<Arc<TextureSheets>>,
    /// The bundled sheet (when the game lacks it) and the project's own descriptions.
    extra: TextureSheets,
    /// `<project>\gamedata\textures`.
    project_textures: PathBuf,
}

impl KnownTextures {
    /// Whether the id names a texture. Blocking: a project file is looked for on the disk.
    #[must_use]
    pub fn knows(&self, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        if self.extra.get(id).is_some() || self.game_sheets.as_ref().is_some_and(|s| s.get(id).is_some()) {
            return true;
        }
        let rel = id.to_ascii_lowercase();
        let file = self.project_textures.join(format!("{}.dds", rel.replace('/', "\\")));
        if std::fs::metadata(&file).is_ok_and(|m| m.is_file()) {
            return true;
        }
        self.game.exists(&format!("textures\\{rel}.dds"))
    }
}

fn text_hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn lossy(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// The reader from the linked game (when it has one) or the editor's copy. The game reads
/// THAT file; the bundled copy is the fallback for an editor with no game.
fn load_reader(files: Option<&GameFiles>, bundled: &Path) -> Result<Arc<Reader>, String> {
    if let Some(files) = files
        && files.exists(GAME_READER)
        && let Ok(bytes) = files.read(GAME_READER)
    {
        return Ok(Arc::new(Reader::new(lossy(bytes), true)));
    }
    match std::fs::read(bundled) {
        Ok(bytes) if !bytes.is_empty() => Ok(Arc::new(Reader::new(lossy(bytes), false))),
        _ => Err(format!(
            "the PDA layout reader was found neither in the linked game nor at {}",
            bundled.display().to_string().replace('/', "\\")
        )),
    }
}

/// A file of the project as the C++ `NqLua::ReadFile` read it: at most 16 MB, a UTF-8 BOM cut.
pub fn read_text(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "cannot open file".to_owned())?;
    if meta.len() > 16 << 20 {
        return Err("file too large".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|_| "read error".to_owned())?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").map(<[u8]>::to_vec).unwrap_or(bytes);
    Ok(lossy(bytes))
}

/// Two project paths name one file: `/` and `\` alike, ASCII case ignored (`SamePath`).
#[must_use]
pub fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.bytes().map(|c| if c == b'/' { b'\\' } else { c.to_ascii_lowercase() }).collect::<Vec<u8>>()
    };
    norm(a) == norm(b)
}

/// A path relative to the project root with `/` (`NqUtil::ProjectRelative`): the path as it
/// was when it is not inside.
#[must_use]
pub fn project_relative(root: &str, abs: &str) -> String {
    let n = abs.replace('/', "\\");
    let root = root.replace('/', "\\");
    let root = root.trim_end_matches('\\');
    let inside = n.len() > root.len() + 1
        && n.get(..root.len()).is_some_and(|h| h.eq_ignore_ascii_case(root))
        && n.as_bytes().get(root.len()) == Some(&b'\\');
    let rel = if inside { n.get(root.len() + 1..).unwrap_or(&n).to_owned() } else { n };
    rel.replace('\\', "/")
}

/// The open project's PDA area. Made on first use by [`PdaService::of`].
pub struct PdaService {
    exec: Executors,
    host: Arc<LuaHost>,
    reader: Query<(), Result<Arc<Reader>, String>>,
    layouts: Query<(SmolStr, SmolStr), Result<Arc<Layout>, ReadError>>,
    textures: Query<(), Arc<KnownTextures>>,
}

impl PdaService {
    /// The service of the open project, or `no active project`.
    pub fn of(editor: &Editor) -> Result<Arc<Self>, CmdError> {
        let session = editor.project()?;
        Ok(Self::of_session(editor, &session))
    }

    #[must_use]
    pub fn of_session(editor: &Editor, session: &ProjectSession) -> Arc<Self> {
        let host = editor.services.get_or_init(LuaHost::default);
        session.services.get_or_init(|| PdaService {
            exec: editor.exec.clone(),
            host,
            reader: Query::new(editor.exec.clone()),
            layouts: Query::new(editor.exec.clone()),
            textures: Query::new(editor.exec.clone()),
        })
    }

    /// The reader is read again on its next use (the preview's Reload).
    pub fn drop_reader(&self) {
        self.reader.clear();
        self.textures.clear();
    }

    /// The game's reader: the linked game's copy, the bundled one without it.
    pub async fn reader(&self, editor: &Editor) -> Result<Arc<Reader>, String> {
        let game = match GameLink::of(editor) {
            Ok(link) => link.snapshot().await.ok(),
            Err(_) => None,
        };
        let rev = game.as_ref().map_or(Rev::ZERO, |(_, r)| Rev(1).mix(*r));
        let files = game.map(|(f, _)| f);
        let bundled = Bundled::of(editor).path(BUNDLED_READER);
        let value = self.reader.fresh(&(), rev, move || load_reader(files.as_deref(), &bundled)).await;
        value.as_ref().clone()
    }

    /// Where the reader came from (`ReaderSource`): empty when there is none.
    pub async fn reader_source(&self, editor: &Editor) -> String {
        self.reader(editor).await.map(|r| r.source.clone()).unwrap_or_default()
    }

    /// The layout of a text (`EditorPda::Read`), computed on the CPU pool once per text, reader
    /// and inputs. `chunk` is the file's base name, `lang` the language of the texts.
    pub async fn evaluate(
        &self,
        editor: &Editor,
        text: Arc<str>,
        chunk: &str,
        lang: &str,
    ) -> Result<Arc<Layout>, ReadError> {
        let lang = if lang.is_empty() { "rus" } else { lang };
        let reader = match self.reader(editor).await {
            Ok(r) => r,
            Err(message) => {
                // a file that does not even parse says so first, as the C++ read it first
                let (t, c) = (text.clone(), chunk.to_owned());
                let parsed = self.exec.cpu(move || gz_luadata::parse_named(&t, &c).map(|_| ())).await;
                return Err(match parsed {
                    Err(e) => ReadError { message: e.message, line: e.line },
                    Ok(()) => ReadError { message, line: 0 },
                });
            }
        };
        let key = (SmolStr::new(chunk), SmolStr::new(lang));
        let rev = Rev(reader.script.rev()).mix(Rev(text_hash(&text)));
        let host = self.host.clone();
        let (c, l) = (chunk.to_owned(), lang.to_owned());
        let value = self
            .layouts
            .fresh(&key, rev, move || evaluate(&host, &reader, &text, &c, &l).map(Arc::new))
            .await;
        value.as_ref().clone()
    }

    /// What W320 knows, `None` when no game is linked: the C++ checked textures only then.
    async fn known_textures(&self, editor: &Editor, session: &ProjectSession) -> Option<Arc<KnownTextures>> {
        let link = GameLink::of(editor).ok()?;
        let (files, game_rev) = link.snapshot().await.ok()?;
        let project = session.services.get::<Project>()?;
        let rev = game_rev.mix(project.tree.rev());
        if let Some(found) = self.textures.peek_at(&(), rev) {
            return Some(found.as_ref().clone());
        }
        let game_sheets = link.texture_sheets().await.ok();
        let need_bundled = !game_sheets.as_ref().is_some_and(|s| s.get("ui_xms_pda_white").is_some());
        let bundled = need_bundled.then(|| Bundled::of(editor).path(BUNDLED_SHEET));
        let root = session.root.clone();
        let exec = self.exec.clone();
        let built = self
            .textures
            .fresh(&(), rev, move || {
                let mut texts: Vec<String> = Vec::new();
                if let Some(path) = bundled
                    && let Ok(t) = read_text(&path)
                {
                    texts.push(t);
                }
                let dir = root.join("gamedata").join("configs").join("ui").join("textures_descr");
                let mut descr: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .map(|rd| {
                        rd.filter_map(Result::ok)
                            .map(|e| e.path())
                            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")))
                            .collect()
                    })
                    .unwrap_or_default();
                descr.sort();
                let project_texts: Vec<String> = exec.io_pool().install(|| {
                    use rayon::prelude::*;
                    descr.par_iter().filter_map(|p| read_text(p).ok()).collect()
                });
                texts.extend(project_texts);
                let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                Arc::new(KnownTextures {
                    game: files,
                    game_sheets,
                    extra: TextureSheets::build(&refs),
                    project_textures: root.join("gamedata").join("textures"),
                })
            })
            .await;
        Some(built.as_ref().clone())
    }

    /// The textures of a layout neither the linked game nor the project has (W320); nothing
    /// when no game is linked.
    pub async fn missing_textures(
        &self,
        editor: &Editor,
        session: &ProjectSession,
        layout: &Layout,
    ) -> Vec<String> {
        let Some(known) = self.known_textures(editor, session).await else { return Vec::new() };
        let mut seen: Vec<String> = Vec::new();
        for it in &layout.items {
            for t in [&it.texture, &it.icon] {
                if !t.is_empty() && !seen.contains(t) {
                    seen.push(t.clone());
                }
            }
        }
        self.exec.io(move || seen.into_iter().filter(|t| !known.knows(t)).collect()).await
    }

    /// The `.pdaasset` files of the project the build ships, project-relative with `\`, sorted
    /// (`EditorPda::List`).
    pub async fn list(&self, editor: &Editor) -> Result<Vec<String>, CmdError> {
        let (session, project) = Project::of(editor)?;
        let manifest = ManifestService::of_session(editor, &session).await?;
        project.tree.refresh().await;
        let tree = project.tree.snapshot();
        let mut files: Vec<String> = tree
            .files_with_extension("pdaasset")
            .iter()
            .map(RelPath::to_backslash)
            .filter(|rel| !manifest.outside_build(rel))
            .collect();
        files.sort();
        Ok(files)
    }

    /// `[pda] layout` of `mod.ltx`, as written (`""`: the module leaves the PDA alone).
    pub async fn used_layout(&self, editor: &Editor) -> String {
        let Ok(manifest) = ManifestService::of(editor).await else { return String::new() };
        let _ = manifest.sync().await;
        manifest.snapshot().manifest().map(|m| m.pda_layout.clone()).unwrap_or_default()
    }

    /// Sets (`rel`) or clears (`""`) `[pda] layout` and saves `mod.ltx` at once.
    pub async fn use_layout(&self, editor: &Editor, rel: &str, origin: Origin) -> Result<(), String> {
        let (session, _) = Project::of(editor).map_err(|_| "no active project".to_owned())?;
        let manifest = ManifestService::of_session(editor, &session).await.map_err(|e| e.message)?;
        if !manifest.snapshot().exists() {
            return Err("the project has no mod.ltx: it is no module".to_owned());
        }
        let edit = self.layout_edit(editor, rel).await?;
        manifest
            .edit(vec![edit], origin)
            .await
            .map_err(|e| if e.code == "io" { "mod.ltx cannot be written".to_owned() } else { e.message })?;
        Ok(())
    }

    /// The edit of `[pda] layout` that names `rel` (`""` clears it), after the checks: a
    /// `.pdaasset` of the project that exists and that the build ships.
    pub async fn layout_edit(&self, editor: &Editor, rel: &str) -> Result<Edit, String> {
        let (session, project) = Project::of(editor).map_err(|_| "no active project".to_owned())?;
        let manifest = ManifestService::of_session(editor, &session).await.map_err(|e| e.message)?;
        let rel = rel.trim();
        let mut value = String::new();
        if !rel.is_empty() {
            let abs = gz_project::ops::resolve_in_project(project.root(), rel)?;
            if !is_pda_path(&abs) {
                return Err("a PDA layout is a .pdaasset file".to_owned());
            }
            let check = abs.clone();
            if !self.exec.io(move || Path::new(&check).is_file()).await {
                return Err(format!("{rel} does not exist"));
            }
            value = project_relative(project.root(), &abs);
            if manifest.outside_build(&value) {
                return Err(format!(
                    "{value} is left out of the build ([exclude] or tooling) - the game would never see it"
                ));
            }
        }
        // one spelling in mod.ltx, whatever separators the caller used
        Ok(Edit::optional("pda", "layout", &value.replace('/', "\\")))
    }
}

/// The lines of the build's PDA check, as the C++ wrote them (each a whole line).
fn line(severity_error: bool, text: String) -> Problem {
    if severity_error { Problem::error("pda", text) } else { Problem::warning("pda", text) }
}

/// The build's question (`ValidateForBuild`): the layout the module uses, its E-problems as
/// errors, the rest as warnings. A layout the build leaves out, a missing file and one that
/// does not parse are errors too.
pub async fn validate_for_build(editor: &Arc<Editor>, session: &Arc<ProjectSession>) -> Vec<Problem> {
    let Ok(manifest) = ManifestService::of_session(editor, session).await else { return Vec::new() };
    let _ = manifest.sync().await;
    let snap = manifest.snapshot();
    let Some(rel) = snap.manifest().map(|m| m.pda_layout.clone()).filter(|r| !r.is_empty()) else {
        return Vec::new();
    };
    let abs = session.root.join(rel.replace('/', "\\"));
    let check = abs.clone();
    let (exists, text) = editor.exec.io(move || (check.is_file(), read_text(&check))).await;
    if !exists {
        return vec![line(true, format!("[pda] layout = {rel}: the file does not exist"))];
    }
    if manifest.outside_build(&rel) {
        return vec![line(true, format!("[pda] layout = {rel}: the build leaves the file out"))];
    }
    let service = PdaService::of_session(editor, session);
    let chunk = base_name(&abs.to_string_lossy());
    let layout = match text {
        Err(e) => return vec![line(true, format!("{rel}: {e}"))],
        Ok(t) => service.evaluate(editor, Arc::from(t), &chunk, "rus").await,
    };
    let layout = match layout {
        Ok(l) => l,
        Err(e) => {
            let text = if e.line > 0 {
                format!("{rel}:{}: {}", e.line, e.message)
            } else {
                format!("{rel}: {}", e.message)
            };
            return vec![line(true, text)];
        }
    };
    let mut out: Vec<Problem> = layout
        .problems
        .iter()
        .map(|p| {
            line(
                p.is_error(),
                format!(
                    "{rel} {} {}: {}",
                    p.code,
                    if p.path.is_empty() { "layout" } else { p.path.as_str() },
                    p.message
                ),
            )
        })
        .collect();
    for t in service.missing_textures(editor, session, &layout).await {
        out.push(line(
            false,
            format!("{rel} W320: texture '{t}' is neither in the linked game nor in the project"),
        ));
    }
    out
}

struct PdaCheck;

impl BuildCheck for PdaCheck {
    fn stage(&self) -> &'static str {
        crate::kind::STAGE
    }

    fn headline(&self) -> Option<&'static str> {
        Some("The PDA layout has errors:")
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        session: Arc<ProjectSession>,
        _walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move { validate_for_build(&editor, &session).await })
    }
}

/// Hangs the area on the editor: the build's check, the template of `file.create kind=pda`
/// (`NewPda` gets the id `new_pda`, `NewPda2` gets `new_pda2`, titled `New PDA`), the PDA kind
/// of `gz-asset` with its manifest check, and - on every open project, through its watcher -
/// the first layout that appears in a module that uses none made the module's layout, and the
/// used layout's file going away clearing `[pda] layout`.
pub fn install(editor: &Editor) {
    BuildChecks::of(editor).add(Arc::new(PdaCheck));
    // `mod.set set={"pda": {"layout": ...}}`
    gz_mod::ManifestArgs::of(editor).provide("pda.layout", |cx: gz_mod::ArgCx| {
        Box::pin(async move {
            let service = PdaService::of(&cx.editor).map_err(|e| e.message)?;
            service.layout_edit(&cx.editor, &cx.value).await.map(|e| vec![e])
        })
    });
    AssetTemplates::of(editor).provide(TemplateKind::Pda, |cx| {
        let id = if cx.n <= 1 { "new_pda".to_owned() } else { format!("new_pda{}", cx.n) };
        Ok(template(&id, "New PDA").into_bytes())
    });
    let kinds = gz_asset::Kinds::of(editor);
    kinds.add(crate::kind::kind());
    kinds.covers_build_stage(crate::kind::STAGE);
    kinds.add_manifest_check(crate::kind::manifest_check);
    ProjectHooks::of(editor).on_open(|ed, _session| {
        let Ok((_, project)) = Project::of(ed) else { return };
        let mut rx = project.tree.changes().subscribe(Filter::Extensions(vec!["pdaasset".into()]));
        let (ed, weak) = (ed.clone(), Arc::downgrade(&project));
        ed.exec.clone().spawn(async move {
            while let Some(c) = rx.recv().await {
                if weak.upgrade().is_none() {
                    break;
                }
                let Ok((session, _)) = Project::of(&ed) else { break };
                let service = PdaService::of_session(&ed, &session);
                let used = service.used_layout(&ed).await;
                match c.kind {
                    ChangeKind::Created if used.is_empty() => {
                        let _ = service.use_layout(&ed, c.path.as_str(), Origin::Mcp).await;
                    }
                    // the module does not keep naming a layout that is gone
                    ChangeKind::Removed if same_path(&used, c.path.as_str()) => {
                        let _ = service.use_layout(&ed, "", Origin::Mcp).await;
                    }
                    _ => {}
                }
            }
        });
    });
}
