//! The format of `mod.ltx` as text: every section and key the editor reads or writes, with its
//! values, its default and one line of what it means. `asset.reference kind=manifest` answers
//! it; a test holds it to [`crate::Manifest::parse`], so a key the model learns without its
//! words here fails the build of the tests.
//!
//! The sections of the areas that read their own (`[world]`, `[start_kit]`, `[screens]`,
//! `[screen.<id>]`, `[track.<id>]`) are described here too - an author learns the whole file from
//! one page - and held to their readers by a test in `grizzly`, which sees every crate.

use std::fmt::Write as _;
use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, Editor};
use serde_json::{Value, json};

/// Who reads a section: this crate's model, or the area that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    /// [`crate::Manifest`].
    Manifest,
    /// `gz-world`: `[world]` and `[start_kit]`.
    World,
    /// `gz-screens`: `[screens]`, `[screen.<id>]` and `[track.<id>]`.
    Screens,
    /// Not in the model: the build and the game act on it (`[vfs]`).
    Files,
}

/// One key of a section.
#[derive(Clone, Copy, Debug)]
pub struct KeyDoc {
    /// The key as written; `<...>` stands for a name the author chooses.
    pub key: &'static str,
    /// What it takes.
    pub values: &'static str,
    /// What it is when left out; `required` for a key the module cannot do without.
    pub default: &'static str,
    /// One line in words.
    pub meaning: &'static str,
}

/// One section of the manifest.
#[derive(Clone, Copy, Debug)]
pub struct SectionDoc {
    /// The section's name as written between the brackets; `<...>` stands for a name the author
    /// chooses.
    pub name: &'static str,
    pub reader: Reader,
    pub summary: &'static str,
    pub keys: &'static [KeyDoc],
}

const fn key(
    key: &'static str,
    values: &'static str,
    default: &'static str,
    meaning: &'static str,
) -> KeyDoc {
    KeyDoc { key, values, default, meaning }
}

/// The tags the Mod Browser knows (`[module] tags`).
const TAGS: &str = "story, quests, gameplay, weapons, equipment, npcs, mutants, locations, graphics, audio, interface, fixes";

const SURFACES: &str = "loading, menu, pause";

