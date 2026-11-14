//! The Mod Settings form as data: what the tab shows of `mod.ltx`, its checks and what an edit
//! sends. Pure - no egui, no editor.
//!
//! The C++ specification is `EditorModManifest.cpp` from `SSettingsForm` to `SaveSettings`
//! (`FormFromManifest`, `SyncSettingsForm`, `SettingsKind`, `CheckSettings`, `CheckRelations`,
//! `CheckCommands`, `PageTone`). The tab saves as it is edited: a form that differs from what
//! it was loaded from, holds no problem that blocks and has no field being typed in, is sent as
//! the commands [`plan`] makes of it.

use gz_editor::{Args, Value};
use gz_mod::text::outside_cp1251;
use gz_mod::{
    Command, Manifest, has_script_function, normalize_github_repo, valid_command_function,
    valid_command_name, validate_github_repo, validate_id, validate_mode_id, validate_version,
    validate_website,
};
use gz_ui::Tone;

/// A page of the tab: its MCP id, its label and the sentence under its title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub about: &'static str,
}

pub const GENERAL: usize = 0;
pub const BROWSER: usize = 1;
pub const MODE: usize = 2;
pub const RELATIONS: usize = 3;
pub const CONTENTS: usize = 4;
pub const PDA: usize = 5;
pub const COMMANDS: usize = 6;

/// The pages in the nav's order (`kSettingsPages`).
pub const PAGES: [PageInfo; 7] = [
    PageInfo {
        id: "general",
        label: "General",
        about: "What the game's Mods menu shows, and where the game looks for updates.",
    },
    PageInfo {
        id: "mod_browser",
        label: "Mod Browser",
        about: "Shown next to the module in the game's Mods window and Mod Browser. Listing it there: Mod > Submit to Mod Browser.",
    },
    PageInfo {
        id: "game_mode",
        label: "Game Mode",
        about: "A module targets one mode: the plain game, a mode the linked game has, or a new mode it brings. A mode is not the mod: the mod's name is on General.",
    },
    PageInfo {
        id: "relations",
        label: "Relations",
        about: "The modules this one needs, loads around or cannot run with. Only matters when the player runs several modules at once: a standalone mod leaves all four empty.",
    },
    PageInfo {
        id: "build_contents",
        label: "Build Contents",
        about: "The project folder is your working place as well as the module. The build ships everything in it except what is listed here.",
    },
    PageInfo {
        id: "pda",
        label: "PDA",
        about: "A module can give the PDA screens of its own. The layout named here is the one the game shows.",
    },
    PageInfo {
        id: "commands",
        label: "Commands",
        about: "Console commands the module adds. The player types the name; the game calls the function of the project's scripts with what follows it and prints what it returns.",
    },
];

/// The page of an MCP id.
#[must_use]
pub fn page_index(id: &str) -> Option<usize> {
    PAGES.iter().position(|p| p.id == id)
}

/// The room the C++ form gave the description (one byte for the terminator).
const DESCRIPTION_ROOM: usize = 4096;

/// What the tab shows of `mod.ltx`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub website: String,
    pub github: String,
    pub target: String,
    pub mode: String,
    pub pmode_id: String,
    pub pmode_title: String,
    /// One line per module, as written (`other.mod = >=1.0` keeps its value).
    pub requires: Vec<String>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub conflicts: Vec<String>,
    /// `[exclude]`, a rule per line.
    pub exclude: String,
    pub tags: Vec<String>,
    pub pda: String,
    pub updates_off: bool,
    pub commands: Vec<Command>,
}

/// A text cut at a character boundary below `room` bytes: one broken UTF-8 byte would make the
/// game read the whole file as cp1251.
fn cut_to(text: &str, room: usize) -> String {
    if text.len() < room {
        return text.to_owned();
    }
    let mut cut = room - 1;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.get(..cut).unwrap_or("").to_owned()
}

impl Form {
    /// The form over a manifest (`FormFromManifest`).
    #[must_use]
    pub fn of(m: &Manifest) -> Self {
        let first = m.provides_modes.first();
        Self {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            author: m.author.clone(),
            description: cut_to(&m.description, DESCRIPTION_ROOM),
            website: m.website.clone(),
            github: m.update_github.clone(),
            target: m.target.clone(),
            mode: m.mode.clone(),
            pmode_id: first.map(|p| p.id.clone()).unwrap_or_default(),
            pmode_title: first.map(|p| p.title.clone()).unwrap_or_default(),
            requires: m.requires.clone(),
            after: m.after.clone(),
            before: m.before.clone(),
            conflicts: m.conflicts.clone(),
            exclude: m.exclude.join("\n"),
            tags: m.tags.clone(),
            pda: m.pda_layout.clone(),
            updates_off: m.disable_game_updates,
            commands: m.commands.clone(),
        }
    }

