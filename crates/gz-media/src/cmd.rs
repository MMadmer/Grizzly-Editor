//! What the media area hangs on the editor: the three kinds of `gz-asset`, their `file.create`
//! templates, the provider of the scene's model assets, the index made when a project opens;
//! and the commands `asset.import`, `model.info`, `model.list`.

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

use crate::import::{self, Content};
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
    /// out, the `.thm` stays beside its picture and ships as it did.
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
    let (writes, r, watch) = (plan.writes.clone(), root.clone(), project.clone());
    let written = ed
        .exec
        .io(move || -> Result<Vec<PathBuf>, String> {
            let mut done = Vec::new();
            for (rel, content) in writes {
                let abs = rel.to_fs(&r);
                let bytes = match content {
                    Content::Bytes(b) => b,
                    Content::Copy(src) => std::fs::read(&src)
                        .map_err(|e| format!("{}: {}", src.display(), gz_base::io_reason(&e)))?,
                };
                if let Some(dir) = abs.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("{}: {}", dir.display(), gz_base::io_reason(&e)))?;
                }
                gz_editor::fsx::write_atomic(&abs, &bytes)
                    .map_err(|e| format!("{rel}: {}", gz_base::io_reason(&e)))?;
                watch.tree.changes().note_written(&rel, &bytes);
                done.push(abs);
            }
            // the descriptions the assets now say all of, once every asset is written
            for rel in removes {
                let abs = rel.to_fs(&r);
                match std::fs::remove_file(&abs) {
                    Ok(()) => done.push(abs),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{rel}: {}", gz_base::io_reason(&e))),
                }
            }
            Ok(done)
        })
        .await
        .map_err(CmdError::io)?;
    for abs in &written {
        project.tree.mark_fs(abs);
    }
    project.tree.refresh().await;
    Ok(
        json!({ "assets": plan.assets, "copied": plan.copied, "kept": plan.kept, "removed": removed, "descriptions": left, "problems": plan.problems }),
    )
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
}