/// Every section the editor reads or writes, in the order the page tells them.
pub const SECTIONS: &[SectionDoc] = &[
    SectionDoc {
        name: "module",
        reader: Reader::Manifest,
        summary: "The module itself: its id, the texts the game's Mods menu shows, and the game it is for.",
        keys: &[
            key(
                "id",
                "a-z 0-9 _ . -",
                "required",
                "The module's id: its folder under the game's modules folder, and what other modules and \
                 mode gates call it. A project's folder name seeds it.",
            ),
            key(
                "name",
                "text",
                "the id",
                "The title the Mods menu prints. Free text: it owes nothing to the id or to the project's folder.",
            ),
            key(
                "version",
                "1 to 4 numbers separated by dots, 32 characters at most (1.2.0)",
                "1.0.0 in a new project",
                "The release number: the game's updater orders releases by it.",
            ),
            key("author", "text", "empty", "Who made it, shown in the Mods menu."),
            key(
                "description",
                "text, 4095 bytes at most, line breaks allowed (quote it)",
                "empty",
                "What the module is, shown in the Mods menu.",
            ),
            key(
                "website",
                "an https link on ap-pro.ru or moddb.com",
                "empty",
                "The module's page, opened from the Mods menu. Any other host is refused by the editor and \
                 ignored by the game.",
            ),
            key(
                "thumbnail",
                "a project-relative path to a 256 x 256 DXT1 or DXT5 .dds",
                "empty",
                "The picture the Mods menu and the Mod Browser show. mod.thumbnail from=<picture> makes \
                 thumbnail.dds at the project's root from any picture and sets this key.",
            ),
            key(
                "tags",
                TAGS,
                "empty",
                "Words the Mod Browser filters by, comma separated; any other word is refused.",
            ),
            key(
                "api",
                "1",
                "1 in a new project",
                "The version of this file's format the module is written for.",
            ),
            key(
                "target",
                "default | existing | new | any",
                "empty: the build refuses a module with no target, no mode and no [provides_mode]",
                "Which game the module is for, stated outright. default: the ordinary game (no mode key). \
                 existing: a mode of the linked game (mode = <id>). new: a mode the module adds \
                 ([provides_mode]). any: every game, for a classic mod (mode = *). A mode or a \
                 [provides_mode] section also answers the question; target = default is how the ordinary \
                 game is chosen on purpose.",
            ),
            key(
                "mode",
                "a game mode id, or *",
                "empty: the ordinary game only",
                "Limits the module's work on levels (spawns, level overlays, the AI map, the game graph) \
                 to one game mode; * lifts the limit. Files, configs and scripts mount in every game \
                 whatever the mode. The ids the linked game has are listed at the end of this page.",
            ),
            key(
                "disable_game_updates",
                "true | false",
                "false",
                "A last resort: turns off the game's own updates and bug reports for everyone who has the \
                 module. The check warns about it.",
            ),
        ],
    },
    SectionDoc {
        name: "provides_mode",
        reader: Reader::Manifest,
        summary: "A game mode the module adds: the game offers it in the new-game screen's list. Its presence is how a module says target = new.",
        keys: &[
            key(
                "id",
                "a-z 0-9 _ . -, 64 characters at most",
                "required",
                "The mode's id. The game turns new_game_<id>_mode back into it, and mode = <id> and the \
                 quests' mode checks compare against it.",
            ),
            key(
                "title",
                "text or a string table id",
                "the id",
                "What the player reads in the list. The game shows it in cp1251: characters outside it are \
                 warned about.",
            ),
        ],
    },
    SectionDoc {
        name: "requires",
        reader: Reader::Manifest,
        summary: "Modules this one needs. One line each; the editor keeps the lines as written.",
        keys: &[key(
            "<module id>",
            "a note for readers, such as >=1.0",
            "-",
            "The game reads the id: this module is switched off while that one is missing or off, and it              loads after it. The value is not checked.",
        )],
    },
    SectionDoc {
        name: "order",
        reader: Reader::Manifest,
        summary: "Where the module loads, against others.",
        keys: &[
            key("after", "module ids, comma separated", "empty", "The module loads after these."),
            key("before", "module ids, comma separated", "empty", "The module loads before these."),
        ],
    },
    SectionDoc {
        name: "conflicts",
        reader: Reader::Manifest,
        summary: "Modules that cannot be on together with this one. One line each.",
        keys: &[key(
            "<module id>",
            "*",
            "-",
            "This module is switched off while that module is installed and on. The value is not read.",
        )],
    },
    SectionDoc {
        name: "update",
        reader: Reader::Manifest,
        summary: "Where the game finds new releases.",
        keys: &[key(
            "github",
            "owner/repo",
            "empty: no updates",
            "The repository whose latest release carries the module. A pasted github.com address is \
             shortened by mod.set. Publish Release writes to it.",
        )],
    },
    SectionDoc {
        name: "exclude",
        reader: Reader::Manifest,
        summary: "What the build leaves out of the module. One entry per line, no key. The game ignores the section.",
        keys: &[key(
            "<path or pattern>",
            "a project path (a folder takes everything below it), a name at any depth (*.psd), or a path pattern (docs/*.md)",
            "empty",
            "The project may hold notes, sources and tools that never ship; editor files, .git and other \
             dot-names are left out already. mod.exclude path=<...> edits it.",
        )],
    },
    SectionDoc {
        name: "vfs",
        reader: Reader::Files,
        summary: "Publishes files under any layout: the game sees the project's file or folder at a path of its own.",
        keys: &[key(
            "<game path>",
            "a path inside the project (a file, or a folder taken whole); no ..",
            "-",
            "Written <game path> = <project path>, for example configs\\xms\\balance.ltx = tuning\\balance.ltx. \
             Mounts after the module's own gamedata folder, which is a convention, not a rule.",
        )],
    },
    SectionDoc {
        name: "vars",
        reader: Reader::Manifest,
        summary: "The module's global variables: one set every quest and behaviour graph of the module shares.",
        keys: &[key(
            "<name>",
            "a default: a number, true or false, or a quoted text",
            "-",
            "The name takes letters, digits and _. The default is kept as written; quests read the \
             variable's type from it. Another module's variable is <module id>.<name>.",
        )],
    },
    SectionDoc {
        name: "pda",
        reader: Reader::Manifest,
        summary: "The module's own PDA interface.",
        keys: &[key(
            "layout",
            "a project-relative .pdaasset",
            "empty: the stock PDA",
            "The layout the PDA shows; asset.reference kind=pda has its format.",
        )],
    },
    SectionDoc {
        name: "commands",
        reader: Reader::Manifest,
        summary: "Console commands the module adds. One line each.",
        keys: &[key(
            "<command name>",
            "<script>.<function>",
            "-",
            "The name takes a-z, 0-9 and _, 64 characters at most, and must not be a word the game has. The \
             function lives in the project's gamedata\\scripts\\<script>.script.",
        )],
    },
    SectionDoc {
        name: "command.<name>",
        reader: Reader::Manifest,
        summary: "Details of one command; only for a command [commands] declares.",
        keys: &[
            key("help", "text or a string table id", "empty", "The console's help line."),
            key(
                "args",
                "text",
                "empty",
                "What the command takes: shown by help and as the hint while typing.",
            ),
            key("cheat", "true | false", "false", "True: runs only in the game's debug mode."),
        ],
    },
    SectionDoc {
        name: "world",
        reader: Reader::World,
        summary: "What the module's mode keeps of the original game, where the player may go and which ways start closed. The section's presence declares a world; without it the game stays as it is. world.set edits it.",
        keys: &[
            key(
                "original_npcs",
                "true | false",
                "true",
                "Keep the characters the game placed by story id (traders, story people, their squads).",
            ),
            key("random_stalkers", "true | false", "true", "Keep the simulation's stalker squads."),
            key("mutants", "true | false", "true", "Keep the simulation's mutant squads."),
            key("npc_quests", "true | false", "true", "Keep the jobs squad leaders offer."),
            key(
                "story_quests",
                "true | false",
                "true",
                "Keep the tasks of named characters and the storylines.",
            ),
            key(
                "player_choice",
                "rule names, comma separated, three at most",
                "empty",
                "The rules the new-game screen lets the player change.",
            ),
            key("levels", "level names, comma separated", "empty: everywhere", "Where the rules apply."),
            key(
                "area",
                "level names, comma separated",
                "empty: anywhere",
                "The only levels the player may reach.",
            ),
            key(
                "locked_levels",
                "level names, comma separated",
                "empty",
                "Every transition on these levels starts closed.",
            ),
            key("locked", "level changer names, comma separated", "empty", "These transitions start closed."),
        ],
    },
    SectionDoc {
        name: "start_kit",
        reader: Reader::World,
        summary: "What a new game of the module's mode begins with; it replaces the new-game screen's loadout. The section's presence declares a kit: an empty one starts the player with the PDA alone. world.set edits it.",
        keys: &[
            key("money", "a number", "the new-game screen's money", "The starting money."),
            key("perks", "true | false", "true", "Whether the perks picked on the new-game screen stay."),
            key(
                "<item section>",
                "a count",
                "1",
                "An item of the game by its section name: item_section = 3. A bare name is one. Ammunition counts in boxes.",
            ),
        ],
    },
    SectionDoc {
        name: "screens",
        reader: Reader::Screens,
        summary: "How the module's screens and music share the surfaces with the game's and other modules'. screens.set edits it.",
        keys: &[
            key(
                "loading_screens",
                "mix | only",
                "mix",
                "only: this module's loading screens are the whole pool.",
            ),
            key("menu_screens", "mix | only", "mix", "As loading_screens, for the menus."),
            key("loading_music", "mix | only", "mix", "only: this module's tracks are the loading music."),
            key("menu_music", "mix | only", "mix", "As loading_music, for the menus."),
            key(
                "loading_music_end",
                "loop | next",
                "loop",
                "What a loading track does at its end: start again, or give way to another one of the list.",
            ),
            key("menu_music_end", "loop | next", "loop", "As loading_music_end, for the menus."),
        ],
    },
    SectionDoc {
        name: "screen.<id>",
        reader: Reader::Screens,
        summary: "One screen: its place, its pool and a stack of layers, bottom first. The id takes a-z, 0-9 and _.",
        keys: &[
            key(
                "show",
                SURFACES,
                "all three",
                "The surfaces it shows on; pause is the menu over a loaded level.",
            ),
            key(
                "levels",
                "level names, comma separated",
                "empty: everywhere",
                "Only while one of these levels loads or runs.",
            ),
            key("modes", "game mode ids, comma separated", "empty: every game", "Only in these game modes."),
            key(
                "weight",
                "a number",
                "1",
                "How often it comes up against the others of the pool; the game's own screen weighs 1.",
            ),
            key(
                "music",
                "any | none | video | track ids, comma separated",
                "any",
                "any: the surface's music. none: silence. video: the video's own sound (loading only). \
                 Otherwise the tracks that play with this screen.",
            ),
            key(
                "hide",
                "bar, percent, stage, tip",
                "empty",
                "What of the loading screen's interface it leaves out.",
            ),
            key(
                "layer",
                "a project-relative file",
                "-",
                "Starts a layer, drawn over the ones before it. The file's extension tells the kind: \
                 picture, sequence, GIF, video, shader (.s) or page (.html). The keys below belong to the \
                 layer they follow.",
            ),
            key(
                "fit",
                "cover | contain | stretch",
                "cover",
                "How a picture fills its place: cut to fit, whole, or pulled.",
            ),
            key("opacity", "0 to 1", "1", "How solid the layer is."),
            key("rect", "x y width height, per cent of the screen", "0 0 100 100", "The layer's place."),
            key("texture", "a project-relative .dds", "empty", "What a shader layer gets as its picture."),
            key("sound", "true | false", "true", "False keeps a video layer quiet."),
        ],
    },
    SectionDoc {
        name: "track.<id>",
        reader: Reader::Screens,
        summary: "One music track. The id takes a-z, 0-9 and _; a screen's music key names it.",
        keys: &[
            key("file", "a project-relative sound file", "required", "The track."),
            key(
                "play",
                SURFACES,
                "empty: only where a screen names it",
                "The surfaces whose shuffle it joins.",
            ),
            key("weight", "a number", "1", "How often it comes up against the others."),
            key("volume", "a number", "1", "How loud."),
        ],
    },
];