    /// `mod.ltx` changed: every field the author has not touched (still what `base` says)
    /// follows `fresh` (`SyncSettingsForm`). The command list is one field.
    pub fn follow(&mut self, base: &Form, fresh: &Form) {
        macro_rules! follow {
            ($($f:ident),*) => {
                $(if self.$f == base.$f { self.$f.clone_from(&fresh.$f); })*
            };
        }
        follow!(
            id,
            name,
            version,
            author,
            description,
            website,
            github,
            target,
            mode,
            pmode_id,
            pmode_title,
            requires,
            after,
            before,
            conflicts,
            exclude,
            tags,
            pda,
            commands
        );
        if self.updates_off == base.updates_off {
            self.updates_off = fresh.updates_off;
        }
    }

    /// The exclude rules as lines, empty ones left out.
    #[must_use]
    pub fn exclude_rules(&self) -> Vec<String> {
        self.exclude.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
    }

    /// Which game the module targets (`SettingsKind`); `None` while it was never chosen.
    #[must_use]
    pub fn kind(&self) -> Option<Kind> {
        let t = self.target.as_str();
        if t.eq_ignore_ascii_case("default") {
            Some(Kind::Plain)
        } else if t.eq_ignore_ascii_case("existing") {
            Some(Kind::Existing)
        } else if t.eq_ignore_ascii_case("new") {
            Some(Kind::New)
        } else if t.eq_ignore_ascii_case("any") || self.mode == "*" {
            // a manifest older than the target key is read off the mode fields
            Some(Kind::Any)
        } else if !self.pmode_id.is_empty() {
            Some(Kind::New)
        } else if !self.mode.is_empty() {
            Some(Kind::Existing)
        } else {
            None
        }
    }

    /// Picks a kind (`SetSettingsKind`): the fields of the others go. `first_mode` is the first
    /// mode of the linked game, which an Existing Mode starts on.
    pub fn set_kind(&mut self, kind: Kind, first_mode: Option<&str>) {
        match kind {
            Kind::Plain => {
                self.mode.clear();
                self.pmode_id.clear();
                self.pmode_title.clear();
                "default".clone_into(&mut self.target);
            }
            Kind::Existing => {
                self.pmode_id.clear();
                self.pmode_title.clear();
                "existing".clone_into(&mut self.target);
                if self.mode.is_empty() || self.mode == "*" {
                    match first_mode {
                        Some(m) => m.clone_into(&mut self.mode),
                        None => self.mode.clear(),
                    }
                }
            }
            Kind::New => {
                self.mode.clear();
                "new".clone_into(&mut self.target);
            }
            Kind::Any => {
                self.pmode_id.clear();
                self.pmode_title.clear();
                "*".clone_into(&mut self.mode);
                "any".clone_into(&mut self.target);
            }
        }
    }

    /// Ticks or unticks a tag; the tags stay in the vocabulary's order, and a hand-written word
    /// the game ignores goes (`TagsRow`).
    pub fn set_tag(&mut self, vocabulary: &[&str], tag: &str, on: bool) {
        let kept: Vec<String> = vocabulary
            .iter()
            .filter(|v| if **v == tag { on } else { self.tags.iter().any(|t| t == *v) })
            .map(|v| (*v).to_owned())
            .collect();
        self.tags = kept;
    }
}

/// The game a module's level work applies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `target = default`: only while no campaign mode is active.
    Plain,
    /// `target = existing`: a mode of the linked game.
    Existing,
    /// `target = new`: a mode the module brings.
    New,
    /// `target = any`, `mode = *`: a classic mod, in every game.
    Any,
}

impl Kind {
    /// The kinds the Target row offers, in its order.
    pub const OFFERED: [Kind; 3] = [Kind::Plain, Kind::Existing, Kind::New];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Kind::Plain => "Plain Game",
            Kind::Existing => "Existing Mode",
            Kind::New => "New Mode",
            Kind::Any => "Every Mode",
        }
    }

    /// The muted sentence under the Target row.
    #[must_use]
    pub fn about(self) -> &'static str {
        match self {
            Kind::Plain => {
                "Applies only while no campaign mode is active: Revolution II and the like do not see this module's level work."
            }
            Kind::Existing => "The level work applies only in that mode.",
            Kind::New => {
                "A new mode is a new game of its own: the player picks it on the new-game screen, by its title."
            }
            Kind::Any => {
                "No gate: the level work lands in the plain game and in every campaign. Use it only for what belongs everywhere."
            }
        }
    }
}

