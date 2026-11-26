//! The names a module brings that are one name for the whole game, said where the author wrote
//! them when the module's id is no word of them (`gz_mod::names`): a section of its configs and
//! patches (`N001`), a string id of its string tables (`N002`), the id of a dialog, a specific
//! character or a profile of its gameplay XML (`N003`), a file it adds to the game's files
//! (`N004`). Two modules that bring one such name collide in the game, which keeps the first and
//! says who lost (`xms_conflicts`). A name the game has already is a change of the game's, and is
//! nobody's to rename.
//!
//! Warnings, never refusals: a module built before the rule keeps working, and its author clears
//! them in the project.

use std::path::Path;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Problem, RelPath};
use gz_build::{BuildCheck, BuildChecks, BuildWalk};
use gz_editor::{Editor, ProjectSession};
use gz_mod::names::{is_own, is_own_path, own_form};

pub const SECTION: &str = "N001";
pub const STRING: &str = "N002";
pub const XML_ID: &str = "N003";
pub const FILE: &str = "N004";

/// The stages of the two build checks: the names written in the module's text files (which
/// `asset.check` gives per file already) and the names of the files themselves.
pub const STAGE_TEXT: &str = "Checking the names the module's files write";
pub const STAGE_FILES: &str = "Checking the names of the module's files";

/// What the game has, as the checks ask it. Each answer is the linked game's, never the project's:
/// a name the module itself made is new.
pub trait GameNames: Send + Sync {
    /// Whether the game's config tree has the section (any case).
    fn section(&self, name: &str) -> bool;
    /// Whether a config, by its path under `configs` (`items\weapons\w_ak74.ltx`), is a file of
    /// the game's config tree: what it says goes into the system config. A config the scripts read
    /// by its path keeps its sections to itself.
    fn in_tree(&self, file: &str) -> bool;
    /// Whether a string table of the game has the id.
    fn string(&self, id: &str) -> bool;
    /// Whether the game has the file, by its path under `gamedata` - or the editor's own data does,
    /// which a build stages into the module under the name the library gives it.
    fn file(&self, path: &str) -> bool;
    /// Whether a section is the module's own, made by one of its configs or items: a patch of it
    /// says nothing new.
    fn module_section(&self, name: &str) -> bool;
    /// Whether a name is another installed module's (its id a word of it): a change of what that
    /// module brings.
    fn other_module(&self, name: &str) -> bool;
}

/// A 1-based place of a byte offset of a text.
fn place(text: &str, at: usize) -> (u32, u32) {
    let head = text.get(..at).unwrap_or(text);
    let line = head.matches('\n').count() + 1;
    let column = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (u32::try_from(line).unwrap_or(u32::MAX), u32::try_from(column).unwrap_or(u32::MAX))
}

fn at(p: Problem, text: &str, start: usize, len: usize) -> Problem {
    let (line, column) = place(text, start);
    let width = u32::try_from(text.get(start..start + len).map_or(1, |s| s.chars().count())).unwrap_or(1);
    p.at(line, column, line, column + width.max(1))
}

/// The path of a project file in the game's files: what follows `gamedata/`, or where a `[vfs]`
/// entry mounts it (`<virtual> = <inside the project>`, a file or a folder). Backslashes, lower
/// case. `None` for a file the module keeps beside its game data (`mod.ltx`, `patch\`, `spawn\`).
#[must_use]
pub fn virtual_path(rel: &str, vfs: &[(String, String)]) -> Option<String> {
    let rel = rel.replace('\\', "/");
    let lower = rel.to_ascii_lowercase();
    let norm = |s: &str| s.trim().replace('/', "\\").trim_matches('\\').to_ascii_lowercase();
    // an explicit mapping wins over the mirror, as the game mounts it
    for (virt, inside) in vfs {
        let inside = inside.trim().replace('\\', "/").trim_matches('/').to_ascii_lowercase();
        if inside.is_empty() {
            continue;
        }
        if lower == inside {
            return Some(norm(virt));
        }
        if let Some(rest) = lower.strip_prefix(&format!("{inside}/")) {
            return Some(format!("{}\\{}", norm(virt), rest.replace('/', "\\")));
        }
    }
    lower.strip_prefix("gamedata/").map(|rest| rest.replace('/', "\\"))
}

