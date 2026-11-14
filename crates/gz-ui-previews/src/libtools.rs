//! The Libraries menu's maintenance rows and the commands behind them: `library.reload` (the
//! scene's objects, the textures or the light animations read again), `library.rewrite_descriptions`
//! (every `.thm` of the texture library written in this editor's format) and `library.sync`
//! (textures against their sources, sounds against theirs); the Texture Library's Cube Map >
//! Smaller Copy.
//!
//! The two long ones are commands like every other: each starts its job on the editor's pools,
//! waits for it and answers what it did, with no window and no tab. A command never asks; where
//! a person is to be asked, the window's action is the one that asks - `library.rewrite_descriptions`
//! rewrites the author's files in place, so its action first sends the command with `dry_run`
//! and asks over what it names; `library.sync` for the sounds renames the game sounds that have
//! no source any more, so its action asks over the orphans the answer names and sends the command
//! again with `retire_orphans`. A texture synchronisation only makes what is missing or older
//! and needs no question at all.

use std::sync::Arc;

use gz_app::{Action, ReplyCx, ShellBuilder, args};
use gz_editor::{Args, CmdError, Editor, JobId, Origin, Registry, Reply, Spec, json};
use gz_fmt_tex::{TextureFlags, TextureFormat, TextureParams, TextureType};
use gz_ui::dialog::{self, Answer, ConfirmSpec};
use gz_ui::{Icon, Tx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::library::stores::LibraryFacts;
use crate::library::{LibraryLists, can_write, sounds, texsync};
use crate::paths::{file_age, list_files};
use crate::writer::LibraryWriter;

/// How many names an answer lists of a longer list.
const LISTED: usize = 10;

/// The first names of a longer list, for an answer.
fn listed(items: &[String]) -> Vec<&str> {
    items.iter().take(LISTED).map(String::as_str).collect()
}

/// The scene's object library forgets what it read; the objects the scene shows are read again
/// at once.
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

/// Every texture cache of the window reads its textures again.
pub fn reload_textures(editor: &Editor) {
    gz_app::gpu::TextureReload::bump(editor);
}

/// A folder the library writes, checked on the IO pool: the command's refusal when it cannot be.
async fn writable(editor: &Arc<Editor>, dir: std::path::PathBuf, what: &'static str) -> Result<(), CmdError> {
    if editor.exec.io(move || can_write(&dir)).await {
        Ok(())
    } else {
        Err(CmdError::new("read_only", format!("The {what} folder cannot be written.")))
    }
}

fn busy(busy: impl std::fmt::Display) -> CmdError {
    CmdError::busy(busy.to_string())
}

/// The answer of a job a command started: at once for `wait=false`, else what it ended with.
async fn answer(editor: &Arc<Editor>, id: JobId, wait: bool) -> Reply {
    if !wait {
        return Ok(json!({ "job": id.0 }));
    }
    crate::library::textures::job_answer(editor, id).await
}

// ---- library.sync ------------------------------------------------------------------------------

/// The libraries `library.sync` brings up to date with their sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SyncKind {
    /// The `.tga` sources of `rawdata\textures` against their descriptions and game textures.
    TextureLibrary,
    /// The `.wav` sources of `rawdata\sounds` against their descriptions and game sounds.
    SoundLibrary,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SyncParams {
    /// The library to synchronize.
    kind: SyncKind,
    /// sound_library: also rename the game sounds and descriptions that have no `.wav` any more to
    /// `.~ogg` and `.~thm` (never deleted). The answer names them either way.
    #[serde(default)]
    retire_orphans: bool,
    /// Answer when the job has ended; false answers the job at once.
    #[serde(default = "crate::cmd::yes_wait")]
    wait: bool,
}

/// What a texture synchronisation answers.
fn texture_sync_answer(r: &texsync::TexSyncReport) -> Value {
    json!({
        "kind": "texture_library",
        "updated": r.updated.len(),
        "names": listed(&r.updated),
        "up_to_date": r.up_to_date,
        "failed": listed(&r.failed),
        "failed_count": r.failed.len(),
    })
}

/// What a sound synchronisation answers.
fn sound_sync_answer(r: &sounds::SoundSyncReport) -> Value {
    json!({
        "kind": "sound_library",
        "updated": r.updated.len(),
        "names": listed(&r.updated),
        "up_to_date": r.up_to_date,
        "orphan_count": r.orphans.len(),
        "orphans": listed(&r.orphans),
        "retired": r.retired,
        "failed": listed(&r.failed),
        "failed_count": r.failed.len(),
    })
}

fn start_texture_sync(editor: &Arc<Editor>) -> Result<JobId, CmdError> {
    let (writer, ed) = (LibraryWriter::of(editor), editor.clone());
    editor
        .jobs
        .start("sync_textures", Some("libraries"), "Synchronizing Textures", move |ctx| async move {
            let c = ctx.clone();
            let done = ctx
                .exec
                .io(move || {
                    texsync::synchronize_all(&writer, &c.cancel, |i, n, name| {
                        c.stage(format!("Synchronizing {name}"));
                        c.progress(i, n);
                    })
                })
                .await;
            LibraryLists::of(&ed).textures_rev.bump();
            let report = done.map_err(gz_task::JobError::new)?;
            for f in &report.failed {
                ed.log.error(f.clone());
            }
            ed.log.info(format!("Textures synchronized: {} written.", report.updated.len()));
            Ok(texture_sync_answer(&report))
        })
        .map_err(busy)
}

fn start_sound_sync(editor: &Arc<Editor>, retire_orphans: bool) -> Result<JobId, CmdError> {
    let (writer, ed) = (LibraryWriter::of(editor), editor.clone());
    editor
        .jobs
        .start("sync_sounds", Some("libraries"), "Synchronizing Sounds", move |ctx| async move {
            let c = ctx.clone();
            let done = ctx
                .exec
                .io(move || {
                    c.stage("Synchronizing the sounds");
                    sounds::synchronize_sounds(&writer, &c.cancel, retire_orphans)
                })
                .await;
            LibraryLists::of(&ed).sounds_rev.bump();
            let report = done.map_err(gz_task::JobError::new)?;
            for f in &report.failed {
                ed.log.warn(f.clone());
            }
            ed.log.info(format!("Sounds synchronized: {} written.", report.updated.len()));
            Ok(sound_sync_answer(&report))
        })
        .map_err(busy)
}

/// `library.sync`: the library against its sources, as a job the command waits for.
async fn library_sync(ed: Arc<Editor>, p: SyncParams, _origin: Origin) -> Reply {
    let dirs = crate::paths::EditorDirs::of(&ed);
    let id = match p.kind {
        SyncKind::TextureLibrary => {
            if p.retire_orphans {
                return Err(CmdError::invalid("retire_orphans is the sound library's: kind=sound_library"));
            }
            writable(&ed, dirs.textures, "texture").await?;
            start_texture_sync(&ed)?
        }
        SyncKind::SoundLibrary => {
            writable(&ed, dirs.sounds, "sound").await?;
            start_sound_sync(&ed, p.retire_orphans)?
        }
    };
    answer(&ed, id, p.wait).await
}

/// Synchronize Sounds in the window: the answer names the sounds that have no source any more;
/// a person says whether they are renamed out of the way, and Yes sends the command again for it.
fn sound_sync_reply(r: &ReplyCx<'_>) {
    let reply = r.reply;
    let ok = reply.get("ok").and_then(Value::as_bool) == Some(true);
    if !ok {
        let text = reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.");
        r.editor.notices.error(text.to_owned());
    }
    let count = reply.get("orphan_count").and_then(Value::as_u64).unwrap_or(0);
    let retired = reply.get("retired").and_then(Value::as_u64).unwrap_or(0);
    if count == 0 || retired > 0 {
        return;
    }
    let items = reply
        .get("orphans")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|n| Tx::raw(n.to_owned())).collect())
        .unwrap_or_default();
    let spec = ConfirmSpec {
        id: "sync_sounds".to_owned(),
        title: "Synchronize Sounds".into(),
        headline: "These sounds have no .wav source any more. Rename them out of the way?".into(),
        items,
        total: usize::try_from(count).unwrap_or(usize::MAX),
        consequence: "They are renamed to .~ogg and .~thm. Nothing is deleted.".into(),
        action: "Rename".into(),
        destructive: false,
    };
    let asker = if r.editor.flags.unattended { dialog::Asker::Unattended } else { dialog::Asker::Ui };
    let ed = r.editor.clone();
    let line = dialog::confirm(r.ctx, asker, spec, move |answer| {
        if answer == Answer::Yes {
            let mut a = args(&[("kind", "sound_library")]);
            a.insert("retire_orphans".into(), json!(true));
            ed.post("library.sync", a);
        }
    });
    if let Some(line) = line {
        r.editor.log.finding(&line);
    }
}