/// What the checks read besides the form: taken when the tab opens or a page is picked.
#[derive(Clone, Debug, Default)]
pub struct Aux {
    /// `<script>.<function>` of the project's scripts.
    pub functions: Vec<String>,
    /// The scripts were read: before that a function is not known to be missing.
    pub functions_read: bool,
    /// `(command, module id)` of the linked game's other modules.
    pub taken: Vec<(String, String)>,
    /// The project's `.pdaasset` files, project-relative with `\`.
    pub pda_files: Vec<String>,
    /// The last picture that did not become a thumbnail.
    pub picture_error: String,
}

/// A problem of the form, shown under its field, as a dot on its page and in `mod.view`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub page: usize,
    /// The row it is shown under: `id`, `version`, `commands.<n>.<part>`, ...
    pub field: String,
    pub tone: Tone,
    /// Nothing is saved until it is fixed.
    pub blocks: bool,
    pub text: String,
}

fn problem(page: usize, field: &str, tone: Tone, blocks: bool, text: impl Into<String>) -> Problem {
    Problem { page, field: field.to_owned(), tone, blocks, text: text.into() }
}

/// The id of a relation line: a `[requires]` line may carry a value after `=`, which the game
/// does not read; the id is what it compares, in lower case (`RelationId`).
#[must_use]
pub fn relation_id(line: &str) -> String {
    line.split('=').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// What the line carries after its id (`>=1.0`), trimmed.
#[must_use]
pub fn relation_value(line: &str) -> &str {
    line.split_once('=').map_or("", |(_, v)| v.trim())
}

fn check_relations(f: &Form, out: &mut Vec<Problem>) {
    let rels: [(&str, &[String]); 4] =
        [("requires", &f.requires), ("after", &f.after), ("before", &f.before), ("conflicts", &f.conflicts)];
    let ids: Vec<Vec<String>> =
        rels.iter().map(|(_, lines)| lines.iter().map(|l| relation_id(l)).collect()).collect();
    let me = f.id.to_ascii_lowercase();
    let warn = |field: &str, text: String| problem(RELATIONS, field, Tone::Warning, false, text);
    for ((field, _), list) in rels.iter().zip(&ids) {
        if !me.is_empty() && list.contains(&me) {
            out.push(warn(field, "The module itself is listed here: the game ignores the line.".to_owned()));
        }
    }
    let (req, after, before, conflicts) = (&ids[0], &ids[1], &ids[2], &ids[3]);
    for id in conflicts {
        if req.contains(id) {
            out.push(warn("conflicts", format!("Required and in conflict at once: {id}. The game refuses this module with it and without it.")));
        }
    }
    for id in before {
        if after.contains(id) {
            out.push(warn("before", format!("Loads both before and after {id}: the game cannot keep both.")));
        }
    }
}

/// The game's own short words a module could pick: refused (`StockCommandWord`; kept in step
/// with `gz_mod::commands`).
fn stock_command_word(name: &str) -> bool {
    const WORDS: [&str; 23] = [
        "bind",
        "chat",
        "disconnect",
        "flush",
        "fov",
        "help",
        "hide",
        "load",
        "name",
        "nq",
        "quit",
        "renderer",
        "save",
        "screenshot",
        "start",
        "unbind",
        "unbindall",
        "cfg_save",
        "cfg_load",
        "main_menu",
        "run_string",
        "time_factor",
        "default_controls",
    ];
    WORDS.contains(&name)
}

/// The game's own command families: a warning (`StockCommandFamily`).
fn stock_command_family(name: &str) -> Option<&'static str> {
    const FAMILIES: [&str; 32] = [
        "xms_", "r_", "r1_", "r2_", "r3_", "r4_", "r__", "rs_", "g_", "sv_", "cl_", "net_", "dbg_", "debug_",
        "ai_", "al_", "ph_", "hud_", "snd_", "vid_", "mm_", "mt_", "dar_", "cam_", "ui_", "inv_", "lua_",
        "gamepad_", "demo_", "stat_", "bind_", "unbind_",
    ];
    FAMILIES.into_iter().find(|f| name.starts_with(f))
}