/// The headers of an LTX text: the name, its byte offset and length, and whether it deletes.
fn headers(text: &str) -> Vec<(String, usize, usize, bool)> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_start_matches('\u{feff}');
        let lead = line.len() - body.trim_start().len();
        let trimmed = body.trim_start();
        if let Some(rest) = trimmed.strip_prefix('[')
            && let Some(end) = rest.find(']')
        {
            let raw = rest.get(..end).unwrap_or("");
            let (gone, raw) = raw.strip_prefix('!').map_or((false, raw), |r| (true, r));
            let name = raw.trim();
            if !name.is_empty() {
                let skip = raw.len() - raw.trim_start().len() + usize::from(gone);
                out.push((name.to_owned(), offset + lead + 1 + skip, name.len(), gone));
            }
        }
        offset += line.len();
    }
    out
}

/// The `#file` a line of a patch switches to: `Some(None)` back to the system config.
fn file_switch(line: &str) -> Option<Option<String>> {
    let rest = line.trim_start().strip_prefix("#file")?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t', '\r', '\n', ';']) {
        return None;
    }
    let path = rest.split(';').next().unwrap_or("").trim();
    Some((!path.is_empty()).then(|| path.replace('/', "\\").to_ascii_lowercase()))
}

fn section_problem(name: &str, module: &str) -> Problem {
    Problem::warning(
        SECTION,
        format!(
            "[{name}] is a new section and not named after the module: a section is one name for the whole game, and another module that writes [{name}] too is merged into it key by key, the later module's keys winning - name it '{}', or carry the module id as a word of it",
            own_form(name, module)
        ),
    )
}

/// `N001` of a config of the module at `virt` (its path in the game's files: `configs\...`), or of
/// a patch (`patch\*.ltxp`, `is_patch`): every section it makes in the system config that the game
/// does not have. A config the system config reads is `configs\xms\*.ltx` or a file of the game's
/// tree the module replaces; a patch's lines go to the system config, or by `#file` to another
/// config, which counts when it is a file of the tree. Every other config is read by the scripts by
/// its path, and its sections are its own.
#[must_use]
pub fn config_names(
    module: &str,
    virt: &str,
    text: &str,
    is_patch: bool,
    game: &dyn GameNames,
) -> Vec<Problem> {
    if module.is_empty() {
        return Vec::new();
    }
    let virt = virt.replace('/', "\\").to_ascii_lowercase();
    let reaches = |file: &str| {
        let flat_xms = file.strip_prefix("xms\\").is_some_and(|rest| !rest.contains('\\'));
        flat_xms || game.in_tree(file)
    };
    if !is_patch {
        let Some(file) = virt.strip_prefix("configs\\") else { return Vec::new() };
        if !reaches(file) {
            return Vec::new();
        }
    }
    let mut out = Vec::new();
    // a patch starts at the system config
    let mut counts = true;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if is_patch && let Some(to) = file_switch(line) {
            counts = to.as_deref().is_none_or(reaches);
            continue;
        }
        if !counts {
            continue;
        }
        for (name, at_byte, len, gone) in headers(line) {
            // a patch's header with no parents changes a section; one of the module's own is its
            // change, said where the section is made
            let makes = !is_patch
                || line
                    .get(at_byte + len..)
                    .is_some_and(|r| r.trim_start().trim_start_matches(']').trim_start().starts_with(':'));
            if gone
                || game.section(&name)
                || is_own(&name, module)
                || game.other_module(&name)
                || (!makes && game.module_section(&name))
            {
                continue;
            }
            out.push(at(section_problem(&name, module), text, start + at_byte, len));
        }
    }
    out
}

