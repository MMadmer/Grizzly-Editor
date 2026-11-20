//! What the media area hangs on the editor: the three kinds of `gz-asset`, their `file.create`
//! templates, the provider of the scene's model assets, the index made when a project opens;
//! and the commands `asset.import`, `asset.export`, `model.info`, `model.list`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_asset::Kinds;
use gz_base::RelPath;
use gz_editor::{CmdError, Editor, Origin, Registry, Reply, Spec};
use gz_project::asset_kind::MediaKind;
use gz_project::{AssetTemplates, Project, ProjectHooks, TemplateKind};
use gz_scene::SceneService;
use gz_scene::library::{Library, LibraryObject, ProviderMakers};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::import;
use crate::kind;
use crate::resolve;
use crate::service::{Media, provider};

/// Hangs the area on the editor.
pub fn install(editor: &Editor) {
    let kinds = Kinds::of(editor);
    for k in MediaKind::ALL {
        kinds.add(kind::ops(k));
    }
    let templates = AssetTemplates::of(editor);
    templates.provide(TemplateKind::Model, |_| {
        Ok(b"-- A model: its geometry from a source beside it (source = \"wall.glb\"), or written here.\n\
return {\n  mesh = 1,\n  surfaces = {\n    { name = \"plank\", texture = \"wood\\\\wood_table_01\", material = \"materials\\\\wooden_board\" },\n  },\n  parts = {\n    { shape = \"box\", size = { 1, 1, 1 }, surface = \"plank\" },\n  },\n}\n"
            .to_vec())
    });
    templates.provide(TemplateKind::Texture, |cx| {
        Ok(format!("-- A texture: put its picture beside this file.\nreturn {{\n  tex = 1,\n  source = \"{}.png\",\n}}\n", cx.stem)
            .into_bytes())
    });
    templates.provide(TemplateKind::Sound, |cx| {
        Ok(format!(
            "-- A sound: put its .wav beside this file.\nreturn {{\n  snd = 1,\n  source = \"{}.wav\",\n}}\n",
            cx.stem
        )
        .into_bytes())
    });
    ProviderMakers::of(editor).add(Arc::new(|ed, root: &Path| {
        Some(provider(root.to_path_buf(), crate::GameModels::reader(ed, root)))
    }));
    ProjectHooks::of(editor).on_open(|ed, _session| {
        let ed = ed.clone();
        ed.exec.clone().spawn(async move {
            if let Ok(m) = Media::of(&ed).await {
                m.index().await;
            }
        });
    });
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportParams {
    /// The files to bring in: absolute, or project-relative. A model (`.glb`, `.gltf`, `.obj`,
    /// `.object`, `.ogf`), a picture (`.png`, `.jpg`, `.tga`, `.bmp`, `.dds`), a texture
    /// description alone (`.thm`: of a texture of the game) or a sound (`.wav`, `.ogg`). A file of
    /// the project stays where it lies and gets its asset beside it; a file of the game's own
    /// formats brings what it says of itself (a `.thm` beside a picture, an `.ogg`'s parameters,
    /// a compiled model of a `gamedata\meshes` tree is dynamic) and keeps its game name - the one
    /// of where it lies under a `gamedata` tree, else of where it lands. A picture the game cannot
    /// load (no DDS, a name it never asks for) gets no asset: it stays, or is copied in, as a file.
    pub from: Vec<String>,
    /// The project folder the files go into: one from outside, or a file of the project from
    /// another folder, is copied there. Left out: a file of the project stays where it lies, one
    /// from outside goes into `models`, `textures` or `sounds` by kind.
    #[serde(default)]
    pub to: Option<String>,
    /// Replace files of the project that are there already (they are lost).
    #[serde(default)]
    pub overwrite: bool,
    /// Remove each `.thm` of the project its asset now says all the game reads of (what only the
    /// X-Ray SDK reads of it - its thumbnail, how its picture was made - is lost with it). Left
    /// out, the `.thm` stays beside its picture, read by nobody: the game reads the asset, and
    /// the build ships no `.thm`.
    #[serde(default)]
    pub remove_descriptions: bool,
    /// Answer what the import would write, keep and remove, and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

fn rel_param(root: &Path, p: &str) -> Result<RelPath, CmdError> {
    let path = Path::new(p);
    if path.is_absolute() {
        return RelPath::from_fs(root, path)
            .ok_or_else(|| CmdError::invalid(format!("{p} is outside the project")));
    }
    RelPath::new(p).map_err(|e| CmdError::invalid(format!("{p}: {e}")))
}

async fn asset_import(ed: Arc<Editor>, p: ImportParams, _o: Origin) -> Reply {
    let (session, project) = Project::of(&ed)?;
    let root = session.root.clone();
    if p.from.is_empty() {
        return Err(CmdError::invalid("from names no file"));
    }
    let to = p.to.as_deref().filter(|t| !t.trim().is_empty()).map(|t| rel_param(&root, t)).transpose()?;
    let from: Vec<PathBuf> = p
        .from
        .iter()
        .map(|f| if Path::new(f).is_absolute() { PathBuf::from(f) } else { root.join(f) })
        .collect();
    let module = Media::of(&ed).await.map(|m| m.latest().map(|i| i.module.clone())).ok().flatten();
    let r = root.clone();
    let plan = ed
        .exec
        .io(move || {
            let module = module.unwrap_or_else(|| crate::files::module_id(&r));
            import::plan(&from, to.as_ref(), &module, &r)
        })
        .await;
    // the descriptions the assets now say: removed when asked, else they stay and are named
    let described: Vec<String> = plan.removes.iter().map(ToString::to_string).collect();
    let removes = if p.remove_descriptions { plan.removes.clone() } else { Vec::new() };
    let left: Vec<&String> = if p.remove_descriptions { Vec::new() } else { described.iter().collect() };
    if plan.writes.is_empty() && removes.is_empty() {
        if !plan.kept.is_empty() {
            return Ok(
                json!({ "assets": [], "copied": [], "kept": plan.kept, "removed": [], "descriptions": left, "problems": plan.problems }),
            );
        }
        return Err(
            CmdError::new("refused", "nothing to import").with_data(json!({ "problems": plan.problems }))
        );
    }
    let guard = project.write_guard(None);
    let mut there = Vec::new();
    for (rel, _) in &plan.writes {
        let abs = rel.to_fs(&root);
        guard.check(&abs).map_err(|e| CmdError::new("refused", e.0))?;
        if abs.exists() {
            there.push(rel.to_string());
        }
    }
    if !there.is_empty() && !p.overwrite {
        return Err(CmdError::new(
            "exists",
            format!(
                "{} of the files are there already: asset.import overwrite=true replaces them",
                there.len()
            ),
        )
        .with_data(json!({ "files": there })));
    }
    for rel in &removes {
        guard.check(&rel.to_fs(&root)).map_err(|e| CmdError::new("refused", e.0))?;
    }
    let removed: Vec<String> = removes.iter().map(ToString::to_string).collect();
    if p.dry_run {
        let writes: Vec<String> = plan.writes.iter().map(|(r, _)| r.to_string()).collect();
        return Ok(
            json!({ "dry_run": true, "assets": plan.assets, "copied": plan.copied, "kept": plan.kept, "removed": removed, "descriptions": left, "writes": writes, "problems": plan.problems }),
        );
    }
    import::write(&ed, &project, &root, plan.writes.clone(), removes).await.map_err(CmdError::io)?;
    Ok(
        json!({ "assets": plan.assets, "copied": plan.copied, "kept": plan.kept, "removed": removed, "descriptions": left, "problems": plan.problems }),
    )
}

/// One path or several.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Paths {
    One(String),
    Many(Vec<String>),
}

impl Paths {
    fn list(&self) -> Vec<String> {
        match self {
            Paths::One(p) => vec![p.clone()],
            Paths::Many(v) => v.clone(),
        }
        .into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportParams {
    /// The media assets (`*.meshasset`, `*.texasset`, `*.sndasset`), project-relative: one or a
    /// list.
    pub path: Paths,
    /// The folder of this computer they are written into, absolute; made when it is not there.
    /// Never inside the project or a game.
    pub to: String,
    /// Replace files that are there already (they are lost).
    #[serde(default)]
    pub overwrite: bool,
    /// Answer what would be written, and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// A path with its `.` and `..` worked out by their words.
fn lexical(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A path as the file system says it is from the nearest part of it that is there: links and
/// short names resolved, what is not there yet added back.
fn real(p: &Path) -> PathBuf {
    let flat = lexical(p);
    let (mut head, mut tail) = (flat.clone(), Vec::new());
    loop {
        if let Ok(c) = std::fs::canonicalize(&head) {
            let s = c.to_string_lossy();
            let mut out = PathBuf::from(s.strip_prefix(r"\\?\").map_or_else(|| s.to_string(), str::to_owned));
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        let Some(name) = head.file_name().map(std::ffi::OsStr::to_owned) else { return flat };
        tail.push(name);
        if !head.pop() {
            return flat;
        }
    }
}

async fn asset_export(ed: Arc<Editor>, p: ExportParams, _o: Origin) -> Reply {
    let (session, project) = Project::of(&ed)?;
    let root = session.root.clone();
    let wanted = p.path.list();
    if wanted.is_empty() {
        return Err(CmdError::invalid("path names no asset"));
    }
    let to = lexical(Path::new(p.to.trim()));
    if !to.is_absolute() {
        return Err(CmdError::invalid("to is a folder of this computer, written whole (D:/export)"));
    }
    let (r, t) = (real(&root), real(&to));
    let same = |a: &Path, b: &Path| {
        a.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches(['\\', '/']))
    };
    if same(&root, &to) || same(&r, &t) || crate::files::inside(&root, &to) || crate::files::inside(&r, &t) {
        return Err(CmdError::new("refused", "an export goes out of the project: to is a folder inside it"));
    }
    project.write_guard(None).check(&to).map_err(|e| CmdError::new("refused", e.0))?;
    let media = Media::of(&ed).await?;
    let index = media.fresh_index().await;
    let models = crate::GameModels::reader(&ed, &root);
    let files = project.game_snapshot().ok().map(|(f, _)| f);
    let textures = move |name: &str| files.as_ref()?.read(&format!("textures\\{name}.dds")).ok();
    // the files of every asset, each name once
    let mut outs: Vec<(String, crate::export::Out)> = Vec::new();
    for w in &wanted {
        let rel = rel_param(&root, w)?;
        let Some(entry) = index.at(rel.as_str()) else {
            return Err(CmdError::new("not_found", format!("{rel} is no media asset of the project")));
        };
        let r = root.clone();
        let entry = entry.clone();
        let got = ed
            .exec
            .io(move || crate::export::files_of(&entry, |t| t.to_fs(&r).is_file()))
            .await
            .map_err(|why| CmdError::new("refused", why))?;
        for o in got {
            if let Some((other, _)) =
                outs.iter().find(|(_, x)| x.name.eq_ignore_ascii_case(&o.name) && x.from != o.from)
            {
                return Err(CmdError::new(
                    "refused",
                    format!("{rel} and {other} both write {}: export them into folders of their own", o.name),
                ));
            }
            if !outs.iter().any(|(_, x)| x.from == o.from) {
                outs.push((rel.to_string(), o));
            }
        }
    }
    let mut there = Vec::new();
    let mut targets = Vec::new();
    for (_, o) in &outs {
        let Some(abs) = crate::export::target(&to, &o.name) else {
            return Err(CmdError::new("refused", format!("{} would land outside {}", o.name, to.display())));
        };
        if abs.exists() {
            there.push(abs.to_string_lossy().into_owned());
        }
        targets.push(abs);
    }
    let answer: Vec<String> = targets.iter().map(|t| t.to_string_lossy().into_owned()).collect();
    if !there.is_empty() && !p.overwrite {
        return Err(CmdError::new(
            "exists",
            format!(
                "{} of the files are there already: asset.export overwrite=true replaces them",
                there.len()
            ),
        )
        .with_data(json!({ "files": there })));
    }
    if p.dry_run {
        return Ok(json!({ "dry_run": true, "files": answer, "replaces": there }));
    }
    let r = root.clone();
    ed.exec
        .io(move || -> Result<(), String> {
            for ((_, o), abs) in outs.iter().zip(&targets) {
                let bytes = crate::export::read(&r, &o.from, &models, &textures)?;
                if let Some(dir) = abs.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("{}: {}", dir.display(), gz_base::io_reason(&e)))?;
                }
                gz_editor::fsx::write_atomic(abs, &bytes)
                    .map_err(|e| format!("{}: {}", abs.display(), gz_base::io_reason(&e)))?;
            }
            Ok(())
        })
        .await
        .map_err(CmdError::io)?;
    Ok(json!({ "files": answer, "replaced": there }))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InfoParams {
    /// The models: library references (`props\box_wood_01`) or model assets by their project
    /// path (`models/wall.meshasset`).
    pub refs: Vec<String>,
}

fn is_asset(r: &str) -> bool {
    r.to_ascii_lowercase().ends_with(MediaKind::Model.extension())
}

fn wire(root: &Path, file: &Path) -> String {
    file.strip_prefix(root).unwrap_or(file).display().to_string().replace('\\', "/")
}

fn model_json(r: &str, source: &str, file: String, o: &LibraryObject) -> Value {
    let b = o.bounds;
    // millimetres: a size is read by a person or an agent, not fed back bit for bit
    let mm = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
    let size = [mm(b.max.x - b.min.x), mm(b.max.y - b.min.y), mm(b.max.z - b.min.z)];
    let obj = &o.object;
    json!({
        "ref": r,
        "source": source,
        "file": file,
        "kind": if obj.is_dynamic() { "dynamic" } else { "static" },
        "bounds": { "min": [mm(b.min.x), mm(b.min.y), mm(b.min.z)], "max": [mm(b.max.x), mm(b.max.y), mm(b.max.z)] },
        "size": size,
        "vertices": obj.meshes.iter().map(|m| m.points.len()).sum::<usize>(),
        "triangles": o.tris.len(),
        "surfaces": obj.surfaces.iter().map(|s| json!({
            "name": s.name, "texture": s.texture, "shader": s.shader, "compile": s.shader_xrlc,
            "material": s.game_mtl, "two_sided": s.two_sided(),
        })).collect::<Vec<_>>(),
        "bones": obj.bones.iter().map(|b| b.name.to_string()).collect::<Vec<_>>(),
        "animations": obj.motions.iter().map(|m| m.name.to_string()).collect::<Vec<_>>(),
    })
}

/// One model as `model.info` answers it. Blocking.
fn info_of(root: &Path, lib: &Library, game: &resolve::GameModel, r: &str) -> Value {
    if is_asset(r) {
        let key = r.replace('/', "\\");
        return match resolve::model(root, &key, game) {
            Ok(m) => {
                let file = wire(root, &root.join(&key));
                let mut o = LibraryObject::provided(&key.to_lowercase(), m.object);
                o.stamp = m.stamp;
                let mut info = model_json(r, "asset", file, &o);
                // a model over a compiled one: the skeleton is the file's own
                if let (Some(c), Some(map)) = (&m.compiled, info.as_object_mut()) {
                    map.insert("bones".into(), json!(c.bones));
                    map.insert(
                        "over".into(),
                        match &c.game {
                            Some(name) => json!({ "game": name }),
                            None => json!({ "file": m.source.as_deref().map(|p| wire(root, p)) }),
                        },
                    );
                }
                info
            }
            Err(e) => json!({ "ref": r, "error": e.to_string() }),
        };
    }
    match lib.load(r) {
        Ok(o) => {
            let source = if o.file.starts_with(root) { "project" } else { "library" };
            model_json(r, source, wire(root, &o.file), &o)
        }
        Err(e) => json!({ "ref": r, "error": e }),
    }
}

async fn model_info(ed: Arc<Editor>, p: InfoParams, _o: Origin) -> Reply {
    if p.refs.is_empty() {
        return Err(CmdError::invalid("refs names no model"));
    }
    if p.refs.len() > MAX_INFO {
        return Err(CmdError::invalid(format!(
            "refs names {} models; at most {MAX_INFO} at once",
            p.refs.len()
        )));
    }
    let svc = SceneService::of(&ed)?;
    let (root, lib, exec) = (svc.root.clone(), svc.library.clone(), ed.exec.clone());
    let game = crate::GameModels::reader(&ed, &root);
    let models = ed
        .exec
        .io(move || {
            exec.io_pool().install(|| {
                use rayon::prelude::*;
                p.refs.par_iter().map(|r| info_of(&root, &lib, &game, r)).collect::<Vec<_>>()
            })
        })
        .await;
    Ok(json!({ "models": models }))
}

const MAX_INFO: usize = 64;
/// The most models `model.list sizes=true` loads at once.
const MAX_SIZES: usize = 100;

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// The project's own library objects (`rawdata\objects`).
    Project,
    /// The editor's library.
    Library,
    /// The project's model assets.
    Asset,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    /// Text the reference holds, any case.
    #[serde(default)]
    pub query: Option<String>,
    /// Only the models under a folder: of the library (`props`) or of the project (`models/bunker`).
    #[serde(default)]
    pub folder: Option<String>,
    /// Only the models of one source.
    #[serde(default)]
    pub source: Option<ModelSource>,
    /// Each model's size in metres, `[x, y, z]`: the models of the page are loaded (at most 100).
    #[serde(default)]
    pub sizes: bool,
    /// How many models to answer (default 100); `total` says how many there are.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many to skip first.
    #[serde(default)]
    pub offset: Option<usize>,
}

async fn model_list(ed: Arc<Editor>, p: ListParams, _o: Origin) -> Reply {
    let limit = p.limit.unwrap_or(100).max(1);
    if p.sizes && limit > MAX_SIZES {
        return Err(CmdError::invalid(format!(
            "sizes=true loads every model of the page: limit at most {MAX_SIZES}"
        )));
    }
    let svc = SceneService::of(&ed)?;
    let media = Media::of(&ed).await?;
    let index = media.index().await;
    let assets: Vec<String> =
        index.of_kind(MediaKind::Model).filter(|e| !e.excluded).map(|e| e.path.to_string()).collect();
    let (root, lib, exec) = (svc.root.clone(), svc.library.clone(), ed.exec.clone());
    let game = crate::GameModels::reader(&ed, &root);
    let answer = ed
        .exec
        .io(move || {
            let project_root = lib.roots().first().cloned();
            let in_project = |r: &str| project_root.as_ref().is_some_and(|pr| pr.join(format!("{r}.object")).is_file());
            // a reference is written with either slash: the query too
            let query = p.query.as_deref().map(|q| q.to_lowercase().replace('/', "\\")).unwrap_or_default();
            let folder = p.folder.as_deref().map(|f| f.replace('/', "\\").trim_matches('\\').to_lowercase());
            let fits = |r: &str| {
                let k = r.replace('/', "\\").to_lowercase();
                (query.is_empty() || k.contains(&query))
                    && folder.as_ref().is_none_or(|f| f.is_empty() || k.starts_with(&format!("{f}\\")))
            };
            let mut items: Vec<(String, &'static str)> = Vec::new();
            if p.source != Some(ModelSource::Asset) {
                for r in lib.list().iter().filter(|r| fits(r)) {
                    let s = if in_project(r) { "project" } else { "library" };
                    if p.source.is_none_or(|want| (want == ModelSource::Project) == (s == "project")) {
                        items.push((r.clone(), s));
                    }
                }
            }
            if p.source.is_none_or(|s| s == ModelSource::Asset) {
                items.extend(assets.iter().filter(|a| fits(a)).map(|a| (a.clone(), "asset")));
            }
            let total = items.len();
            let from = p.offset.unwrap_or(0).min(total);
            let page = &items[from..(from + limit).min(total)];
            let models: Vec<Value> = if p.sizes {
                exec.io_pool().install(|| {
                    use rayon::prelude::*;
                    page.par_iter()
                        .map(|(r, s)| {
                            let info = info_of(&root, &lib, &game, r);
                            json!({ "ref": r, "source": s, "size": info.get("size").cloned().unwrap_or(Value::Null) })
                        })
                        .collect()
                })
            } else {
                page.iter().map(|(r, s)| json!({ "ref": r, "source": s })).collect()
            };
            json!({ "models": models, "total": total })
        })
        .await;
    Ok(answer)
}

/// The `model` area and `asset.import`.
pub fn register(reg: &mut Registry) {
    reg.area("model", "Models: library objects and model assets, what they are and what there is");
    reg.cmd(
        Spec::read(
            "model.info",
            "What models are: bounds and size in metres, vertices, triangles, surfaces, bones, animations.",
        )
        .example("refs=[\"props\\\\box_wood_01\",\"models/wall.meshasset\"]"),
        model_info,
    );
    reg.cmd(
        Spec::read(
            "model.list",
            "The models there are: the library's objects and the project's model assets.",
        )
        .example("query=barrel source=library sizes=true limit=20"),
        model_list,
    );
    reg.cmd(
        Spec::write(
            "asset.import",
            "Brings model, picture and sound files into the project, each with its asset beside it; a file of the project gets its asset where it lies, saying what the game's own formats say of it.",
        )
        .destroys_with(&["overwrite", "remove_descriptions"])
        .example("from=[\"D:/art/wall.glb\"] to=models/bunker"),
        asset_import,
    );
    reg.cmd(
        Spec::write(
            "asset.export",
            "Writes media assets' files out of the project as they were brought in: the source with what it names beside it and its .thm, or the game's file an asset describes.",
        )
        .destroys_with(&["overwrite"])
        .example("path=textures/wall.texasset to=D:/export"),
        asset_export,
    );
}