fn check_commands(f: &Form, aux: &Aux, out: &mut Vec<Problem>) {
    for (i, c) in f.commands.iter().enumerate() {
        let field = |part: &str| format!("commands.{i}.{part}");
        let mut add = |part: &str, tone: Tone, blocks: bool, text: String| {
            out.push(Problem { page: COMMANDS, field: field(part), tone, blocks, text })
        };
        if c.name.is_empty() {
            add("name", Tone::Required, true, "Required: what the player types in the console.".to_owned());
        } else if !valid_command_name(&c.name) {
            add(
                "name",
                Tone::Error,
                true,
                "Use a-z, 0-9 and _ only: the console finds a command by its exact name.".to_owned(),
            );
        } else if stock_command_word(&c.name) {
            add(
                "name",
                Tone::Error,
                true,
                "The game has a command of this name: it keeps its own and refuses this one.".to_owned(),
            );
        } else if let Some(family) = stock_command_family(&c.name) {
            add(
                "name",
                Tone::Warning,
                false,
                format!(
                    "Named like the game's own {family}* commands: if the game has one of this name, it keeps it."
                ),
            );
        }
        if !c.name.is_empty() && f.commands.iter().take(i).any(|k| k.name.eq_ignore_ascii_case(&c.name)) {
            add(
                "name",
                Tone::Error,
                true,
                "Declared twice: the game takes the first and refuses this one.".to_owned(),
            );
        }
        if let Some((_, module)) = aux.taken.iter().find(|(command, _)| command.eq_ignore_ascii_case(&c.name))
        {
            add(
                "name",
                Tone::Warning,
                false,
                format!(
                    "Module {module} of the linked game has this command too: the one that loads first keeps it."
                ),
            );
        }
        if c.function.is_empty() {
            add(
                "function",
                Tone::Required,
                true,
                "Required: <script>.<function> of the project's scripts.".to_owned(),
            );
        } else if !valid_command_function(&c.function) {
            add("function", Tone::Error, true, "<script>.<function>, e.g. my_mod.status.".to_owned());
        } else if aux.functions_read && !has_script_function(&aux.functions, &c.function) {
            add(
                "function",
                Tone::Warning,
                false,
                "gamedata\\scripts has no such function: the build refuses the module until it does."
                    .to_owned(),
            );
        }
        if let Some(bad) = outside_cp1251(&c.help) {
            add(
                "help",
                Tone::Warning,
                false,
                format!("Outside cp1251 ({bad}): the console shows '?' for it."),
            );
        }
        if let Some(bad) = outside_cp1251(&c.args) {
            add(
                "args",
                Tone::Warning,
                false,
                format!("Outside cp1251 ({bad}): the console shows '?' for it."),
            );
        }
    }
}

/// Whether `rel` names one of `files` (any case, either separator).
#[must_use]
pub fn same_path(a: &str, b: &str) -> bool {
    a.replace('/', "\\").eq_ignore_ascii_case(&b.replace('/', "\\"))
}