/// The `id` values of the elements named `tags` in an XML text: the id, its byte offset and
/// length, and the tag.
fn ids_of<'a>(text: &str, tags: &[&'a str]) -> Vec<(String, usize, usize, &'a str)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(lt) = text.get(i..).and_then(|r| r.find('<')).map(|p| p + i) {
        // a comment says nothing
        if text.get(lt..).is_some_and(|r| r.starts_with("<!--")) {
            i = text.get(lt..).and_then(|r| r.find("-->")).map_or(text.len(), |p| lt + p + 3);
            continue;
        }
        i = lt + 1;
        let name_end =
            text.get(i..).and_then(|r| r.find(|c: char| c.is_whitespace() || c == '>' || c == '/'));
        let Some(name_end) = name_end.map(|p| p + i) else { break };
        let tag = text.get(i..name_end).unwrap_or("");
        let Some(&want) = tags.iter().find(|t| t.eq_ignore_ascii_case(tag)) else { continue };
        let close = text.get(name_end..).and_then(|r| r.find('>')).map_or(text.len(), |p| p + name_end);
        let attrs = text.get(name_end..close).unwrap_or("");
        let mut j = 0;
        while let Some(p) = attrs.get(j..).and_then(|r| r.find("id")).map(|p| p + j) {
            j = p + 2;
            let before = p.checked_sub(1).and_then(|b| attrs.as_bytes().get(b).copied());
            if !before.is_none_or(|c| c.is_ascii_whitespace()) {
                continue;
            }
            let rest = attrs.get(p + 2..).unwrap_or("");
            let eq = rest.len() - rest.trim_start().len();
            let Some(after_eq) = rest.trim_start().strip_prefix('=') else { continue };
            let lead = after_eq.len() - after_eq.trim_start().len();
            let value = after_eq.trim_start();
            let Some(quote) = value.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
            let Some(end) = value.get(1..).and_then(|v| v.find(quote)) else { continue };
            let id = value.get(1..=end).unwrap_or("");
            let start = name_end + p + 2 + eq + 1 + lead + 1;
            if !id.is_empty() && bytes.get(start..start + id.len()).is_some() {
                out.push((id.to_owned(), start, id.len(), want));
            }
            break;
        }
        i = close;
    }
    out
}

/// `N002` and `N003` of an XML file of the module at `virt`: the string ids of a string table
/// (`configs\text\<language>\`) the game does not have, the ids of the dialogs, specific
/// characters and profiles of its gameplay XML (`configs\gameplay\`). The game keeps the first of
/// such an id and drops a second, so one of the game's own is never the module's either.
#[must_use]
pub fn xml_names(module: &str, virt: &str, text: &str, game: &dyn GameNames) -> Vec<Problem> {
    let virt = virt.replace('/', "\\").to_ascii_lowercase();
    // a file of the game's path replaces the game's: there is one of each id, and none is new
    if module.is_empty() || game.file(&virt) {
        return Vec::new();
    }
    let mut out = Vec::new();
    ids_in(module, &virt, text, 0, text.len(), game, &mut out);
    out
}

/// `N002` and `N003` of an XML patch (`patch\*.xmlp`): what each `<xms-patch target="...">` puts
/// into the game's file is judged as that file's would be - a new string, a new dialog.
#[must_use]
pub fn xmlp_names(module: &str, text: &str, game: &dyn GameNames) -> Vec<Problem> {
    let mut out = Vec::new();
    if module.is_empty() {
        return out;
    }
    let mut i = 0;
    while let Some(open) = text.get(i..).and_then(|r| r.find("<xms-patch")).map(|p| p + i) {
        let head_end = text.get(open..).and_then(|r| r.find('>')).map_or(text.len(), |p| open + p);
        let head = text.get(open..head_end).unwrap_or("");
        let close =
            text.get(head_end..).and_then(|r| r.find("</xms-patch")).map_or(text.len(), |p| head_end + p);
        let target = head
            .split_once("target")
            .and_then(|(_, r)| r.trim_start().strip_prefix('='))
            .map(str::trim_start)
            .and_then(|r| {
                let q = r.chars().next().filter(|c| *c == '"' || *c == '\'')?;
                r.get(1..).and_then(|v| v.split(q).next())
            })
            .unwrap_or("");
        if !target.is_empty() {
            let virt = format!(
                "configs\\{}",
                target.replace('/', "\\").trim_start_matches('\\').to_ascii_lowercase()
            );
            ids_in(module, &virt, text, head_end, close, game, &mut out);
        }
        i = close.max(open + 1);
    }
    out
}