/// The section's description for a name as written in a file (`screen.night` is `screen.<id>`).
#[must_use]
pub fn section_named(name: &str) -> Option<&'static SectionDoc> {
    let lower = name.to_ascii_lowercase();
    SECTIONS.iter().find(|s| {
        s.name == lower
            || s.name.split_once('<').is_some_and(|(stem, _)| !stem.is_empty() && lower.starts_with(stem))
    })
}

/// The example: a complete manifest of a module with a mode of its own.
pub const EXAMPLE: &str = "\
[module]
id          = night_watch
name        = \"Night Watch\"
version     = 1.0.0
author      = Sid
description = \"A short story module.\"
website     = https://www.moddb.com/mods/night-watch
tags        = story, quests
api         = 1
target      = new

[provides_mode]
id          = night_watch
title       = \"Night Watch\"

[update]
github      = Sid/night-watch

[requires]
some.library = >=1.0

[order]
after       = some.overhaul

[exclude]
docs
*.psd

[vars]
kills       = 0

[commands]
nw_status   = nw_cmd.status

[command.nw_status]
help        = \"Shows how the watch is going\"
";

const SYNTAX: &str = "\
mod.ltx - the module manifest

The one file every module has, at the root of its project. LTX, not Lua: [sections] of `key = value`
lines. The game reads it and the editor reads and writes it; the editor keeps what it does not know
(an unknown key, an unknown section) exactly where it is, and rewrites only the lines it changes.