// ---- library.rewrite_descriptions --------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RewriteParams {
    /// Write nothing: answer how many descriptions there are and name the first ones.
    #[serde(default)]
    dry_run: bool,
    /// Answer when the job has ended; false answers the job at once.
    #[serde(default = "crate::cmd::yes_wait")]
    wait: bool,
}

/// Each description read and written again in this editor's format. One that does not read is
/// left as it is and named; one that would come out the same is not written (its age stays).
fn rewrite_all(
    writer: &LibraryWriter,
    files: &[(String, std::path::PathBuf)],
    c: &gz_task::JobCtx,
) -> (Vec<String>, usize, Vec<String>) {
    let (mut rewritten, mut unchanged, mut failed) = (Vec::new(), 0usize, Vec::new());
    for (i, (name, path)) in files.iter().enumerate() {
        if c.cancel.is_cancelled() {
            break;
        }
        c.stage(format!("Rewriting {name}"));
        c.progress(i, files.len());
        let one = || -> Result<bool, String> {
            let old = std::fs::read(path).map_err(|e| e.to_string())?;
            let thm = gz_fmt_tex::read_texture_thm(&old).map_err(|e| e.to_string())?;
            let new = gz_fmt_tex::write_texture_thm(&thm).map_err(|e| e.to_string())?;
            if new == old {
                return Ok(false);
            }
            // the description keeps the age it had: the synchronisation knows a texture is
            // current by that age
            let age = file_age(path);
            writer.write(path, &new)?;
            if let Some(age) = age {
                let _ = writer.set_age(path, age);
            }
            Ok(true)
        };
        match one() {
            Ok(true) => rewritten.push(name.clone()),
            Ok(false) => unchanged += 1,
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    (rewritten, unchanged, failed)
}

/// `library.rewrite_descriptions`: the `.thm` files of the texture library written in this
/// editor's format, as a job the command waits for.
async fn library_rewrite(ed: Arc<Editor>, p: RewriteParams, _origin: Origin) -> Reply {
    let writer = LibraryWriter::of(&ed);
    let dir = writer.textures.clone();
    let files = ed.exec.io(move || list_files(&dir, &["thm"], false, false)).await;
    let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
    if p.dry_run {
        return Ok(json!({ "dry_run": true, "files": names.len(), "names": listed(&names) }));
    }
    if files.is_empty() {
        return Ok(json!({
            "rewritten": 0,
            "unchanged": 0,
            "failed": Vec::<String>::new(),
            "note": "The texture library holds no texture descriptions.",
        }));
    }
    writable(&ed, writer.textures.clone(), "texture").await?;
    let ed2 = ed.clone();
    let id = ed
        .jobs
        .start(
            "texture_library",
            Some("libraries"),
            "Rewriting Texture Descriptions",
            move |ctx| async move {
                let c = ctx.clone();
                let (rewritten, unchanged, failed) =
                    ctx.exec.io(move || rewrite_all(&writer, &files, &c)).await;
                for f in &failed {
                    ed2.log.error(format!("The texture description was not rewritten: {f}"));
                }
                ed2.log.info(format!("Texture descriptions rewritten: {}.", rewritten.len()));
                LibraryLists::of(&ed2).textures_rev.bump();
                Ok(json!({
                    "rewritten": rewritten.len(),
                    "names": listed(&rewritten),
                    "unchanged": unchanged,
                    "failed": listed(&failed),
                    "failed_count": failed.len(),
                }))
            },
        )
        .map_err(busy)?;
    answer(&ed, id, p.wait).await
}

/// Rewrite Texture Descriptions in the window: the command's dry run names what would be
/// written, a person is asked over it, and Yes sends the command again for real.
fn rewrite_reply(r: &ReplyCx<'_>) {
    let reply = r.reply;
    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        let text = reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.");
        r.editor.notices.error(text.to_owned());
        return;
    }
    let total = reply.get("files").and_then(Value::as_u64).unwrap_or(0);
    let asker = if r.editor.flags.unattended { dialog::Asker::Unattended } else { dialog::Asker::Ui };
    if total == 0 {
        let line = dialog::notice(
            r.ctx,
            asker,
            gz_ui::Tone::Info,
            "Rewrite Texture Descriptions",
            "The texture library holds no texture descriptions.",
        );
        if let Some(line) = line {
            r.editor.log.finding(&line);
        }
        return;
    }
    let items = reply
        .get("names")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|n| Tx::raw(n.to_owned())).collect())
        .unwrap_or_default();
    let spec = ConfirmSpec {
        id: "rewrite_thumbnails".to_owned(),
        title: "Rewrite Texture Descriptions".into(),
        headline: "Rewrite every texture description (.thm) of the texture library?".into(),
        items,
        total: usize::try_from(total).unwrap_or(usize::MAX),
        consequence: "Each file is read and written again in place, in the format this editor writes.".into(),
        action: "Rewrite".into(),
        destructive: false,
    };
    let ed = r.editor.clone();
    let line = dialog::confirm(r.ctx, asker, spec, move |answer| {
        if answer == Answer::Yes {
            ed.post("library.rewrite_descriptions", Args::new());
        }
    });
    if let Some(line) = line {
        r.editor.log.finding(&line);
    }
}