/// The names of `text[from..to]`, an XML of the game's file at `virt`, put at their places in
/// `text`.
fn ids_in(
    module: &str,
    virt: &str,
    text: &str,
    from: usize,
    to: usize,
    game: &dyn GameNames,
    out: &mut Vec<Problem>,
) {
    let part = text.get(from..to).unwrap_or("");
    if virt.starts_with("configs\\text\\") {
        for (id, start, len, _) in ids_of(part, &["string"]) {
            if is_own(&id, module) || game.string(&id) || game.other_module(&id) {
                continue;
            }
            let p = Problem::warning(
                STRING,
                format!(
                    "string '{id}' is new and not named after the module: a string id is one name for the whole game, and another module's string of that id replaces this one - name it '{}', or carry the module id as a word of it",
                    own_form(&id, module)
                ),
            );
            out.push(at(p, text, from + start, len));
        }
    } else if virt.starts_with("configs\\gameplay\\") {
        for (id, start, len, tag) in
            ids_of(part, &["dialog", "specific_character", "character", "info_portion", "article"])
        {
            if is_own(&id, module) || game.other_module(&id) {
                continue;
            }
            let what = match tag {
                "dialog" => "dialog",
                "specific_character" => "specific character",
                "info_portion" => "info portion",
                "article" => "article",
                _ => "profile",
            };
            let p = Problem::warning(
                XML_ID,
                format!(
                    "{what} '{id}' is not named after the module: the id is one name for the whole game, and of two of it the game keeps the first and drops the other - the game's own, or another module's loaded before - so name it '{}', or carry the module id as a word of it",
                    own_form(&id, module)
                ),
            );
            out.push(at(p, text, from + start, len));
        }
    }
}

/// `N004`: the files the module adds to the game's files (`virtual`: their paths under
/// `gamedata`, backslashes) that neither the game has nor a folder or a name of the module holds -
/// one warning per folder, naming its files. A file the game has is a replacement of it.
#[must_use]
pub fn file_names(module: &str, files: &[String], game: &dyn GameNames) -> Vec<Problem> {
    if module.is_empty() {
        return Vec::new();
    }
    let mut by_folder: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for f in files {
        let f = f.replace('/', "\\").to_ascii_lowercase();
        // what the game reads from the module's own folder is no name of the game's files
        if f.starts_with("configs\\xms\\") || is_own_path(&f, module) || game.file(&f) {
            continue;
        }
        // the bump pair of a picture the game has goes with that picture, under its name
        let base = f.strip_suffix("_bump#.dds").or_else(|| f.strip_suffix("_bump.dds"));
        if base.is_some_and(|b| game.file(&format!("{b}.dds"))) {
            continue;
        }
        let (folder, name) = f.rsplit_once('\\').unwrap_or(("", f.as_str()));
        by_folder.entry(folder.to_owned()).or_default().push(name.to_owned());
    }
    by_folder
        .into_iter()
        .map(|(folder, names)| {
            let shown: Vec<&str> = names.iter().take(8).map(String::as_str).collect();
            let more = names.len().saturating_sub(shown.len());
            let list = if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") };
            let place = if folder.is_empty() { "gamedata".to_owned() } else { format!("gamedata\\{folder}") };
            // a script's name is the name its code is reached by: it is renamed, never moved
            let fix = if folder == "scripts" {
                format!("name them '{}'", own_form("...", module))
            } else {
                format!("put them under a folder named '{module}', or name them '{}'", own_form("...", module))
            };
            Problem::warning(
                FILE,
                format!(
                    "{place}: {} new file(s) not named after the module ({list}): a path is one name for the whole game, and another module's file of that path replaces this one - {fix}",
                    names.len()
                ),
            )
        })
        .collect()
}

/// The module's id and its `[vfs]` entries.
async fn module_of(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
) -> Option<(String, Vec<(String, String)>)> {
    let service = gz_mod::ManifestService::of_session(editor, session).await.ok()?;
    let snap = service.snapshot();
    let id = snap.manifest().map(|m| m.id.clone()).filter(|id| !id.is_empty())?;
    let vfs = snap
        .doc()
        .map(|d| d.section_entries("vfs").into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect())
        .unwrap_or_default();
    Some((id, vfs))
}