Writing it
  - Section names are read without case, keys exactly. A key written twice keeps its last value.
  - `;` starts a comment outside a quoted value (// does not: a website is https://...).
  - A text that holds `;`, quotes or line breaks is quoted: name = \"Night; Watch\", with \\\" for a
    quote and \\n for a line break inside.
  - A list is comma separated; a section of lines (requires, exclude, vars, commands) is one entry
    per line.
  - Edit the file with your own tools, or run mod.set for the sections it takes (help mod.set); check
    it with asset.check path=mod.ltx (Grizzly-Cmd.exe check mod.ltx).

First, the target: a module is for ONE game
  A finished module states which game it is for, and the build refuses one that does not. Pick one:
    ordinary game        [module] target = default
    a mode of the game   [module] target = existing   and   mode = <id>   (ids at the end of this page)
    a mode of its own    [module] target = new        and a [provides_mode] section with id = <id>
    every game, classic  [module] target = any        and   mode = *
  mod.set sets them: mod.set set={\"module\":{\"target\":\"default\"}} (setting mode also sets target
  to existing, and a [provides_mode] to new, unless target is given).
";

const TAIL: &str = "\
Left as written
  [budget] (spawns) and [redirects] belong to the game; the editor does not read them and keeps them.
  The game's own rules for every section are in its docs/dead-air/MODDING.md.
";

/// Wraps `text` at `width` columns, every line after `indent` spaces.
fn wrapped(out: &mut String, text: &str, indent: usize, width: usize) {
    let pad = " ".repeat(indent);
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && indent + line.len() + 1 + word.len() > width {
            let _ = writeln!(out, "{pad}{line}");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        let _ = writeln!(out, "{pad}{line}");
    }
}

fn section_text(out: &mut String, s: &SectionDoc) {
    let _ = writeln!(out, "[{}]", s.name);
    wrapped(out, s.summary, 2, 104);
    for k in s.keys {
        let _ = writeln!(out);
        let _ = writeln!(out, "  {} = {}", k.key, k.values);
        let mut meaning = k.meaning.to_owned();
        match k.default {
            "" | "-" => {}
            "required" => meaning.push_str(" Required."),
            d => {
                let _ = write!(meaning, " Default: {d}.");
            }
        }
        wrapped(out, &meaning, 6, 104);
    }
    let _ = writeln!(out);
}

/// The page without what only the linked game knows.
#[must_use]
pub fn reference_text() -> String {
    let mut out = String::from(SYNTAX);
    let _ = writeln!(out);
    for s in SECTIONS {
        section_text(&mut out, s);
    }
    out.push_str("Example\n");
    for line in EXAMPLE.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "  {line}");
        }
    }
    out.push('\n');
    out.push_str(TAIL);
    out
}

