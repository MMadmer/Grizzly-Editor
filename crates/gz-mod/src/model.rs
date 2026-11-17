//! The typed view of `mod.ltx`: the manifest's fields, read the way the manifest loader
//! reads it. Read-only: every change is an [`crate::Edit`] of the document.

use std::sync::Arc;

use crate::doc::{Item, ManifestDoc, entry_line};
use crate::text::{display_value, split_list, text_value};

/// The manifest's file name at the project root.
pub const FILE: &str = "mod.ltx";

/// A game mode the module declares (`[provides_mode]`): its id and its title (a string id or
/// text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvidesMode {
    pub id: String,
    pub title: String,
}

/// A global variable of `[vars]`: `name = default`, the default as written (quotes included) -
/// the quests read its type from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobalVar {
    pub name: String,
    pub value: String,
}

/// A playlist of the module's radio music (`[playlist.<id>]`): its id and its title (a string id
/// or text), what a radio or a quest is bound to by `<module>.<id>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RadioPlaylist {
    pub id: String,
    pub title: String,
}

/// A console command: a `[commands]` line and its `[command.<name>]` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    /// `<script>.<function>`.
    pub function: String,
    /// Decoded text or a string table id.
    pub help: String,
    pub args: String,
    /// Only in Dead Air's debug mode (`-dbg`).
    pub cheat: bool,
    /// `false`: only a `[command.<name>]` section names it.
    pub declared: bool,
    /// The unknown lines of its section, as written.
    pub extra: Vec<String>,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            name: String::new(),
            function: String::new(),
            help: String::new(),
            args: String::new(),
            cheat: false,
            declared: true,
            extra: Vec::new(),
        }
    }
}

impl Command {
    /// Whether the command needs a section of its own.
    #[must_use]
    pub fn has_details(&self) -> bool {
        !self.help.is_empty() || !self.args.is_empty() || self.cheat || !self.extra.is_empty()
    }

    /// The lines of its `[command.<name>]` section, as `Save` writes them.
    #[must_use]
    pub fn section_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.help.is_empty() {
            out.push(entry_line("help", &display_value(&self.help, false), 11));
        }
        if !self.args.is_empty() {
            out.push(entry_line("args", &display_value(&self.args, false), 11));
        }
        if self.cheat {
            out.push(entry_line("cheat", "true", 11));
        }
        out.extend(self.extra.iter().cloned());
        out
    }
}

/// `mod.ltx` read into the manifest's fields. Unknown keys and sections are not here:
/// they stay in the [`ManifestDoc`], as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    /// Decoded: a quoted value is unquoted.
    pub name: String,
    pub version: String,
    pub author: String,
    /// May hold real line breaks.
    pub description: String,
    pub website: String,
    /// `[links]` as written, by [`crate::LinkKind::index`]. Read through [`Manifest::link`]: the
    /// older `website` stands in for the kind it is.
    pub links: [String; 6],
    /// `[module] thumbnail`, project-relative with `\` separators as written.
    pub thumbnail: String,
    pub tags: Vec<String>,
    /// `[update] github`: owner/repo.
    pub update_github: String,
    pub api: String,
    /// `[module] mode`: the game mode the module's work on levels is limited to; `*` lifts the
    /// limit (a classic module's); empty = the ordinary game only.
    pub mode: String,
    /// `default`, `existing`, `new` (`any` for a classic mod): the targeting choice. Empty =
    /// never decided, and the build refuses that.
    pub target: String,
    pub disable_game_updates: bool,
    /// `[requires]` lines as written (`other.mod = >=1.0`).
    pub requires: Vec<String>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub conflicts: Vec<String>,
    pub provides_modes: Vec<ProvidesMode>,
    /// Unknown `[provides_mode]` lines.
    pub provides_extra: Vec<String>,
    /// `[exclude]` entries as written.
    pub exclude: Vec<String>,
    /// `[vars]`, in the author's order. Shared so that a snapshot of the globals costs nothing.
    pub vars: Arc<[GlobalVar]>,
    /// `[pda] layout`: a project-relative `.pdaasset`, empty when the module leaves the PDA alone.
    pub pda_layout: String,
    pub commands: Vec<Command>,
    /// `[playlist.<id>]`, in the author's order: the radio's stations the module brings (the
    /// Screens & Music tab owns the rest of those sections).
    pub playlists: Arc<[RadioPlaylist]>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sec {
    None,
    Module,
    Requires,
    Order,
    Conflicts,
    Provides,
    Update,
    Links,
    Exclude,
    Vars,
    Pda,
    Commands,
    Command(usize),
    Playlist(usize),
    /// Read raw by the areas that own them, or unknown.
    Other,
}

