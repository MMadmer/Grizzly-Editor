//! The tabs that show one asset (docs/UI_DESIGN.md, section 6, "Model Preview and Texture
//! Preview" and "Libraries").
//!
//! - **The model preview** ([`model`], the `model` tab): an orbit viewport of its own - a
//!   `gz_render::Renderer` whose target is sized to the canvas, fed a draw list built here (the
//!   model, a floor grid under it, the bones when asked) and shown through the window's egui
//!   renderer - with Frame, Reset View | Wireframe, Bones | Studio / Unlit | Details. The model
//!   is read, measured and made into a mesh on the pools ([`model_load`]); a visual only the
//!   linked game holds draws with the game's textures through [`loader::Sources`], which reads
//!   the project, then the editor, then the game.
//! - **The texture preview** ([`texture`], the `texture` tab): the file read and uploaded on the
//!   pools, as the GPU takes it when it can (BC1-BC7, 32-bit, every level the file holds),
//!   decoded to B8G8R8A8 otherwise ([`texture_load`]); Fit, 1:1, Zoom | RGBA / R / G / B / A |
//!   Mip | Details, pan and zoom at the cursor, a checker clipped to the view.
//! - **The libraries** ([`library`]): Light Animations (`gamedata\lanims.xr`, a timeline drawn by
//!   hand, the exact colour at a frame), the Texture Library (the `.tga` sources of
//!   `rawdata\textures` with their `.thm`; Check New Textures turns it into New Textures over the
//!   import folder, and the finished import brings the library back) and the Sound Library
//!   (`rawdata\sounds`: a waveform from the peak envelope, playback on an audio thread of its own,
//!   [`audio`]). Lists are walks on the IO pool kept per revision; edits live in a `Store` per
//!   library with undo and are saved explicitly, as a job beside the editor.
//!
//! [`install`] plugs the five tabs and the Libraries menu (Light Animations, Texture Library,
//! Sound Library, Reload Objects) into the shell; Check New Textures, Synchronize Textures...,
//! Synchronize Sounds..., Rewrite Texture Descriptions... and Reload Textures are actions too, on
//! the palette and under their tab's More ([`libtools`]). [`register`] adds `preview.model`,
//! `preview.texture`, `library.view`, `shot.asset` and the `library.*` maintenance commands.
//!
//! Widgets the kit lacks, local to this crate: [`local::local_checker`] (a checkerboard lined
//! up with a picture, clipped to the view), [`local::local_heading`] (a preview's title and its
//! facts),
//! [`local::local_ends`] (a row's two end labels), [`local::local_frame`],
//! [`local::local_texture_pick`] (a texture picked from a searchable list) and
//! [`library::list::LibraryList`] (the libraries' list: search, leaf order, muted folders, only
//! the rows in view laid out).
//!
//! A frame never computes, decodes or touches a file: loads, decodes, uploads, walks and saves
//! run on the pools; a frame registers what arrived with egui and draws it.

#![forbid(unsafe_code)]

pub mod audio;
mod cmd;
pub mod gpu;
pub mod image_pass;
pub mod library;
pub mod libtools;
pub mod loader;
pub mod local;
pub mod model;
pub mod model_load;
pub mod paths;
pub mod pending;
pub mod texture;
pub mod texture_load;

use gz_app::{Action, DocCx, ShellBuilder, args};
use gz_editor::Registry;
use gz_ui::Icon;
use gz_ui::dialog::{self, Answer, ConfirmSpec};

use crate::library::{lanim, sounds, textures};
use crate::paths::EditorDirs;

fn asker(cx: &DocCx<'_>) -> dialog::Asker {
    if cx.editor.flags.unattended { dialog::Asker::Unattended } else { dialog::Asker::Ui }
}

/// Synchronize Textures...: asks, then every source picture against its description and game
/// texture, as a job.
pub(crate) fn synchronize_textures(cx: &mut DocCx<'_>) {
    let spec = ConfirmSpec {
        id: "sync_textures".to_owned(),
        title: "Synchronize Textures".to_owned(),
        headline: "Synchronize the texture library with the texture sources?".to_owned(),
        items: Vec::new(),
        total: 0,
        consequence:
            "The textures whose source changed are converted again: their .dds and .thm files are replaced."
                .to_owned(),
        action: "Synchronize".to_owned(),
        destructive: false,
    };
    let editor = cx.editor.clone();
    let line = dialog::confirm(cx.ctx, asker(cx), spec, move |answer| {
        if answer != Answer::Yes {
            return;
        }
        let dirs = EditorDirs::of(&editor);
        let ed = editor.clone();
        let started = editor.jobs.start(
            "sync_textures",
            Some("libraries"),
            "Synchronizing Textures",
            move |ctx| async move {
                let c = ctx.clone();
                let done = ctx
                    .exec
                    .io(move || {
                        library::texsync::synchronize_all(&dirs, &c.cancel, |i, n, name| {
                            c.stage(format!("Synchronizing {name}"));
                            c.progress(i, n);
                        })
                    })
                    .await;
                library::LibraryLists::of(&ed).textures_rev.bump();
                match done {
                    Ok((updated, failed)) => {
                        for f in &failed {
                            ed.log.error(f.clone());
                        }
                        Ok(serde_json::json!({ "updated": updated, "failed": failed }))
                    }
                    Err(e) => Err(gz_task::JobError::new(e)),
                }
            },
        );
        if let Err(busy) = started {
            editor.notices.error(format!("Synchronize Textures waits: {busy}."));
        }
    });
    if let Some(line) = line {
        cx.editor.log.plain(line);
    }
}

