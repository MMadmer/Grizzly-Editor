//! The Libraries menu's maintenance rows and the commands behind them: `library.reload` (the
//! scene's objects or the textures read again), `library.rewrite_descriptions` (every `.thm` of the
//! texture library written in this editor's format) and `library.sync` (textures against their
//! sources, sounds against theirs); the Texture Library's Cube Map > Smaller Copy.

use std::sync::Arc;
use std::time::Instant;

use gz_app::{Action, ShellBuilder, args};
use gz_editor::{CmdError, Editor, Origin, Registry, Reply, Spec, json};
use gz_fmt_tex::{TextureFlags, TextureFormat, TextureParams, TextureType};
use gz_ui::Icon;
use gz_ui::dialog::{self, Answer, ConfirmSpec};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::paths::{EditorDirs, list_files};

/// `Lib.ReloadObjects`: the scene's object library forgets what it read; the objects the scene
/// shows are read again at once.
pub fn reload_objects(editor: &Arc<Editor>) -> Result<(), String> {
    let svc = editor
        .project_opt()
        .and_then(|s| s.services.get::<gz_scene::SceneService>())
        .ok_or_else(|| "no scene is open".to_owned())?;
    svc.library.clear();
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let snap = svc.store.snapshot();
        svc.prefetch_library(&ed, &snap).await;
        ed.exec.wake().ping();
    });
    Ok(())
}

/// `EDevice->ReloadTextures`: every texture cache of the window reads its textures again.
pub fn reload_textures(editor: &Editor) {
    gz_app::gpu::TextureReload::bump(editor);
}

/// Rewrite Texture Descriptions...: the `.thm` files of the texture library are counted off the
/// frame, the question names them, and Yes rewrites each in this editor's format, as a job.
pub fn rewrite_thumbnails(editor: &Arc<Editor>) {
    let asker = if editor.flags.unattended { dialog::Asker::Unattended } else { dialog::Asker::Ui };
    let dirs = EditorDirs::of(editor);
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let d = dirs.textures.clone();
        let files = ed.exec.io(move || list_files(&d, &["thm"], false, false)).await;
        let Ok(queue) = gz_app::UiQueue::of(&ed) else { return };
        queue.post(move |_shell, ctx| {
            if files.is_empty() {
                let line = dialog::notice(
                    ctx,
                    asker,
                    gz_ui::Tone::Info,
                    "Rewrite Texture Descriptions",
                    "The texture library holds no texture descriptions.",
                );
                if let Some(line) = line {
                    ed.log.finding(&line);
                }
                return;
            }
            let spec = ConfirmSpec {
                id: "rewrite_thumbnails".to_owned(),
                title: "Rewrite Texture Descriptions".to_owned(),
                headline: "Rewrite every texture description (.thm) of the texture library?".to_owned(),
                items: files.iter().take(10).map(|(n, _)| n.clone()).collect(),
                total: files.len(),
                consequence:
                    "Each file is read and written again in place, in the format this editor writes."
                        .to_owned(),
                action: "Rewrite".to_owned(),
                destructive: false,
            };
            let e2 = ed.clone();
            let line = dialog::confirm(ctx, asker, spec, move |answer| {
                if answer == Answer::Yes {
                    start_rewrite(&e2, files);
                }
            });
            if let Some(line) = line {
                ed.log.finding(&line);
            }
        });
    });
}

/// Each description read and written again.
fn start_rewrite(editor: &Arc<Editor>, files: Vec<(String, std::path::PathBuf)>) {
    let ed = editor.clone();
    let started = editor.jobs.start(
        "texture_library",
        Some("libraries"),
        "Rewriting Texture Descriptions",
        move |ctx| async move {
            let c = ctx.clone();
            let total = files.len();
            let (done, failed) = ctx
                .exec
                .io(move || {
                    let mut done = 0usize;
                    let mut failed = Vec::new();
                    for (i, (name, path)) in files.iter().enumerate() {
                        if c.cancel.is_cancelled() {
                            break;
                        }
                        c.stage(format!("Rewriting {name}"));
                        c.progress(i, total);
                        // a file that does not read as a description is left as it is
                        let rewritten = std::fs::read(path)
                            .map_err(|e| e.to_string())
                            .and_then(|b| gz_fmt_tex::read_texture_thm(&b).map_err(|e| e.to_string()))
                            .and_then(|t| gz_fmt_tex::write_texture_thm(&t).map_err(|e| e.to_string()))
                            .and_then(|b| gz_editor::fsx::write_atomic(path, &b).map_err(|e| e.to_string()));
                        match rewritten {
                            Ok(()) => done += 1,
                            Err(e) => failed.push(format!("{name}: {e}")),
                        }
                    }
                    (done, failed)
                })
                .await;
            for f in &failed {
                ed.log.error(format!("The texture description was not rewritten: {f}"));
            }
            ed.log.info(format!("Texture descriptions rewritten: {done}."));
            crate::library::LibraryLists::of(&ed).textures_rev.bump();
            Ok(serde_json::json!({ "rewritten": done, "failed": failed }))
        },
    );
    if let Err(busy) = started {
        editor.notices.error(format!("Rewrite Texture Descriptions waits: {busy}."));
    }
}