/// What the game has, for the checks: the items area's reading of the linked game, `None` without
/// one (nothing can be told new then).
async fn game_of(editor: &Arc<Editor>, module: &str) -> Option<Arc<dyn GameNames>> {
    let items = crate::service::Items::of(editor).await.ok()?;
    let lookups = items.lookups(editor).await;
    if !lookups.linked {
        return None;
    }
    let roster = items.roster(editor).await;
    let own = gz_mod::sections::own_sections(editor).await;
    let library = gz_project::library_root(editor).join("gamedata");
    Some(Arc::new(crate::service::GameNamesOf {
        lookups,
        game: roster.game.clone(),
        own,
        module: module.to_ascii_lowercase(),
        library,
    }))
}

/// The names one text file of the project writes, in that file.
fn text_names(
    module: &str,
    vfs: &[(String, String)],
    rel: &RelPath,
    text: &str,
    game: &dyn GameNames,
) -> Vec<Problem> {
    let name = rel.as_str().to_ascii_lowercase();
    let in_patch = rel.parent().is_some_and(|p| p.as_str().eq_ignore_ascii_case("patch"));
    // a patch is read from the module's own `patch\` folder
    let found = if name.ends_with(".ltxp") {
        if in_patch { config_names(module, "patch", text, true, game) } else { Vec::new() }
    } else if name.ends_with(".xmlp") {
        if in_patch { xmlp_names(module, text, game) } else { Vec::new() }
    } else if let Some(virt) = virtual_path(rel.as_str(), vfs) {
        if name.ends_with(".ltx") {
            config_names(module, &virt, text, false, game)
        } else if name.ends_with(".xml") {
            xml_names(module, &virt, text, game)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    found.into_iter().map(|p| p.in_file(rel.clone())).collect()
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = gz_editor::fsx::read_opt(path).ok().flatten()?;
    Some(gz_base::text::decode_guess(&bytes).into_owned())
}

/// `asset.check` of a config, a patch or an XML file: the names it writes.
fn text_check(editor: Arc<Editor>, rel: RelPath) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let Ok(session) = editor.project() else { return Vec::new() };
        let Some((module, vfs)) = module_of(&editor, &session).await else { return Vec::new() };
        let exclude = gz_mod::ManifestService::of_session(&editor, &session)
            .await
            .ok()
            .and_then(|s| s.snapshot().manifest().map(|m| m.exclude.clone()))
            .unwrap_or_default();
        if gz_mod::outside_build(&exclude, rel.as_str()) {
            return Vec::new();
        }
        let Some(game) = game_of(&editor, &module).await else { return Vec::new() };
        let path = session.fs(&rel);
        let Some(text) = editor.exec.io(move || read_text(&path)).await else { return Vec::new() };
        editor.exec.cpu(move || text_names(&module, &vfs, &rel, &text, &*game)).await
    })
}

/// The names the module's text files write, over the build's walk: what a build says beside its
/// result (`asset.check` gives them per file).
struct TextNamesCheck;

impl BuildCheck for TextNamesCheck {
    fn stage(&self) -> &'static str {
        STAGE_TEXT
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        session: Arc<ProjectSession>,
        walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            let Some((module, vfs)) = module_of(&editor, &session).await else { return Vec::new() };
            let Some(game) = game_of(&editor, &module).await else { return Vec::new() };
            let root = session.root.clone();
            let files: Vec<RelPath> = walk
                .files
                .iter()
                .filter(|f| matches!(f.from, gz_build::FileFrom::Project))
                .map(|f| f.rel.clone())
                .filter(|r| {
                    let n = r.as_str().to_ascii_lowercase();
                    n.ends_with(".ltx") || n.ends_with(".ltxp") || n.ends_with(".xml") || n.ends_with(".xmlp")
                })
                .collect();
            editor
                .exec
                .io(move || {
                    let mut out = Vec::new();
                    for rel in files {
                        if let Some(text) = read_text(&rel.to_fs(&root)) {
                            out.extend(text_names(&module, &vfs, &rel, &text, &*game));
                        }
                    }
                    out
                })
                .await
        })
    }
}

/// The names of the files the module adds to the game's files, over the build's walk and what its
/// media assets publish.
struct FileNamesCheck;