/// What the linked game adds: the modes it has, by the names a player reads.
async fn modes_text(ed: &Arc<Editor>) -> String {
    let mut args = Args::new();
    args.insert("type".to_owned(), json!("mode"));
    args.insert("limit".to_owned(), json!(200));
    let reply = ed.call("game.lookup", args, Origin::Mcp).await;
    let known = reply.get("ok").and_then(Value::as_bool) == Some(true)
        && reply.get("available").and_then(Value::as_bool) == Some(true);
    let mut out = String::from("\nModes of the linked game (mode = <id>)\n");
    if !known {
        wrapped(
            &mut out,
            "No game is linked, or its new-game screen could not be read: link one to list its modes              (game.lookup type=mode lists them).",
            2,
            104,
        );
        return out;
    }
    let entries = reply.get("entries").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    if entries.is_empty() {
        out.push_str("  The linked game has no mode besides the ordinary game: target = default.\n");
    }
    for e in entries {
        let id = e.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = e.get("name").and_then(Value::as_str).unwrap_or_default();
        let from = e.get("extra").and_then(Value::as_str).filter(|x| *x != "game");
        let _ = write!(out, "  {id}");
        if !name.is_empty() && name != id {
            let _ = write!(out, " - {name}");
        }
        if let Some(from) = from {
            let _ = write!(out, " (from the module {from})");
        }
        out.push('\n');
    }
    out
}

/// `asset.reference kind=manifest`: the page, and the modes of the linked game when one is
/// known.
pub async fn reference(ed: Arc<Editor>) -> String {
    let text = reference_text();
    // the modes go before the closing note
    let (head, tail) = text.split_at(text.rfind("Left as written").unwrap_or(text.len()));
    format!("{head}{}\n{tail}", modes_text(&ed).await.trim_start_matches('\n'))
}

