//! The Libraries menu's maintenance rows and the commands behind them (`MenuLibraries`):
//! Reload Objects (`CommandReloadObjects`), Reload Textures (`CommandReloadTextures`), Rewrite
//! Texture Descriptions... (`RewriteThumbnails`), and Mute Sound (`CommandMuteSound`); the
//! Texture Library's Cube Map \ Smaller Copy (`CreateSmallerCubeMap`).
//!
//! Refresh Sound Environment Geometry is not a row: its C++ command sets a flag whose work,
//! `RealRefreshEnvGeometry`, is commented out - it does nothing there either.

use std::sync::Arc;
use std::time::Instant;

use gz_app::{CommandVar, CppAction, CppCommand, ShellBuilder, UiCommand};
use gz_editor::Editor;
use gz_fmt_tex::{TextureFlags, TextureFormat, TextureParams, TextureType};
use gz_ui::Icon;
use gz_ui::dialog::{self, Answer, ConfirmSpec};

use crate::paths::{EditorDirs, list_files};

/// The Rust command of Rewrite Texture Descriptions... (the C++ ran it from the menu alone).
pub const REWRITE_THUMBNAILS: &str = "COMMAND_REWRITE_THUMBNAILS";

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
                    ed.log.plain(line);
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
                ed.log.plain(line);
            }
        });
    });
}

/// `RewriteThumbnailsNow`: each description read and written again.
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

/// Cube Map \ Smaller Copy (`CreateSmallerCubeMap`): `<name>.tga` of the texture sources made
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

/// The rows and the commands of the Libraries menu's maintenance group.
pub fn install(b: &mut ShellBuilder) {
    b.cpp_command(
        CppCommand::new(
            10,
            "COMMAND_RELOAD_TEXTURES",
            "",
            CppAction::shell(|cx, _, _| {
                reload_textures(cx.editor);
                Ok(CommandVar::Int(1))
            }),
        )
        .editable(false)
        .face("Libraries", "Reload Textures", Icon::Refresh),
    );
    b.cpp_command(
        CppCommand::new(
            73,
            "COMMAND_RELOAD_OBJECTS",
            "",
            CppAction::shell(|cx, _, _| reload_objects(cx.editor).map(|()| CommandVar::Int(1))),
        )
        .editable(false)
        .face("Libraries", "Reload Objects", Icon::Refresh)
        .enabled(|ed, _| ed.project_opt().and_then(|s| s.services.get::<gz_scene::SceneService>()).is_some()),
    );
    b.cpp_command(
        CppCommand::new(
            38,
            "COMMAND_MUTE_SOUND",
            "",
            CppAction::shell(|cx, p1, _| {
                gz_app::gpu::Gpu::of(cx.editor).mute_for_now(cx.editor, p1.truthy());
                Ok(CommandVar::Int(1))
            }),
        )
        .editable(false)
        .face("Libraries", "Mute Sound", Icon::None),
    );
    b.command(UiCommand::new(
        REWRITE_THUMBNAILS,
        "Libraries",
        "Rewrite Texture Descriptions...",
        Icon::Refresh,
        |cx| rewrite_thumbnails(cx.editor),
    ));
    b.cpp_command(
        CppCommand::new(127, REWRITE_THUMBNAILS, "Libraries\\Rewrite Texture Descriptions", CppAction::Ui)
            .face("Libraries", "Rewrite Texture Descriptions...", Icon::Refresh),
    );
    // the rows shown are the faces: their keys and their switches are the commands'
    b.menu_item("Libraries", 2, 0, "COMMAND_RELOAD_OBJECTS");
    b.menu_item("Libraries", 2, 1, "COMMAND_RELOAD_TEXTURES");
    b.menu_item("Libraries", 2, 2, REWRITE_THUMBNAILS);
}