/// Cube Map \ Smaller Copy: `<name>.tga` of the texture sources made
/// into `<name>#small.dds` of the game textures, 32 x 32 a face, as a job.
pub fn smaller_cube_map(editor: &Arc<Editor>, dirs: &EditorDirs, name: &str) {
    let (src, dst) =
        (dirs.textures.join(format!("{name}.tga")), dirs.game_textures.join(format!("{name}#small.dds")));
    let (ed, name) = (editor.clone(), name.to_owned());
    let started = editor.jobs.start(
        "texture_library",
        Some("libraries"),
        "Cube Map: scale image...",
        move |ctx| async move {
            let t = Instant::now();
            let n = name.clone();
            let made = ctx
                .exec
                .io(move || -> Result<(), String> {
                    let bytes = std::fs::read(&src).map_err(|_| format!("Can't load texture '{n}'."))?;
                    let loaded = gz_fmt_tex::load_image(&bytes, Some("tga"))
                        .map_err(|_| format!("Can't load texture '{n}'."))?;
                    let small = gz_fmt_tex::cubemap::simplify(&loaded.image).map_err(|e| match e {
                        gz_fmt_tex::TexError::Invalid(why) => format!("Texture '{n}' - {why}"),
                        other => other.to_string(),
                    })?;
                    let params = TextureParams {
                        width: i32::try_from(small.width).unwrap_or(0),
                        height: i32::try_from(small.height).unwrap_or(0),
                        fmt: TextureFormat::RGBA,
                        ty: TextureType::CUBE_MAP,
                        flags: TextureFlags::empty(),
                        ..TextureParams::default()
                    };
                    let made =
                        gz_fmt_tex::make_game_texture(&small, &params, None).map_err(|e| e.to_string())?;
                    if let Some(dir) = dst.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    gz_editor::fsx::write_atomic(&dst, &made.dds)
                        .map_err(|e| format!("Can't write {}: {e}", dst.display()))
                })
                .await;
            match made {
                Ok(()) => {
                    ed.notices.note(format!(
                        "Smaller cubemap successfully created [{:.2} sec].",
                        t.elapsed().as_secs_f32()
                    ));
                    Ok(serde_json::json!({ "texture": format!("{name}#small") }))
                }
                Err(e) => {
                    ed.log.error(&e);
                    Err(gz_task::JobError::new(e))
                }
            }
        },
    );
    if let Err(busy) = started {
        editor.notices.error(format!("Smaller Copy waits: {busy}."));
    }
}

/// The actions and rows of the Libraries menu's maintenance group.
pub fn install(b: &mut ShellBuilder) {
    b.action(
        Action::new("library.reload_objects", "Reload Objects", "Libraries", "library.reload")
            .icon(Icon::Refresh)
            .args(|_| args(&[("kind", "objects")]))
            .enabled(|cx| {
                let open = cx
                    .editor
                    .project_opt()
                    .and_then(|s| s.services.get::<gz_scene::SceneService>())
                    .is_some();
                gz_app::Enabled::when(open, "No scene is open.")
            }),
    );
    b.action(
        Action::new("library.reload_textures", "Reload Textures", "Libraries", "library.reload")
            .icon(Icon::Refresh)
            .args(|_| args(&[("kind", "textures")])),
    );
    b.action(
        Action::new(
            "library.rewrite_descriptions",
            "Rewrite Texture Descriptions...",
            "Libraries",
            "library.rewrite_descriptions",
        )
        .icon(Icon::Refresh),
    );
    b.menu_action("Libraries", 1, 0, "library.reload_objects", None);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum LibraryKind {
    Textures,
    Sounds,
    Objects,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KindParams {
    /// textures or sounds (sync); textures or objects (reload).
    kind: LibraryKind,
}

/// `library.sync`: the library against its sources, as a job; the window asks first.
async fn library_sync(ed: Arc<Editor>, p: KindParams, _origin: Origin) -> Reply {
    if p.kind == LibraryKind::Objects {
        return Err(CmdError::invalid("kind must be textures or sounds"));
    }
    gz_app::on_ui(&ed, move |shell, ctx| {
        shell.with_cx(ctx, |cx| match p.kind {
            LibraryKind::Textures => crate::synchronize_textures(cx),
            _ => crate::synchronize_sounds(cx),
        });
        Ok(())
    })
    .await?;
    Ok(json!({ "started": true }))
}

/// `library.reload`: what the editor read of the textures or of the scene's objects, read again.
fn library_reload(ed: &Editor, p: &KindParams, _origin: Origin) -> Reply {
    match p.kind {
        LibraryKind::Textures => {
            reload_textures(ed);
            Ok(json!({ "reloaded": "textures" }))
        }
        LibraryKind::Objects => Err(CmdError::invalid("reload kind=objects runs from the window's thread")),
        LibraryKind::Sounds => Err(CmdError::invalid("kind must be textures or objects")),
    }
}

async fn library_reload_any(ed: Arc<Editor>, p: KindParams, origin: Origin) -> Reply {
    if p.kind == LibraryKind::Objects {
        reload_objects(&ed).map_err(|e| CmdError::new("refused", e))?;
        return Ok(json!({ "reloaded": "objects" }));
    }
    library_reload(&ed, &p, origin)
}

async fn library_rewrite(ed: Arc<Editor>, _p: gz_editor::NoParams, _origin: Origin) -> Reply {
    rewrite_thumbnails(&ed);
    Ok(json!({ "started": true }))
}

/// Registers `library.sync`, `library.reload` and `library.rewrite_descriptions`.
pub fn register(reg: &mut Registry) {
    reg.area(
        "library",
        "The editor's libraries: textures, sounds, light animations, the objects the scene reads",
    );
    reg.cmd(
        Spec::write("library.sync", "Synchronizes the textures or the sounds with their sources, as a job.")
            .example("kind=textures"),
        library_sync,
    );
    reg.cmd(
        Spec::write("library.reload", "Reads the textures or the scene's objects again."),
        library_reload_any,
    );
    reg.cmd(
        Spec::write(
            "library.rewrite_descriptions",
            "Writes every texture description of the library in this editor's format, as a job.",
        ),
        library_rewrite,
    );
}
