//! `AGENTS.md`: the page at a project's root for whoever works on its files without the editor -
//! an agent, a script, the author in a text editor (`docs/ASSETS.md` section 5). Made from the
//! project (its id, its module, the kinds of assets it holds and where), written when a project
//! is created and once when this editor first opens a project that has none; never again by
//! itself after it was deleted or changed (`project.guide` rewrites it on request).
//!
//! It names no place outside the project: the editor's folder is "the editor's folder".

use std::collections::BTreeMap;
use std::path::Path;

use gz_ltx::LtxDoc;

use crate::rules;

/// The file's name at the project's root.
pub const FILE: &str = "AGENTS.md";

/// The kinds the page counts: extension, what the author calls several.
const KINDS: [(&str, &str); 14] = [
    (".nqasset", "Quests"),
    (".behasset", "Behaviour graphs"),
    (".npcasset", "Characters"),
    (".pdaasset", "PDA interfaces"),
    (".itemasset", "Items"),
    (".tradeasset", "Traders"),
    (".lootasset", "Loot tables"),
    (".recipeasset", "Recipes"),
    (".scene", "Scenes"),
    (".level", "Scenes in the X-Ray SDK format"),
    (".group", "Object groups"),
    (".meshasset", "Models"),
    (".texasset", "Textures"),
    (".sndasset", "Sounds"),
];

/// What the page says about the project's files: per kind, how many and in which folders.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Holdings {
    /// The kind (its place in `KINDS`) -> how many, and their folders project-relative with `/`
    /// (the root as `.`).
    pub kinds: BTreeMap<usize, (u32, Vec<String>)>,
}

fn walk(root: &Path, dir: &Path, depth: u32, out: &mut Holdings) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        if rules::is_tooling_entry(&name) || t.is_symlink() {
            continue;
        }
        if depth == 0 && rules::is_editor_only_entry(&name) {
            continue;
        }
        if t.is_dir() {
            walk(root, &e.path(), depth + 1, out);
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let Some(i) = KINDS.iter().position(|(ext, _)| lower.ends_with(ext)) else { continue };
        let folder = dir
            .strip_prefix(root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| ".".to_owned());
        let slot = out.kinds.entry(i).or_default();
        slot.0 += 1;
        if !slot.1.contains(&folder) {
            slot.1.push(folder);
        }
    }
}

/// What the project at `root` holds. Blocking: one walk of the project's folders.
#[must_use]
pub fn holdings(root: &Path) -> Holdings {
    let mut out = Holdings::default();
    walk(root, root, 0, &mut out);
    for (_, folders) in out.kinds.values_mut() {
        folders.sort();
    }
    out
}