// ---- cube maps ---------------------------------------------------------------------------------

/// Cube Map \ Smaller Copy: `<name>.tga` of the texture sources made
/// into `<name>#small.dds` of the game textures, 32 x 32 a face, as a job.
pub fn smaller_cube_map(editor: &Arc<Editor>, name: &str) -> Result<gz_editor::JobId, String> {
    let dirs = LibraryWriter::of(editor);
    let (src, dst) =
        (dirs.textures.join(format!("{name}.tga")), dirs.game_textures.join(format!("{name}#small.dds")));
    let (ed, name) = (editor.clone(), name.to_owned());
    let started = editor.jobs.start(
        "texture_library",
        Some("libraries"),
        "Making the Smaller Cube Map",
        move |ctx| async move {
            let n = name.clone();
            let made = ctx
                .exec
                .io(move || -> Result<(), String> {
                    let unread = |why: String| format!("The texture '{n}' could not be read: {why}.");
                    let bytes = std::fs::read(&src).map_err(|e| unread(e.to_string()))?;
                    let loaded =
                        gz_fmt_tex::load_image(&bytes, Some("tga")).map_err(|e| unread(e.to_string()))?;
                    let small = gz_fmt_tex::cubemap::simplify(&loaded.image).map_err(|e| match e {
                        gz_fmt_tex::TexError::Invalid(why) => {
                            format!("The texture '{n}' is no cube map that can be made smaller: {why}.")
                        }
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
                    dirs.write(&dst, &made.dds)
                })
                .await;
            match made {
                Ok(()) => {
                    ed.notices.note("The smaller cube map was made.".to_owned());
                    Ok(json!({ "texture": format!("{name}#small") }))
                }
                Err(e) => {
                    ed.log.error(&e);
                    Err(gz_task::JobError::new(e))
                }
            }
        },
    );
    started.map_err(|busy| format!("Smaller Copy waits: {busy}."))
}

// ---- the window --------------------------------------------------------------------------------

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
        Action::new("library.sync_textures", "Synchronize Textures", "Libraries", "library.sync")
            .icon(Icon::Refresh)
            .args(|_| args(&[("kind", "texture_library")]))
            .enabled(|cx| {
                gz_app::Enabled::when(
                    !LibraryFacts::of(cx.editor).tex().read_only,
                    "The texture folder cannot be written.",
                )
            }),
    );
    b.action(
        Action::new("library.sync_sounds", "Synchronize Sounds", "Libraries", "library.sync")
            .icon(Icon::Refresh)
            .args(|_| args(&[("kind", "sound_library")]))
            .reply(sound_sync_reply)
            .enabled(|cx| {
                gz_app::Enabled::when(
                    !LibraryFacts::of(cx.editor).sound().read_only,
                    "The sound folder cannot be written.",
                )
            }),
    );
    b.action(
        Action::new(
            "library.rewrite_descriptions",
            "Rewrite Texture Descriptions...",
            "Libraries",
            "library.rewrite_descriptions",
        )
        .icon(Icon::Refresh)
        .args(|_| {
            let mut a = Args::new();
            a.insert("dry_run".into(), json!(true));
            a
        })
        .reply(rewrite_reply)
        .enabled(|cx| {
            gz_app::Enabled::when(
                !LibraryFacts::of(cx.editor).tex().read_only,
                "The texture folder cannot be written.",
            )
        }),
    );
    b.menu_action("Libraries", 1, 0, "library.reload_objects", None);
}