/// Synchronize Sounds...: the orphans are counted off the frame, then the question names them
///.
pub(crate) fn synchronize_sounds(cx: &mut DocCx<'_>) {
    let editor = cx.editor.clone();
    let asker = asker(cx);
    let dirs = EditorDirs::of(&editor);
    let exec = editor.exec.clone();
    editor.exec.clone().spawn(async move {
        let d = dirs.clone();
        let (items, total) = exec.io(move || sounds::orphans(&d)).await;
        let Ok(queue) = gz_app::UiQueue::of(&editor) else { return };
        queue.post(move |_shell, ctx| {
            let spec = ConfirmSpec {
                id: "sync_sounds".to_owned(),
                title: "Synchronize Sounds".to_owned(),
                headline: if total > 0 {
                    "Synchronize the sound library? These sounds have no .wav source any more:".to_owned()
                } else {
                    "Synchronize the sound library with the .wav sources?".to_owned()
                },
                items,
                total,
                consequence: if total > 0 {
                    "They are renamed to .~ogg and .~thm; the sounds whose source changed are converted again.".to_owned()
                } else {
                    "The sounds whose source changed are converted again and their game files replaced.".to_owned()
                },
                action: "Synchronize".to_owned(),
                destructive: false,
            };
            let ed = editor.clone();
            let line = dialog::confirm(ctx, asker, spec, move |answer| {
                if answer != Answer::Yes {
                    return;
                }
                let e2 = ed.clone();
                let started = ed.jobs.start("sync_sounds", Some("libraries"), "Synchronizing Sounds", move |ctx| async move {
                    let c = ctx.clone();
                    let done = ctx.exec.io(move || sounds::synchronize_sounds(&dirs, &c.cancel)).await;
                    library::LibraryLists::of(&e2).sounds_rev.bump();
                    match done {
                        Ok((updated, notes)) => {
                            for n in &notes {
                                e2.log.warn(n.clone());
                            }
                            Ok(serde_json::json!({ "updated": updated, "notes": notes }))
                        }
                        Err(e) => Err(gz_task::JobError::new(e)),
                    }
                });
                if let Err(busy) = started {
                    ed.notices.error(format!("Synchronize Sounds waits: {busy}."));
                }
            });
            if let Some(line) = line {
                editor.log.plain(line);
            }
        });
    });
}

/// Plugs the model and texture previews, the three library tabs and the Libraries menu into the
/// shell.
pub fn install(b: &mut ShellBuilder) {
    b.document("model", model::factory);
    b.document("texture", texture::factory);
    b.document(lanim::KIND, lanim::factory);
    b.document(textures::KIND, textures::factory);
    b.document(sounds::KIND, sounds::factory);

    let open_tab =
        |id: &'static str, title: &'static str, icon: Icon, kind: &'static str| (id, title, icon, kind);
    for (id, title, icon, kind) in [
        open_tab("library.light_animations", "Light Animations", Icon::Light, lanim::KIND),
        open_tab("library.textures", "Texture Library", Icon::FileTexture, textures::KIND),
        open_tab("library.sounds", "Sound Library", Icon::FileSound, sounds::KIND),
    ] {
        let a = Action::new(id, title, "Libraries", "ui.tab").icon(icon);
        b.action(match kind {
            k if k == lanim::KIND => {
                a.args(|_| args(&[("action", "open"), ("doc", lanim::KIND), ("kind", lanim::KIND)]))
            }
            k if k == textures::KIND => {
                a.args(|_| args(&[("action", "open"), ("doc", textures::KIND), ("kind", textures::KIND)]))
            }
            _ => a.args(|_| args(&[("action", "open"), ("doc", sounds::KIND), ("kind", sounds::KIND)])),
        });
    }
    // the library and New Textures share one tab: the library gives way unless it holds
    // unsaved changes, which the shell asks about first
    b.action(
        Action::new("library.check_textures", "Check New Textures", "Libraries", "ui.tab")
            .icon(Icon::Search)
            .args(|_| {
                args(&[
                    ("action", "open"),
                    ("kind", textures::KIND),
                    ("path", "import"),
                    ("source", "import"),
                ])
            }),
    );
    b.action(
        Action::new("library.sync_textures", "Synchronize Textures...", "Libraries", "library.sync")
            .icon(Icon::Refresh)
            .args(|_| args(&[("kind", "textures")])),
    );
    b.action(
        Action::new("library.sync_sounds", "Synchronize Sounds...", "Libraries", "library.sync")
            .icon(Icon::Refresh)
            .args(|_| args(&[("kind", "sounds")])),
    );
    // the menu opens the libraries; what they do by hand is under each tab's More
    for (order, id) in
        ["library.light_animations", "library.textures", "library.sounds"].into_iter().enumerate()
    {
        b.menu_action("Libraries", 0, i32::try_from(order).unwrap_or(0), id, None);
    }
    libtools::install(b);
}

/// The commands of this package: `preview.model`, `preview.texture`, `library.view`, `shot.asset`, `library.*`.
pub fn register(reg: &mut Registry) {
    cmd::register(reg);
    libtools::register(reg);
}