impl Manifest {
    /// The link of a kind: what `[links]` says, else the older `website` when it is of that kind.
    #[must_use]
    pub fn link(&self, kind: crate::LinkKind) -> &str {
        let written = self.links.get(kind.index()).map_or("", String::as_str);
        if written.is_empty() && crate::website_kind(&self.website) == Some(kind) {
            &self.website
        } else {
            written
        }
    }

    /// The module's own page, which the Mod Browser asks for: AP-PRO's, else ModDB's.
    #[must_use]
    pub fn page(&self) -> &str {
        let ap_pro = self.link(crate::LinkKind::ApPro);
        if ap_pro.is_empty() { self.link(crate::LinkKind::ModDb) } else { ap_pro }
    }
}

fn truthy(v: &str) -> bool {
    v.eq_ignore_ascii_case("true")
        || v == "1"
        || v.eq_ignore_ascii_case("yes")
        || v.eq_ignore_ascii_case("on")
}

impl Manifest {
    /// Reads the model out of a document, line by line: section names
    /// without case, keys exactly, a key written twice keeps the last value.
    #[must_use]
    pub fn parse(doc: &ManifestDoc) -> Self {
        let mut m = Manifest::default();
        let mut vars: Vec<GlobalVar> = Vec::new();
        let mut playlists: Vec<RadioPlaylist> = Vec::new();
        let mut sec = Sec::None;
        for item in doc.items() {
            let (key, value, code) = match item {
                Item::Section(name) => {
                    let lower = name.to_ascii_lowercase();
                    sec = match lower.as_str() {
                        "module" => Sec::Module,
                        "requires" => Sec::Requires,
                        "order" => Sec::Order,
                        "conflicts" => Sec::Conflicts,
                        "provides_mode" => Sec::Provides,
                        "update" => Sec::Update,
                        "links" => Sec::Links,
                        "exclude" => Sec::Exclude,
                        "vars" => Sec::Vars,
                        "pda" => Sec::Pda,
                        "commands" => Sec::Commands,
                        s if s.len() > 8 && s.starts_with("command.") => {
                            let name = s.get(8..).unwrap_or("");
                            match m.commands.iter().position(|c| c.name.eq_ignore_ascii_case(name)) {
                                Some(at) => Sec::Command(at),
                                None => {
                                    m.commands.push(Command {
                                        name: name.to_owned(),
                                        declared: false,
                                        ..Command::default()
                                    });
                                    Sec::Command(m.commands.len() - 1)
                                }
                            }
                        }
                        s if s.len() > 9 && s.starts_with("playlist.") => {
                            let id = s.get(9..).unwrap_or("");
                            match playlists.iter().position(|p| p.id.eq_ignore_ascii_case(id)) {
                                Some(at) => Sec::Playlist(at),
                                None => {
                                    playlists.push(RadioPlaylist { id: id.to_owned(), title: String::new() });
                                    Sec::Playlist(playlists.len() - 1)
                                }
                            }
                        }
                        _ => Sec::Other,
                    };
                    continue;
                }
                Item::Other => continue,
                Item::Entry { key, value, code } => (key, value, code),
            };
            match sec {
                Sec::None | Sec::Other => {}
                Sec::Module => match key {
                    "id" => m.id = value.to_owned(),
                    "name" => m.name = text_value(value),
                    "version" => m.version = value.to_owned(),
                    "author" => m.author = text_value(value),
                    "description" => m.description = text_value(value),
                    "website" => m.website = value.to_owned(),
                    // as the game reads it: true in any case, or 1
                    "disable_game_updates" => {
                        m.disable_game_updates = value.eq_ignore_ascii_case("true") || value == "1";
                    }
                    "thumbnail" => m.thumbnail = value.to_owned(),
                    "tags" => m.tags = split_list(value, ','),
                    "api" => m.api = value.to_owned(),
                    "mode" => m.mode = value.to_owned(),
                    "target" => m.target = value.to_owned(),
                    _ => {}
                },
                Sec::Order => match key {
                    "after" => m.after = split_list(value, ','),
                    "before" => m.before = split_list(value, ','),
                    _ => {}
                },
                Sec::Update => {
                    if key == "github" {
                        m.update_github = value.to_owned();
                    }
                }
                // a key per service; the names are the reference's, held together by its tests
                Sec::Links => {
                    let kind = match key {
                        "ap_pro" => Some(crate::LinkKind::ApPro),
                        "moddb" => Some(crate::LinkKind::ModDb),
                        "telegram" => Some(crate::LinkKind::Telegram),
                        "discord" => Some(crate::LinkKind::Discord),
                        "boosty" => Some(crate::LinkKind::Boosty),
                        "patreon" => Some(crate::LinkKind::Patreon),
                        _ => None,
                    };
                    if let Some(slot) = kind.and_then(|k| m.links.get_mut(k.index())) {
                        value.clone_into(slot);
                    }
                }
                // each `id` line starts a declared mode, `title` fills the last one
                Sec::Provides => match key {
                    "id" => {
                        m.provides_modes.push(ProvidesMode { id: value.to_owned(), title: String::new() })
                    }
                    "title" if !m.provides_modes.is_empty() => {
                        if let Some(last) = m.provides_modes.last_mut() {
                            last.title = value.to_owned();
                        }
                    }
                    _ => m.provides_extra.push(code.to_owned()),
                },
                Sec::Requires => m.requires.push(code.to_owned()),
                Sec::Conflicts => m.conflicts.push(code.to_owned()),
                Sec::Exclude => m.exclude.push(code.to_owned()),
                Sec::Vars => {
                    if !key.is_empty() {
                        vars.push(GlobalVar { name: key.to_owned(), value: value.to_owned() });
                    }
                }
                Sec::Pda => {
                    if key == "layout" {
                        m.pda_layout = value.to_owned();
                    }
                }
                // every line is a command of its own: a name declared twice stays twice, for the
                // check to name it
                Sec::Commands => {
                    if key.is_empty() {
                        continue;
                    }
                    if let Some(c) =
                        m.commands.iter_mut().find(|c| !c.declared && c.name.eq_ignore_ascii_case(key))
                    {
                        c.name = key.to_owned();
                        c.function = value.to_owned();
                        c.declared = true;
                    } else {
                        m.commands.push(Command {
                            name: key.to_owned(),
                            function: value.to_owned(),
                            ..Command::default()
                        });
                    }
                }
                Sec::Playlist(at) => {
                    if key == "title"
                        && let Some(p) = playlists.get_mut(at)
                    {
                        p.title = text_value(value);
                    }
                }
                Sec::Command(at) => {
                    if let Some(c) = m.commands.get_mut(at) {
                        match key {
                            "help" => c.help = text_value(value),
                            "args" => c.args = text_value(value),
                            "cheat" => c.cheat = truthy(value),
                            _ => c.extra.push(code.to_owned()),
                        }
                    }
                }
            }
        }
        m.vars = vars.into();
        m.playlists = playlists.into();
        m
    }