/// Whether a word names the manifest as a kind of asset.
#[must_use]
pub fn is_manifest_word(word: &str) -> bool {
    let w = word.trim().to_ascii_lowercase();
    matches!(w.as_str(), "manifest" | "module manifest" | "mod.ltx")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{Manifest, ManifestDoc, validate};

    /// The string literals a source line opens a match arm with (`"id" =>`, `"x" if ...`).
    fn arm_literal(line: &str) -> Option<&str> {
        let rest = line.trim_start().strip_prefix('"')?;
        let end = rest.find('"')?;
        let after = rest.get(end + 1..)?;
        (after.starts_with(" =>") || after.starts_with(" if ")).then(|| rest.get(..end)).flatten()
    }

    /// The keys and the sections [`Manifest::parse`] matches, read from its source.
    fn parsed_names() -> (Vec<String>, Vec<String>) {
        let src = include_str!("model.rs");
        let from = src.find("pub fn parse(doc").expect("parse is in model.rs");
        let to = src.find("pub fn title").expect("title follows parse");
        let (mut keys, mut sections) = (Vec::new(), Vec::new());
        for line in src.get(from..to).unwrap_or("").lines() {
            if let Some(name) = arm_literal(line) {
                if line.contains("=> Sec::") {
                    sections.push(name.to_owned());
                } else {
                    keys.push(name.to_owned());
                }
            }
            // a section with one key is read as `if key == "layout"`
            if let Some(at) = line.find("key == \"") {
                let rest = line.get(at + 8..).unwrap_or("");
                if let Some(end) = rest.find('"') {
                    keys.push(rest.get(..end).unwrap_or("").to_owned());
                }
            }
            if line.contains("starts_with(\"command.\")") {
                sections.push("command.<name>".to_owned());
            }
        }
        (keys, sections)
    }

    #[test]
    fn every_section_and_key_the_model_reads_is_in_the_page() {
        let (keys, sections) = parsed_names();
        // the scan itself must see the model: a parser rewritten so that it finds nothing fails here
        assert!(keys.len() >= 20 && sections.len() >= 10, "{keys:?} {sections:?}");
        let page = reference_text();
        for s in &sections {
            assert!(
                SECTIONS.iter().any(|d| d.name == s && d.reader == Reader::Manifest),
                "the model reads [{s}], the reference has no section of that name"
            );
            assert!(page.contains(&format!("[{s}]")), "[{s}] is not in the text");
        }
        for k in &keys {
            assert!(
                SECTIONS
                    .iter()
                    .filter(|d| d.reader == Reader::Manifest)
                    .flat_map(|d| d.keys)
                    .any(|d| d.key == k),
                "the model reads the key '{k}', the reference has no words for it"
            );
        }
    }

    #[test]
    fn no_key_is_invented() {
        let (keys, sections) = parsed_names();
        for s in SECTIONS.iter().filter(|s| s.reader == Reader::Manifest) {
            assert!(sections.iter().any(|n| n == s.name), "the model does not read [{}]", s.name);
            for k in s.keys.iter().filter(|k| !k.key.starts_with('<')) {
                assert!(keys.iter().any(|n| n == k.key), "the model does not read [{}] {}", s.name, k.key);
            }
        }
    }

    #[test]
    fn the_example_is_a_valid_manifest_of_every_section_it_shows() {
        let m = Manifest::parse(&ManifestDoc::parse(EXAMPLE));
        assert!(validate(&m).is_empty(), "{:?}", validate(&m));
        assert_eq!(m.id, "night_watch");
        assert_eq!(m.target, "new");
        assert_eq!(m.provides_modes.len(), 1);
        assert_eq!((m.update_github.as_str(), m.after.len(), m.exclude.len()), ("Sid/night-watch", 1, 2));
        assert_eq!((m.vars.len(), m.commands.len(), m.requires.len(), m.tags.len()), (1, 1, 1, 2));
        assert!(m.commands[0].declared && !m.commands[0].help.is_empty());
        assert!(crate::validate_website(&m.website) && crate::validate_version(&m.version));
        // every section it names is one the page describes
        for name in ManifestDoc::parse(EXAMPLE).sections() {
            assert!(section_named(name).is_some(), "[{name}] has no words");
        }
    }

    #[test]
    fn the_page_says_the_target_values_and_the_command() {
        let page = reference_text();
        for word in
            ["default", "existing", "new", "any", "mod.set set=", "target = default", "[provides_mode]"]
        {
            assert!(page.contains(word), "{word}");
        }
        assert!(
            is_manifest_word("module manifest") && is_manifest_word("MOD.LTX") && !is_manifest_word("quest")
        );
        assert_eq!(section_named("screen.night").map(|s| s.name), Some("screen.<id>"));
        assert_eq!(section_named("World").map(|s| s.name), Some("world"));
        assert!(section_named("nothing").is_none());
    }
}