/// The problems of the form, in the C++ order (`CheckSettings`).
#[must_use]
pub fn check(f: &Form, aux: &Aux) -> Vec<Problem> {
    let mut out = Vec::new();
    if f.id.is_empty() {
        out.push(problem(GENERAL, "id", Tone::Error, true, "Required: the folder the module installs into."));
    } else if !validate_id(&f.id) {
        out.push(problem(GENERAL, "id", Tone::Error, true, "Use a-z, 0-9, _ . and - only."));
    }
    if !validate_version(&f.version) {
        out.push(problem(
            GENERAL,
            "version",
            Tone::Error,
            true,
            "1 to 4 numbers separated by dots, e.g. 1.2.0.",
        ));
    }
    let website = f.website.trim();
    if !validate_website(website) {
        out.push(problem(
            GENERAL,
            "website",
            Tone::Error,
            true,
            "Only https://ap-pro.ru and https://moddb.com links are allowed.",
        ));
    } else if !website.is_empty() && !website_is_module_page(website) {
        // the listing wants the module's own page, which the game's own check does not ask for
        out.push(problem(GENERAL, "website", Tone::Warning, false, "The Mod Browser lists a module with its own page, e.g. https://ap-pro.ru/stuff/<section>/<name>-r<number>/."));
    }
    if !validate_github_repo(&normalize_github_repo(&f.github)) {
        out.push(problem(GENERAL, "github", Tone::Error, true, "owner/repo, e.g. Madmer/agroprom-story."));
    }

    if !aux.picture_error.is_empty() {
        out.push(problem(BROWSER, "picture", Tone::Error, false, aux.picture_error.clone()));
    }

    // the choice sticks while its details are still blank, so the missing detail is named instead
    match f.kind() {
        None => out.push(problem(
            MODE,
            "target",
            Tone::Required,
            false,
            "Not chosen yet: the module cannot be built.",
        )),
        Some(Kind::Existing) if f.mode.is_empty() => out.push(problem(
            MODE,
            "mode",
            Tone::Required,
            false,
            "Pick the mode: the module cannot be built until then.",
        )),
        Some(Kind::New) if f.pmode_id.is_empty() => {
            out.push(problem(
                MODE,
                "pmode_id",
                Tone::Required,
                false,
                "Enter the mode's id: the module cannot be built until then.",
            ));
        }
        _ => {}
    }
    // "*" is the engine's no-gate wildcard: legal for mode=, never for a mode of the module's own
    if f.mode != "*" && !validate_mode_id(&f.mode) {
        out.push(problem(MODE, "mode", Tone::Error, true, "Use a-z, 0-9, _ . and - only."));
    }
    if !validate_mode_id(&f.pmode_id) {
        out.push(problem(MODE, "pmode_id", Tone::Error, true, "Use a-z, 0-9, _ . and - only."));
    }
    // the caption ships in a cp1251 string table: anything outside that page reaches the
    // new-game screen as a look-alike or '?', while this field keeps showing the original
    if f.kind() == Some(Kind::New)
        && let Some(bad) = outside_cp1251(&f.pmode_title)
    {
        out.push(problem(
            MODE,
            "pmode_title",
            Tone::Warning,
            false,
            format!("Outside cp1251 ({bad}): the new-game screen shows a look-alike letter or '?'."),
        ));
    }

    if !f.pda.is_empty() {
        if !aux.pda_files.iter().any(|p| same_path(p, &f.pda)) {
            out.push(problem(
                PDA,
                "pda",
                Tone::Warning,
                false,
                "The file does not exist: the build refuses the module until it does.",
            ));
        } else if gz_mod::outside_build(&f.exclude_rules(), &f.pda.replace('/', "\\")) {
            out.push(problem(
                PDA,
                "pda",
                Tone::Warning,
                false,
                "The build leaves this file out, so the game would never see it.",
            ));
        }
    }

    check_relations(f, &mut out);
    check_commands(f, aux, &mut out);
    out
}

/// The first problem that stops the save, and how many there are.
#[must_use]
pub fn blocking(problems: &[Problem]) -> (Option<&Problem>, usize) {
    let mut it = problems.iter().filter(|p| p.blocks);
    let first = it.next();
    (first, first.map_or(0, |_| 1 + it.count()))
}

/// The worst tone of a page's problems: the dot beside it in the nav (`PageTone`).
#[must_use]
pub fn page_tone(problems: &[Problem], page: usize) -> Option<Tone> {
    let rank = |t: Tone| match t {
        Tone::Error => 3,
        Tone::Required => 2,
        _ => 1,
    };
    problems.iter().filter(|p| p.page == page).max_by_key(|p| rank(p.tone)).map(|p| p.tone)
}

/// A tone as `mod.view` names it.
#[must_use]
pub fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Error => "error",
        Tone::Required => "required",
        Tone::Warning => "warning",
        _ => "note",
    }
}

/// What an edit sends: the `set` of one `mod.set` - every change of the form in one request,
/// written all or none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub set: serde_json::Map<String, Value>,
}

impl Plan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// The arguments of `mod.set`.
    #[must_use]
    pub fn args(&self) -> Args {
        let mut a = Args::new();
        a.insert("set".to_owned(), Value::Object(self.set.clone()));
        a
    }

    fn key(&mut self, section: &str, key: &str, value: Value) {
        let entry =
            self.set.entry(section.to_owned()).or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Value::Object(map) = entry {
            map.insert(key.to_owned(), value);
        }
    }
}

fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

fn list(v: &[String]) -> Value {
    Value::Array(v.iter().map(|x| s(x)).collect())
}