    /// The module's display name: its name, or its id without one.
    #[must_use]
    pub fn title(&self) -> &str {
        if self.name.is_empty() { &self.id } else { &self.name }
    }
}

/// A project name as a module id: lower-cased, every byte outside `[a-z0-9_.-]` made `_`, 127
/// bytes at most; `unnamed_mod` for nothing.
#[must_use]
pub fn derive_id(project_name: &str) -> String {
    let mut out = String::with_capacity(project_name.len().min(127));
    for b in project_name.bytes().take(127) {
        let c = b.to_ascii_lowercase();
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-');
        out.push(if ok { char::from(c) } else { '_' });
    }
    if out.is_empty() { "unnamed_mod".to_owned() } else { out }
}

/// The `mod.ltx` a project gets when it has none:
/// the id and the name from the folder, version 1.0.0, api 1. Nothing else - a module has
/// whatever layout its author gives it.
#[must_use]
pub fn scaffold_text(project_name: &str) -> String {
    let mut t = String::from("[module]\r\n");
    for (key, value) in [
        ("id", derive_id(project_name)),
        ("name", display_value(project_name, false)),
        ("version", "1.0.0".to_owned()),
        ("api", "1".to_owned()),
    ] {
        t.push_str(&format!("{key:<11} = {value}\r\n"));
    }
    t
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const FULL: &str = "[module]\r\nid          = od\r\nname        = \"Overnight; Delivery\"\r\nversion     = 1.2.0\r\nauthor      = Madmer\r\ndescription = \"Line one\\nLine \\\"two\\\"\"\r\nwebsite     = https://ap-pro.ru/stuff/x\r\ntags        = story, quests\r\napi         = 1\r\nmode        = od_mode\r\ntarget      = new\r\nengine_min  = 1.4\r\n\r\n[update]\r\ngithub      = Madmer/od\r\n\r\n[requires]\r\nother.mod = >=1.0 ; why\r\n\r\n[order]\r\nafter       = a, b\r\n\r\n[provides_mode]\r\nid          = od_mode\r\ntitle       = od_mode_title\r\nweird = 1\r\n\r\n[exclude]\r\ndocs\r\n*.psd\r\n\r\n[vars]\r\nkills          = 0\r\nmet            = \"1\"\r\n\r\n[pda]\r\nlayout      = ui\\od.pdaasset\r\n\r\n[command.status]\r\nhelp        = \"Shows; it\"\r\ncheat       = true\r\nodd = 1\r\n\r\n[commands]\r\nStatus          = od.status\r\nstatus          = od.again\r\n\r\n[world]\r\nmutants = false\r\n";

    #[test]
    fn reads_every_section() {
        let m = Manifest::parse(&ManifestDoc::parse(FULL));
        assert_eq!(m.id, "od");
        assert_eq!(m.name, "Overnight; Delivery");
        assert_eq!(m.description, "Line one\nLine \"two\"");
        assert_eq!(m.website, "https://ap-pro.ru/stuff/x");
        assert_eq!(m.tags, ["story", "quests"]);
        assert_eq!(m.update_github, "Madmer/od");
        assert_eq!(m.requires, ["other.mod = >=1.0"]);
        assert_eq!(m.after, ["a", "b"]);
        assert_eq!(m.provides_modes, [ProvidesMode { id: "od_mode".into(), title: "od_mode_title".into() }]);
        assert_eq!(m.provides_extra, ["weird = 1"]);
        assert_eq!(m.exclude, ["docs", "*.psd"]);
        assert_eq!(
            &*m.vars,
            [
                GlobalVar { name: "kills".into(), value: "0".into() },
                GlobalVar { name: "met".into(), value: "\"1\"".into() }
            ]
        );
        assert_eq!(m.pda_layout, "ui\\od.pdaasset");
        // the section read first made the entry; its [commands] line declared it
        assert_eq!(m.commands.len(), 2);
        assert_eq!(m.commands[0].name, "Status");
        assert!(m.commands[0].declared && m.commands[0].cheat);
        assert_eq!(m.commands[0].help, "Shows; it");
        assert_eq!(m.commands[0].extra, ["odd = 1"]);
        assert_eq!(m.commands[1].function, "od.again");
    }

    #[test]
    fn reads_the_radio_playlists() {
        let text = "[module]\r\nid = od\r\n[playlist.Road_Songs]\r\ntitle  = \"Road songs\"\r\ntracks = a, b\r\n[playlist.night]\r\ntracks = c\r\n";
        let m = Manifest::parse(&ManifestDoc::parse(text));
        assert_eq!(
            &*m.playlists,
            [
                RadioPlaylist { id: "road_songs".into(), title: "Road songs".into() },
                RadioPlaylist { id: "night".into(), title: String::new() }
            ]
        );
    }

    #[test]
    fn scaffold_text_is_fixed() {
        assert_eq!(
            scaffold_text("My Mod"),
            "[module]\r\nid          = my_mod\r\nname        = My Mod\r\nversion     = 1.0.0\r\napi         = 1\r\n"
        );
        assert_eq!(derive_id(""), "unnamed_mod");
        assert_eq!(derive_id("Ночь"), "________");
        let m = Manifest::parse(&ManifestDoc::parse(&scaffold_text("x;y")));
        assert_eq!(m.name, "x;y");
    }
}