impl BuildCheck for FileNamesCheck {
    fn stage(&self) -> &'static str {
        STAGE_FILES
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        session: Arc<ProjectSession>,
        walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            let Some((module, vfs)) = module_of(&editor, &session).await else { return Vec::new() };
            let Some(game) = game_of(&editor, &module).await else { return Vec::new() };
            let mut files: Vec<String> =
                walk.files.iter().filter_map(|f| virtual_path(f.rel.as_str(), &vfs)).collect();
            for publisher in BuildChecks::of(&editor).publishers() {
                let plan = publisher.plan(editor.clone(), session.clone()).await;
                files.extend(plan.published.iter().filter_map(|p| virtual_path(p, &vfs)));
            }
            // an asset of the editor's kinds is read by the game per module, by the module: a media
            // asset's text goes with its data, which is said by itself
            let kinds = gz_asset::Kinds::of(&editor);
            files.retain(|f| kinds.of_file(f.rsplit('\\').next().unwrap_or(f)).is_none());
            files.sort();
            files.dedup();
            editor.exec.cpu(move || file_names(&module, &files, &*game)).await
        })
    }
}

/// Puts the checks into `asset.check` and the build.
pub fn install(editor: &Editor) {
    gz_asset::Kinds::of(editor).add_text_check("module names", &["ltx", "ltxp", "xml", "xmlp"], text_check);
    let checks = BuildChecks::of(editor);
    checks.add(Arc::new(TextNamesCheck));
    checks.add(Arc::new(FileNamesCheck));
    // the names a text file writes are its own problems in asset.check; the files' names are the
    // module's, said with its manifest
    gz_asset::Kinds::of(editor).covers_build_stage(STAGE_TEXT);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Game;

    impl GameNames for Game {
        fn section(&self, name: &str) -> bool {
            ["wpn_ak74", "item_count"].iter().any(|s| s.eq_ignore_ascii_case(name))
        }
        fn in_tree(&self, file: &str) -> bool {
            file == "items\\w.ltx" || file == "misc\\squads.ltx"
        }
        fn string(&self, id: &str) -> bool {
            id == "st_game_string"
        }
        fn file(&self, path: &str) -> bool {
            [
                "textures\\ui\\ui_icons.dds",
                "textures\\wpn\\ak.dds",
                "configs\\gameplay\\character_desc_escape.xml",
            ]
            .contains(&path)
        }
        fn module_section(&self, name: &str) -> bool {
            name == "made_by_an_item"
        }
        fn other_module(&self, name: &str) -> bool {
            is_own(name, "other")
        }
    }

    fn codes(p: &[Problem]) -> Vec<(&str, Option<u32>)> {
        p.iter().map(|p| (p.code.as_str(), p.line)).collect()
    }

    #[test]
    fn a_new_section_of_a_config_or_a_patch_is_named_after_the_module() {
        let ltx = "[wpn_ak74]\ncost = 1\n[wpn_rifle] : wpn_ak74\n[wpn_od_rifle]:wpn_ak74\n[other_rifle]\n";
        let p = config_names("od", "configs\\items\\w.ltx", ltx, false, &Game);
        assert_eq!(codes(&p), [("N001", Some(3))]);
        assert_eq!(p[0].column, Some(2));
        assert!(p[0].message.contains("[wpn_rifle]") && p[0].message.contains("'od_wpn_rifle'"));
        // `configs\xms\*.ltx` goes into the system config too; a config the scripts read by its path
        // keeps its sections to itself
        assert_eq!(
            codes(&config_names("od", "configs\\xms\\a.ltx", "[logic]\n", false, &Game)),
            [("N001", Some(1))]
        );
        assert!(
            config_names("od", "configs\\scripts\\od\\guard.ltx", "[logic]\n[walker@guard]\n", false, &Game)
                .is_empty()
        );
        // a patch: the system config, a file of the tree and one out of it by #file, a deletion, a change
        // of what an item of the module makes, a section made from a parent
        let patch = "[wpn_ak74]\n[!gone]\n[new_one] : wpn_ak74\n#file misc\\death_generic.ltx\n[keep_items]\n#file misc\\squads.ltx\n[od_list]\n[squad_x]\n#file\n[item_count]\n[made_by_an_item]\n[made_by_an_item] : wpn_ak74\n";
        let p = config_names("od", "patch", patch, true, &Game);
        assert_eq!(codes(&p), [("N001", Some(3)), ("N001", Some(8)), ("N001", Some(12))]);
        // what is no config of the game's is nobody's name
        assert!(config_names("od", "textures\\x.ltx", "[a]\n", false, &Game).is_empty());
        assert!(config_names("", "configs\\xms\\a.ltx", "[a]\n", false, &Game).is_empty());
    }

    #[test]
    fn a_string_and_a_dialog_of_the_module_are_named_after_it() {
        let st = "<?xml version=\"1.0\"?>\n<string_table>\n  <string id=\"st_game_string\"><text>x</text></string>\n  <string id=\"st_mine\"><text>y</text></string>\n  <string id='od_mine'><text>z</text></string>\n</string_table>\n";
        let p = xml_names("od", "configs\\text\\eng\\st_od.xml", st, &Game);
        assert_eq!(codes(&p), [("N002", Some(4))]);
        assert_eq!(p[0].column, Some(15));
        let dialogs = "<game_dialogs>\n<dialog id=\"talk_1\">\n</dialog>\n<dialog id=\"od_talk\"/>\n<specific_character id=\"trader_x\" team=\"a\">\n</game_dialogs>\n";
        let p = xml_names("od", "configs\\gameplay\\dialogs_od.xml", dialogs, &Game);
        assert_eq!(codes(&p), [("N003", Some(2)), ("N003", Some(5))]);
        assert!(p[1].message.starts_with("specific character 'trader_x'"), "{}", p[1].message);
        assert!(xml_names("od", "configs\\ui\\x.xml", dialogs, &Game).is_empty());
        // a file of the game's path replaces it: its ids are the game's
        assert!(xml_names("od", "configs\\gameplay\\character_desc_escape.xml", dialogs, &Game).is_empty());
        // another installed module's id, an info portion
        let more = "<x>\n<dialog id=\"other_talk\"/>\n<info_portion id=\"done_it\"/>\n</x>\n";
        let p = xml_names("od", "configs\\gameplay\\info_od.xml", more, &Game);
        assert_eq!(codes(&p), [("N003", Some(3))]);
        assert!(p[0].message.starts_with("info portion 'done_it'"), "{}", p[0].message);
        // a patch of XML: what it puts into the game's files, by the file each block targets
        let patch = "<?xml version=\"1.0\"?>\n<xms-patch target=\"gameplay\\dialogs.xml\">\n  <append into=\"game_dialogs\">\n    <dialog id=\"talk_2\"/>\n  </append>\n  <append into=\"dialog[id=dm_x]\"><precondition>od.x</precondition></append>\n</xms-patch>\n<xms-patch target='text\\eng\\st_items.xml'>\n  <append into=\"string_table\"><string id=\"st_game_string\"/><string id=\"st_new\"/></append>\n</xms-patch>\n";
        let p = xmlp_names("od", patch, &Game);
        assert_eq!(codes(&p), [("N003", Some(4)), ("N002", Some(9))]);
    }

    #[test]
    fn a_new_file_is_named_after_the_module_by_a_folder_or_its_name() {
        let files = [
            "textures\\ui\\ui_icons.dds",
            "textures\\weapons\\rifle.dds",
            "textures\\weapons\\scope.dds",
            "textures\\od\\pistol.dds",
            "scripts\\od_main.script",
            "scripts\\utils2.script",
            "configs\\xms\\balance.ltx",
            "textures\\wpn\\ak_bump.dds",
            "textures\\wpn\\ak_bump#.dds",
        ]
        .map(String::from);
        let p = file_names("od", &files, &Game);
        let said: Vec<&str> = p.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(p.len(), 2, "{said:?}");
        assert!(
            said[0]
                .starts_with("gamedata\\scripts: 1 new file(s) not named after the module (utils2.script)")
        );
        assert!(said[1].starts_with(
            "gamedata\\textures\\weapons: 2 new file(s) not named after the module (rifle.dds, scope.dds)"
        ));
    }

    #[test]
    fn a_project_file_has_its_path_in_the_game() {
        let vfs = [("textures\\wpn".to_owned(), "art/weapons".to_owned())];
        assert_eq!(virtual_path("gamedata/scripts/a.script", &vfs).as_deref(), Some("scripts\\a.script"));
        assert_eq!(virtual_path("art/weapons/x/y.dds", &vfs).as_deref(), Some("textures\\wpn\\x\\y.dds"));
        assert_eq!(virtual_path("mod.ltx", &vfs), None);
        assert_eq!(virtual_path("patch/a.ltxp", &vfs), None);
    }
}