/// The page for the project at `root` whose file is `file_name` and id `id`.
#[must_use]
pub fn text(root: &Path, file_name: &str, id: &str) -> String {
    let manifest = std::fs::read(root.join("mod.ltx"))
        .map(|b| LtxDoc::parse(&gz_base::text::decode_guess(&b)))
        .unwrap_or_default();
    let module = |key: &str| {
        manifest.get("module", key).map(|v| gz_ltx::unquote(v).trim().to_owned()).unwrap_or_default()
    };
    let (module_id, module_name) = (module("id"), module("name"));
    let title = if module_name.is_empty() {
        file_name.trim_end_matches(".grizzly").to_owned()
    } else {
        module_name.clone()
    };
    let held = holdings(root);

    let mut out = String::new();
    let mut line = |s: &str| {
        out.push_str(s);
        out.push('\n');
    };
    line(&format!("# {title}"));
    line("");
    line("A Grizzly Editor project: a module for Dead Air Refined. The files are the source of truth -");
    line("edit them directly. An editor that has the project open follows every change.");
    line("");
    line(&format!("- Project file: `{file_name}` (id `{id}`)."));
    if module_id.is_empty() {
        line("- Module manifest: `mod.ltx` (LTX, read by the game).");
    } else {
        line(&format!("- Module manifest: `mod.ltx` - module `{module_id}` (LTX, read by the game)."));
    }
    if held.kinds.is_empty() {
        line(
            "- No quests, characters, PDA interfaces, scenes or models yet: a file of the kind anywhere is one.",
        );
    }
    for (i, (count, folders)) in &held.kinds {
        let shown: Vec<String> = folders.iter().take(4).map(|f| format!("`{f}/`")).collect();
        let more = if folders.len() > 4 {
            format!(" and {} more folders", folders.len() - 4)
        } else {
            String::new()
        };
        line(&format!("- {}: {count} in {}{more}.", KINDS[*i].1, shown.join(", ")));
    }
    line("");
    line("## The files");
    line("");
    line("Every asset is Lua data: `return { ... }` of tables, strings, numbers and booleans, no code.");
    line(
        "The first key names the kind and its version (`nq = 1`, `npc = 1`, `pda = 1`, `mesh = 1`); keys the editor",
    );
    line("does not know are kept, comments survive every save, things are named by `id`, never by position.");
    line("An inventory item - a new one or a change of a game item - is an `.itemasset` (`item = 1`):");
    line("`Grizzly-Cmd.exe reference item` lists its keys and the linked game's catalog of item keys.");
    line(
        "A file says its own thing: who sells an item, who carries it and what makes it are a `.tradeasset`,",
    );
    line(
        "a `.lootasset` and a `.recipeasset`, never keys of the item (`reference trader`, `loot`, `recipes`).",
    );
    line(
        "Game-format textures, sounds and models under `gamedata\\` ship as they are, an asset beside each (`asset.import`).",
    );
    line("");
    line(
        "`mod.ltx` is LTX, not Lua: `Grizzly-Cmd.exe reference manifest` lists every section and key. A module",
    );
    line("targets ONE game mode: set `[module] target = default` (the ordinary game) or `mode = <id>`.");
    line("");
    line(
        "A level of the game is worked on as a scene over it (`base = \"<level>\"`: the files hold only what the",
    );
    line(
        "module adds): `Grizzly-Cmd.exe help level-design` is the guide, `run asset.find kind=level` lists levels.",
    );
    line("");
    line("## Commands");
    line("");
    line("`Grizzly-Cmd.exe` is in the editor's folder (or on PATH); run it in this folder:");
    line("");
    line(
        "- `Grizzly-Cmd.exe check [path]` - every problem with file, line and column; exit code 1 on an error.",
    );
    line("- `Grizzly-Cmd.exe reference <kind>` - the format of a kind (`manifest`, `quest`, `pda`, ...) and");
    line("  what the linked game adds.");
    line("- `Grizzly-Cmd.exe build` - the module into the game, as the editor's Build button does.");
    line("");
    line("MCP (the editor's control endpoint) is for what files cannot do: the window and the 3D scene.");
    line("");
    line("## Building a place in a level");
    line("");
    line("A scene is `<name>.scene` and a folder of Lua files (`objects.lua`, `terrain.lua`, ...); with");
    line(
        "`base = \"<level>\"` it lies over a level of the game and holds what the module adds. A spawn point takes any",
    );
    line(
        "section - the module's characters and items are `<module>_<id>`. Models, textures and sounds are `.meshasset`,",
    );
    line(
        "`.texasset` and `.sndasset` beside their sources (`source = \"game:<model>.ogf\"` dresses a model of the game). Never",
    );
    line(
        "write a coordinate from memory: `shot.map` (a map from above), `scene.query` (the ground, heights, rays,",
    );
    line("fit), `scene.align` (onto the ground), `scene.audit` (what floats, overlaps or will not ship),");
    line("`level.build`. `Grizzly-Cmd.exe help level-design` is the guide in short.");
    line("");
    line("The editor wrote this page once. Edit or delete it freely: it is not written again.");
    out
}

/// Writes the page unless the project has one: `Ok(true)` when it was written.
pub fn write_once(root: &Path, file_name: &str, id: &str) -> std::io::Result<bool> {
    let path = root.join(FILE);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Ok(false);
    }
    gz_editor::fsx::write_atomic(&path, text(root, file_name, id).as_bytes())?;
    Ok(true)
}

/// Writes the page again, whatever is there (`project.guide`).
pub fn rewrite(root: &Path, file_name: &str, id: &str) -> std::io::Result<()> {
    gz_editor::fsx::write_atomic(&root.join(FILE), text(root, file_name, id).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_says_what_the_project_holds_and_names_nothing_outside_it() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        std::fs::write(root.join("mod.ltx"), "[module]\r\nid = night\r\nname = \"Night Watch\"\r\n").unwrap();
        for f in [
            "quests/a.nqasset",
            "quests/b.nqasset",
            "npc/sid.npcasset",
            ".git/x.nqasset",
            "temp/c.nqasset",
            "models/hut.meshasset",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "return {}").unwrap();
        }
        let text = text(root, "Night.grizzly", "7c1e6c1e-5c0f-4f6e-9f43-2d5b1c9a0e11");
        let lines = text.lines().count();
        assert!((20..=50).contains(&lines), "{lines} lines:\n{text}");
        assert!(text.contains(".tradeasset") && text.contains("reference trader"), "{text}");
        assert!(text.starts_with("# Night Watch\n"), "{text}");
        assert!(text.contains("module `night`"), "{text}");
        assert!(text.contains("- Quests: 2 in `quests/`."), "{text}");
        assert!(text.contains("- Characters: 1 in `npc/`."), "{text}");
        assert!(text.contains("- Models: 1 in `models/`."), "{text}");
        assert!(text.contains("help level-design") && text.contains("scene.audit"), "{text}");
        // program-files: in memory
        assert!(text.contains("Grizzly-Cmd.exe check"), "{text}");
        // the manifest is a kind too, and the one thing a new module must state
        assert!(text.contains("reference manifest") && text.contains("target = default"), "{text}");
        let root_text = root.display().to_string();
        assert!(!text.contains(&root_text) && !text.contains(":\\"), "{text}");
        // once: an existing page is the author's
        assert!(write_once(root, "Night.grizzly", "x").unwrap());
        std::fs::write(root.join(FILE), "mine").unwrap();
        assert!(!write_once(root, "Night.grizzly", "x").unwrap());
        assert_eq!(std::fs::read_to_string(root.join(FILE)).unwrap(), "mine");
    }
}