/// What `library.reload` reads again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ReloadKind {
    /// Every texture the window shows is read from its file again.
    Textures,
    /// The scene's object library forgets what it read; the objects the scene shows are read at once.
    Objects,
    /// `gamedata\lanims.xr` is read again; unsaved changes need `discard`.
    LightAnimations,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReloadParams {
    /// What to read again.
    kind: ReloadKind,
    /// light_animations: drop the unsaved changes of the library.
    #[serde(default)]
    discard: bool,
}

/// `library.reload`: what the editor read of the textures, of the scene's objects or of the
/// light animations, read again.
async fn library_reload(ed: Arc<Editor>, p: ReloadParams, _origin: Origin) -> Reply {
    match p.kind {
        ReloadKind::Textures => {
            reload_textures(&ed);
            Ok(json!({ "reloaded": "textures" }))
        }
        ReloadKind::Objects => {
            reload_objects(&ed).map_err(|e| CmdError::new("refused", e))?;
            Ok(json!({ "reloaded": "objects" }))
        }
        ReloadKind::LightAnimations => {
            let stores = crate::library::stores::LibraryStores::of(&ed);
            if stores.lanim.is_dirty() && !p.discard {
                return Err(CmdError::unsaved(
                    json!([{ "id": "light_animations", "label": "Light Animations" }]),
                )
                .with_data(json!({ "note": "pass discard=true" })));
            }
            crate::library::stores::LibraryStores::reload_lanims(&ed).await;
            Ok(json!({ "reloaded": "light_animations" }))
        }
    }
}

/// Registers `library.sync`, `library.reload` and `library.rewrite_descriptions`.
pub fn register(reg: &mut Registry) {
    reg.area(
        "library",
        "The editor's libraries: textures, sounds, light animations, the objects the scene reads",
    );
    reg.cmd(
        Spec::write(
            "library.sync",
            "Brings the texture or the sound library up to date with its sources: what is missing or older is made, as a job.",
        )
        .example("kind=texture_library"),
        library_sync,
    );
    reg.cmd(
        Spec::write(
            "library.reload",
            "Reads the textures, the scene's objects or the light animations again.",
        )
        .destroys_with(&["discard"])
        .example("kind=light_animations"),
        library_reload,
    );
    reg.cmd(
        Spec::write(
            "library.rewrite_descriptions",
            "Writes every texture description of the library in this editor's format, as a job.",
        )
        .example("dry_run=true"),
        library_rewrite,
    );
}