/// The changes that make `mod.ltx` say what `form` says, where `base` is what it says now.
#[must_use]
pub fn plan(base: &Form, form: &Form) -> Plan {
    let mut p = Plan::default();
    macro_rules! text {
        ($section:literal: $($f:ident => $key:literal),*) => {
            $(if form.$f != base.$f { p.key($section, $key, s(&form.$f)); })*
        };
    }
    text!("module": id => "id", name => "name", version => "version", author => "author", description => "description", website => "website", mode => "mode");
    text!("update": github => "github");
    if form.after != base.after {
        p.key("order", "after", list(&form.after));
    }
    if form.before != base.before {
        p.key("order", "before", list(&form.before));
    }
    if form.tags != base.tags {
        p.key("module", "tags", list(&form.tags));
    }
    if form.updates_off != base.updates_off {
        p.key("module", "disable_game_updates", Value::Bool(form.updates_off));
    }
    for (section, now, was) in
        [("requires", &form.requires, &base.requires), ("conflicts", &form.conflicts, &base.conflicts)]
    {
        if now != was {
            p.set.insert(section.to_owned(), list(now));
        }
    }
    if form.exclude_rules() != base.exclude_rules() {
        p.set.insert("exclude".to_owned(), list(&form.exclude_rules()));
    }
    if form.pmode_id != base.pmode_id || form.pmode_title != base.pmode_title {
        p.key("provides_mode", "id", s(&form.pmode_id));
        p.key("provides_mode", "title", s(&form.pmode_title));
    }
    let commands = command_ops(&base.commands, &form.commands);
    if !commands.is_empty() {
        p.set.insert("commands".to_owned(), Value::Object(commands));
    }
    if form.pda != base.pda {
        p.key("pda", "layout", s(&form.pda));
    }
    // the targeting choice is always stated: left out, the command would read it off the mode
    let stated = ["default", "existing", "new"].iter().any(|t| t.eq_ignore_ascii_case(&form.target));
    if stated && (form.target != base.target || !p.is_empty()) {
        p.key("module", "target", s(&form.target.to_ascii_lowercase()));
    }
    p
}

/// The `commands` of `mod.set` that turn `base` into `form`: a command gone is null, a new or
/// changed one its fields. A renamed command is the old one gone and a new one.
#[must_use]
pub fn command_ops(base: &[Command], form: &[Command]) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for b in base {
        if !form.iter().any(|c| c.name.eq_ignore_ascii_case(&b.name)) {
            out.insert(b.name.clone(), Value::Null);
        }
    }
    for c in form {
        let same = base.iter().find(|b| b.name.eq_ignore_ascii_case(&c.name));
        if same.is_some_and(|b| {
            b.name == c.name
                && b.function == c.function
                && b.help == c.help
                && b.args == c.args
                && b.cheat == c.cheat
                && b.declared == c.declared
        }) {
            continue;
        }
        let mut fields = serde_json::Map::new();
        fields.insert("function".to_owned(), s(&c.function));
        fields.insert("help".to_owned(), s(&c.help));
        fields.insert("args".to_owned(), s(&c.args));
        fields.insert("cheat".to_owned(), Value::Bool(c.cheat));
        out.insert(c.name.clone(), Value::Object(fields));
    }
    out
}

fn slug_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

fn segment(p: &[u8], allowed: fn(u8) -> bool) -> Option<(&[u8], &[u8])> {
    let n = p.iter().take_while(|c| !matches!(**c, b'/' | b'?' | b'#') && allowed(**c)).count();
    (n > 0).then(|| p.split_at(n))
}

fn starts<'a>(p: &'a [u8], prefix: &str) -> Option<&'a [u8]> {
    let head = p.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix.as_bytes()).then(|| p.get(prefix.len()..).unwrap_or(&[]))
}

fn page_end(p: &[u8]) -> bool {
    let p = p.strip_prefix(b"/").unwrap_or(p);
    matches!(p.first(), None | Some(b'?' | b'#'))
}

/// Whether `url` is the module's own page on AP-PRO or ModDB (`EditorCatalog::WebsiteIsModulePage`;
/// the same rule as the Mod Browser area's, kept here so the form needs no catalog).
#[must_use]
pub fn website_is_module_page(url: &str) -> bool {
    if url.is_empty() || !validate_website(url) {
        return false;
    }
    let Some(p) = starts(url.as_bytes(), "https://") else { return false };
    let p = starts(p, "www.").unwrap_or(p);
    if let Some(p) = starts(p, "ap-pro.ru/") {
        if let Some(p) = starts(p, "stuff/") {
            let Some((_, p)) = segment(p, slug_char) else { return false };
            let Some(p) = p.strip_prefix(b"/") else { return false };
            let Some((last, p)) = segment(p, slug_char) else { return false };
            let Some(r) = last.windows(2).rposition(|w| w == b"-r") else { return false };
            let number = last.get(r + 2..).unwrap_or(&[]);
            return !number.is_empty() && number.iter().all(u8::is_ascii_digit) && page_end(p);
        }
        if let Some(p) = starts(p, "forums/topic/") {
            let digits = p.iter().take_while(|c| c.is_ascii_digit()).count();
            let Some(p) = p.get(digits..).filter(|_| digits > 0).and_then(|p| p.strip_prefix(b"-")) else {
                return false;
            };
            return segment(p, slug_char).is_some_and(|(_, rest)| page_end(rest));
        }
        return false;
    }
    if let Some(p) = starts(p, "moddb.com/") {
        let tail_ok = |rest: &[u8]| matches!(rest.first(), None | Some(b'/'));
        if let Some(p) = starts(p, "mods/").or_else(|| starts(p, "addons/")) {
            return segment(p, slug_char).is_some_and(|(_, rest)| tail_ok(rest));
        }
        if let Some(p) = starts(p, "games/") {
            let Some((_, p)) = segment(p, slug_char) else { return false };
            let Some(p) = starts(p, "/addons/") else { return false };
            return segment(p, slug_char).is_some_and(|(_, rest)| tail_ok(rest));
        }
    }
    false
}

/// The folder a build fills, read from its end: `<folder>\modules\<id>` or `<folder>\gamedata_<id>`.
#[must_use]
pub fn writes_to(folder: &str, id: &str, flat: bool) -> String {
    let dest = folder.trim_end_matches(['\\', '/']);
    if flat { format!("{dest}\\gamedata_{id}") } else { format!("{dest}\\modules\\{id}") }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form { id: "od".into(), version: "1.0".into(), target: "default".into(), ..Form::default() }
    }

    #[test]
    fn a_clean_form_has_no_problem() {
        assert!(check(&form(), &Aux::default()).is_empty());
    }

    #[test]
    fn the_general_fields_say_why() {
        let f = Form {
            id: String::new(),
            version: "x".into(),
            website: "https://example.com".into(),
            github: "nope".into(),
            ..form()
        };
        let p = check(&f, &Aux::default());
        let fields: Vec<&str> = p.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(fields, ["id", "version", "website", "github"]);
        assert!(p.iter().all(|p| p.blocks && p.tone == Tone::Error));
        assert_eq!(blocking(&p).1, 4);
    }

    #[test]
    fn a_website_that_is_not_the_module_page_warns() {
        let f = Form { website: "https://ap-pro.ru/".into(), ..form() };
        let p = check(&f, &Aux::default());
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].tone, Tone::Warning);
        assert!(!p[0].blocks);
        assert!(website_is_module_page("https://ap-pro.ru/stuff/mods/od-r12/"));
    }

    #[test]
    fn the_mode_names_the_missing_detail() {
        let mut f = Form { target: String::new(), ..form() };
        assert_eq!(check(&f, &Aux::default())[0].field, "target");
        f.set_kind(Kind::Existing, None);
        assert_eq!(check(&f, &Aux::default())[0].field, "mode");
        f.set_kind(Kind::Existing, Some("metro"));
        assert_eq!(f.mode, "metro");
        f.set_kind(Kind::New, None);
        assert_eq!(f.mode, "");
        assert_eq!(check(&f, &Aux::default())[0].field, "pmode_id");
        f.pmode_id = "night".into();
        f.pmode_title = "Ночь".into();
        assert!(check(&f, &Aux::default()).is_empty());
    }

    #[test]
    fn an_old_manifest_reads_its_kind_off_the_mode() {
        let f = Form { target: String::new(), mode: "*".into(), ..form() };
        assert_eq!(f.kind(), Some(Kind::Any));
        let f = Form { target: String::new(), mode: "metro".into(), ..form() };
        assert_eq!(f.kind(), Some(Kind::Existing));
    }

    #[test]
    fn relations_warn_about_what_the_game_cannot_keep() {
        let f = Form {
            requires: vec!["qa.other = >=1.0".into()],
            conflicts: vec!["qa.other".into()],
            after: vec!["b".into(), "OD".into()],
            before: vec!["b".into()],
            ..form()
        };
        let p = check(&f, &Aux::default());
        let texts: Vec<&str> = p.iter().map(|p| p.text.as_str()).collect();
        assert!(texts[0].starts_with("The module itself"));
        assert!(texts[1].contains("qa.other"));
        assert!(texts[2].contains("before and after b"));
        assert!(p.iter().all(|p| p.page == RELATIONS && p.tone == Tone::Warning));
        assert_eq!(relation_id("Other.Mod = >=1.0"), "other.mod");
        assert_eq!(relation_value("Other.Mod = >=1.0"), ">=1.0");
    }

    #[test]
    fn commands_are_checked_per_row() {
        let c = |name: &str, function: &str| Command {
            name: name.into(),
            function: function.into(),
            ..Command::default()
        };
        let f = Form {
            commands: vec![c("", ""), c("save", "od.x"), c("od_s", "od.missing"), c("OD_S", "od.ok")],
            ..form()
        };
        let aux = Aux { functions: vec!["od.ok".into()], functions_read: true, ..Aux::default() };
        let p = check(&f, &aux);
        let fields: Vec<&str> = p.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "commands.0.name",
                "commands.0.function",
                "commands.1.name",
                "commands.1.function",
                "commands.2.function",
                "commands.3.name",
                "commands.3.name"
            ]
        );
        assert_eq!(p[0].tone, Tone::Required);
    }

    #[test]
    fn a_pda_layout_must_exist_and_ship() {
        let f = Form { pda: "pda/od.pdaasset".into(), ..form() };
        assert_eq!(check(&f, &Aux::default()).len(), 1);
        let aux = Aux { pda_files: vec!["pda\\od.pdaasset".into()], ..Aux::default() };
        assert!(check(&f, &aux).is_empty());
        let f = Form { exclude: "pda".into(), ..f };
        assert!(check(&f, &aux)[0].text.contains("leaves this file out"));
    }

    #[test]
    fn untouched_fields_follow_the_file() {
        let base = form();
        let mut f = Form { name: "Mine".into(), ..base.clone() };
        let fresh = Form { name: "Theirs".into(), version: "2.0".into(), ..base.clone() };
        f.follow(&base, &fresh);
        assert_eq!(f.name, "Mine");
        assert_eq!(f.version, "2.0");
    }

    #[test]
    fn the_plan_sends_what_changed_and_states_the_target() {
        let base = form();
        assert!(plan(&base, &base).is_empty());
        let f = Form {
            name: "OD".into(),
            requires: vec!["a".into(), "b = 1.0".into()],
            exclude: "docs\n\n*.psd".into(),
            ..base.clone()
        };
        let p = plan(&base, &f);
        assert_eq!(p.set["module"]["name"], s("OD"));
        assert_eq!(p.set["requires"], serde_json::json!(["a", "b = 1.0"]));
        assert_eq!(p.set["exclude"], serde_json::json!(["docs", "*.psd"]));
        assert_eq!(p.set["module"]["target"], s("default"));
        let f = Form { pda: "pda\\x.pdaasset".into(), ..base.clone() };
        assert_eq!(plan(&base, &f).set["pda"]["layout"], s("pda\\x.pdaasset"));
    }

    #[test]
    fn a_classic_module_stays_classic() {
        let base = Form { target: "any".into(), mode: "*".into(), ..form() };
        assert_eq!(base.kind(), Some(Kind::Any));
        let f = Form { name: "Weapons".into(), ..base.clone() };
        let p = plan(&base, &f);
        assert!(p.set["module"].get("target").is_none() && p.set["module"].get("mode").is_none());
        let mut back = base.clone();
        back.set_kind(Kind::Plain, None);
        back.set_kind(Kind::Any, None);
        assert!(plan(&base, &back).is_empty());
    }

    #[test]
    fn commands_plan_removes_and_sets() {
        let c = |name: &str| Command { name: name.into(), function: "od.x".into(), ..Command::default() };
        let base = vec![c("a"), c("b")];
        let form = vec![c("a"), c("c")];
        let ops = command_ops(&base, &form);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops["b"], Value::Null);
        assert_eq!(ops["c"]["function"], s("od.x"));
    }

    #[test]
    fn tags_keep_the_vocabulary_order() {
        let mut f = Form { tags: vec!["weapons".into(), "made_up".into()], ..form() };
        f.set_tag(&["story", "weapons"], "story", true);
        assert_eq!(f.tags, ["story", "weapons"]);
        f.set_tag(&["story", "weapons"], "weapons", false);
        assert_eq!(f.tags, ["story"]);
    }

    #[test]
    fn the_destination_is_read_from_its_end() {
        assert_eq!(writes_to("D:\\Game\\", "od", false), "D:\\Game\\modules\\od");
        assert_eq!(writes_to("D:\\Out", "od", true), "D:\\Out\\gamedata_od");
    }

    #[test]
    fn a_long_description_is_cut_on_a_character() {
        let long = "я".repeat(3000);
        let cut = cut_to(&long, DESCRIPTION_ROOM);
        assert!(cut.len() < DESCRIPTION_ROOM);
        assert!(cut.chars().all(|c| c == 'я'));
    }
}
