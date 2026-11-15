//! The metadata rules - the game enforces the same ones (`MOD_UPDATES.md`, section 1) - and the
//! manifest's own check before a build.
//!
//! One list: the words a key takes and how its value is judged are the reference's table
//! ([`crate::reference::SECTIONS`]). [`refusal`] is what `mod.set` and the Mod Settings form turn
//! a wrong value away with, [`validate`] is what `asset.check`, `mod.info` and a build say about
//! a file written by hand - the same [`accepts`] answers in all of them.

use gz_base::{Problem, RelPath};

use crate::doc::ManifestDoc;
use crate::model::{FILE, Manifest};
use crate::reference::{Check, Reader, SectionDoc, key_doc, key_of, section_named};
use crate::text::{outside_cp1251, split_list, text_value, trim};

fn id_char(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
}

/// The XMS id charset: `[a-z0-9_.-]+`.
#[must_use]
pub fn validate_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(id_char)
}

/// A mode id follows the module id charset, but empty is allowed (= no mode: the ordinary game).
#[must_use]
pub fn validate_mode_id(id: &str) -> bool {
    id.is_empty() || validate_id(id)
}

/// `mode = <id>`: a mode id, or `*` (every mode, a classic module's); empty is no mode.
#[must_use]
pub fn validate_mode_ref(id: &str) -> bool {
    id == "*" || validate_mode_id(id)
}

/// The id of a mode the module adds: the id charset, 64 characters at most, never `*`; empty
/// is no mode.
#[must_use]
pub fn validate_own_mode_id(id: &str) -> bool {
    validate_mode_id(id) && id.len() <= 64
}

/// `min_parts`..`max_parts` dot-separated decimal numbers, 9 digits each at most.
#[must_use]
pub fn dotted_number(v: &str, min_parts: usize, max_parts: usize) -> bool {
    if v.is_empty() {
        return false;
    }
    let (mut parts, mut digits) = (1usize, 0usize);
    for c in v.bytes() {
        if c.is_ascii_digit() {
            digits += 1;
            if digits > 9 {
                return false;
            }
        } else if c == b'.' {
            parts += 1;
            if digits == 0 || parts > max_parts {
                return false;
            }
            digits = 0;
        } else {
            return false;
        }
    }
    digits > 0 && parts >= min_parts
}

/// `MOD_UPDATES.md` 1.2: 1-4 dot-separated numbers and nothing else, 32 characters in all - a
/// release has to be orderable.
#[must_use]
pub fn validate_version(version: &str) -> bool {
    dotted_number(version, 1, 4) && version.len() <= 32
}

/// `MOD_UPDATES.md` 1.3: https, the host exactly `(www.)ap-pro.ru` or `(www.)moddb.com`, no
/// user info or port, a limited charset, 512 bytes at most; empty is valid.
#[must_use]
pub fn validate_website(url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    if url.len() > 512 {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else { return false };
    // the host is everything up to the first '/', compared whole: "user@host", "host:port"
    // and "host.evil.com" all land inside it and fail the match
    let (host, path) = match rest.find('/') {
        Some(i) => (rest.get(..i).unwrap_or(""), rest.get(i..).unwrap_or("")),
        None => (rest, ""),
    };
    const HOSTS: [&str; 4] = ["ap-pro.ru", "www.ap-pro.ru", "moddb.com", "www.moddb.com"];
    if !HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        return false;
    }
    path.bytes().all(|c| c.is_ascii_alphanumeric() || b"-._~/?#&=%+".contains(&c))
}

/// `MOD_UPDATES.md` 1.4: `owner/repo`; empty is valid.
#[must_use]
pub fn validate_github_repo(repo: &str) -> bool {
    if repo.is_empty() {
        return true;
    }
    let Some((owner, name)) = repo.split_once('/') else { return false };
    if owner.is_empty() || owner.len() > 39 || name.is_empty() || name.len() > 100 {
        return false;
    }
    if !owner.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return false;
    }
    // a second '/' is not in the repository charset, so "a/b/c" fails here
    if !name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_')) {
        return false;
    }
    name != "." && name != ".."
}

/// A pasted `https://github.com/owner/repo[.git][/...]` reduced to `owner/repo`; anything else
/// comes back trimmed for [`validate_github_repo`] to judge.
#[must_use]
pub fn normalize_github_repo(input: &str) -> String {
    let mut out = trim(input).to_owned();
    const PREFIXES: [&str; 6] = [
        "https://github.com/",
        "http://github.com/",
        "https://www.github.com/",
        "http://www.github.com/",
        "github.com/",
        "www.github.com/",
    ];
    for p in PREFIXES {
        let hit = out.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p));
        if !hit {
            continue;
        }
        out.drain(..p.len());
        // a page URL carries more than the repository: ?tab=, #readme, /tree/main
        if let Some(q) = out.find(['?', '#']) {
            out.truncate(q);
        }
        if let Some(owner_end) = out.find('/')
            && let Some(repo_end) = out.get(owner_end + 1..).and_then(|r| r.find('/'))
        {
            out.truncate(owner_end + 1 + repo_end);
        }
        break;
    }
    // clone URLs end in .git; GitHub allows no repository name that does
    if out.len() > 4 && out.get(out.len() - 4..).is_some_and(|t| t.eq_ignore_ascii_case(".git")) {
        out.truncate(out.len() - 4);
    }
    out
}

/// Letters, digits and `_`: what the runtime reads as a global's name.
#[must_use]
pub fn valid_global_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// What a global variable's name takes, as a sentence's end.
pub const GLOBAL_NAME_TAKES: &str = "letters, digits and _";

/// The Mod Browser's tags (MOD_CATALOG 6.2), in their order. The reference's page and the tags
/// hook of `gz-release` are held to this list by tests.
pub const TAG_VOCABULARY: &[&str] = &[
    "story",
    "quests",
    "gameplay",
    "weapons",
    "equipment",
    "npcs",
    "mutants",
    "locations",
    "graphics",
    "audio",
    "interface",
    "fixes",
];

#[must_use]
pub fn known_tag(tag: &str) -> bool {
    TAG_VOCABULARY.contains(&tag)
}

/// The room the game's string buffer gives the description: 4095 bytes and the terminator.
pub const DESCRIPTION_ROOM: usize = 4096;

/// The four values of `target`.
pub const TARGETS: [&str; 4] = ["default", "existing", "new", "any"];

/// A flag as the file writes it: `true`, `1`, `yes`, `on` and `false`, `0`, `no`, `off`, in
/// lower case - the games's readers compare the words exactly.
#[must_use]
pub fn parse_flag(word: &str) -> Option<bool> {
    match word {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn is_number(v: &str) -> bool {
    v.parse::<f64>().is_ok()
}

/// Whether `value` is what a key judged by `check` takes. The value as written: quotes are not
/// looked into, so a text key (`description`) is decoded by its caller.
#[must_use]
pub fn accepts(check: Check, value: &str) -> bool {
    let v = trim(value);
    match check {
        Check::Free => true,
        Check::Id => validate_id(v),
        Check::Version => validate_version(v),
        Check::Website => validate_website(v),
        Check::Tags => split_list(v, ',').iter().all(|t| known_tag(t)),
        Check::Target => TARGETS.iter().any(|t| t.eq_ignore_ascii_case(v)),
        Check::Mode => validate_mode_ref(v),
        Check::OwnModeId => validate_own_mode_id(v),
        Check::Repo => validate_github_repo(v),
        Check::Description => v.len() < DESCRIPTION_ROOM,
        Check::Flag => parse_flag(v).is_some(),
        Check::FlagAnyCase => parse_flag(&v.to_ascii_lowercase()).is_some(),
        Check::Choice(words) => words.contains(&v),
        Check::Words(words) => split_list(v, ',').iter().all(|t| words.contains(&t.as_str())),
        Check::Number => is_number(v),
        Check::Range(lo, hi) => v.parse::<f64>().is_ok_and(|x| (f64::from(lo)..=f64::from(hi)).contains(&x)),
        Check::Count => v.parse::<i64>().is_ok(),
        Check::Rect => {
            let parts: Vec<&str> = v.split_whitespace().collect();
            parts.len() == 4 && parts.iter().all(|p| is_number(p))
        }
    }
}

/// The refusal of a value written to `[section] key` by a command or the form: `None` when the
/// key takes it (or the editor holds no rule for the key). One sentence in the key's words.
#[must_use]
pub fn refusal(section: &str, key: &str, value: &str) -> Option<String> {
    let (_, doc) = key_doc(section, key)?;
    if accepts(doc.check, value) {
        return None;
    }
    Some(format!("{key} takes {}, not '{value}'. {}", doc.values, doc.meaning))
}

/// The refusal of a global variable's name.
#[must_use]
pub fn global_name_refusal(name: &str) -> Option<String> {
    (!valid_global_name(name))
        .then(|| format!("a global variable's name takes {GLOBAL_NAME_TAKES}, not '{name}'"))
}

/// The refusal of a global variable declared a second time.
#[must_use]
pub fn global_twice(name: &str) -> String {
    format!("the global variable '{name}' is declared twice")
}

/// The first problem of a new project: the fix in the file's words (the key, its values, the
/// command), then the window.
const NO_TARGET: &str = concat!(
    "no target game mode: write `target = default` under [module] in mod.ltx for the ordinary game, ",
    "`mode = <id>` with `target = existing` for a mode of the linked game, or a [provides_mode] section ",
    "with `id = <id>` and `target = new` for a mode this module adds. ",
    "The command: mod.set set={\"module\":{\"target\":\"default\"}}; `reference manifest` lists the modes ",
    "and every key. In the window: Mod > Mod Settings, Game Mode"
);

/// A `target` and a `mode` that do not go together, in the file's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    /// `mod.target_default`, `mod.target_existing`, `mod.target_new` or `mod.target_any`.
    pub code: &'static str,
    /// The two lines as written, what goes with what: the sentence the check and `mod.set` say.
    pub text: String,
    /// The line to write instead, `key = value`.
    pub fix: String,
    /// The choice is made and its detail is not written yet (`existing` with no mode, `new` with
    /// no `[provides_mode]`): an error of the file's check - the module cannot be built - but
    /// no contradiction, so `mod.set` writes it. The window's form saves the choice first and
    /// the detail when it is picked.
    pub unfinished: bool,
}

/// Whether the target and the mode a manifest states go together, as the reference pairs them:
/// `default` takes no mode, `existing` a mode id (not `*`), `new` a `[provides_mode]` (its mode
/// may be named too), `any` the mode `*`. `provides`: the module declares a mode of its own.
/// A target that is none of the four, or none written, has nothing to pair (`mod.target_value`
/// and `mod.target` say what is wrong there).
#[must_use]
pub fn target_pairing(target: &str, mode: &str, provides: bool) -> Option<Pairing> {
    let (target, mode) = (trim(target), trim(mode));
    let kind = target.to_ascii_lowercase();
    let star = mode == "*";
    let lines = if mode.is_empty() {
        format!("[module] target = {target} with no mode")
    } else {
        format!("[module] target = {target} with mode = {mode}")
    };
    let (code, rule, fix) = match kind.as_str() {
        "default" if !mode.is_empty() => (
            "mod.target_default",
            format!(
                "target = default is the ordinary game and goes with no mode. Remove the mode, or write {}.",
                if star { "target = any for every game" } else { "target = existing for that mode" }
            ),
            if star { "target = any" } else { "target = existing" },
        ),
        "existing" if mode.is_empty() || star => (
            "mod.target_existing",
            format!(
                "target = existing is a mode of the linked game and goes with that mode's id. Write {}.",
                if star {
                    "the mode's id, or target = any for every game"
                } else {
                    "mode = <id>, or target = default for the ordinary game"
                }
            ),
            if star { "target = any" } else { "target = default" },
        ),
        "new" if !provides => (
            "mod.target_new",
            "target = new is a mode the module adds and goes with a [provides_mode] section. Add one with id = <id>, or write target = default for the ordinary game.".to_owned(),
            "target = default",
        ),
        "any" if !star => (
            "mod.target_any",
            format!(
                "target = any is every game and goes with mode = *. Write mode = *, or {}.",
                if mode.is_empty() { "target = default for the ordinary game" } else { "target = existing for that mode" }
            ),
            if mode.is_empty() { "mode = *" } else { "target = existing" },
        ),
        _ => return None,
    };
    let unfinished = (kind == "existing" && mode.is_empty()) || kind == "new";
    Some(Pairing { code, text: format!("{lines}: {rule}"), fix: fix.to_owned(), unfinished })
}

/// The rules that read several keys at once: the id, the targeting choice, the declared modes;
/// the warnings about the mode titles and about a module that turns the game's updates off. Each
/// problem names its key in `field`; [`validate`] puts it at its line.
fn manifest_rules(m: &Manifest) -> Vec<Problem> {
    let mut out = Vec::new();
    if !validate_id(&m.id) {
        out.push(
            Problem::error("mod.id", "module id missing or invalid in mod.ltx (allowed: a-z 0-9 _ . -)")
                .in_field("id"),
        );
    }
    // A finished module always states which game it is for. "Nothing chosen" is not a third
    // option - it is how content ends up applied in a campaign it was never built for. A target
    // that is none of the four is a wrong value, said by its own problem; one that is written
    // must go with the mode.
    if m.target.is_empty() && m.mode.is_empty() && m.provides_modes.is_empty() {
        out.push(Problem::error("mod.target", NO_TARGET).in_field("target").with_fix("target = default"));
    }
    let provides = m.provides_modes.iter().any(|p| !p.id.is_empty());
    if let Some(pair) = target_pairing(&m.target, &m.mode, provides) {
        out.push(Problem::error(pair.code, pair.text).in_field("target").with_fix(pair.fix));
    }
    if m.disable_game_updates {
        out.push(
            Problem::warning(
                "mod.updates_off",
                "This module turns off the game's updates and bug reports for everyone who has it.",
            )
            .in_field("disable_game_updates"),
        );
    }
    for mode in m.provides_modes.iter().filter(|p| !p.id.is_empty()) {
        // the game shows the title in cp1251, and a build driven over MCP never sees the
        // field's own warning
        if let Some(bad) = outside_cp1251(&mode.title) {
            out.push(
                Problem::warning(
                    "mod.mode_title",
                    format!("Mode '{}': the title has characters outside cp1251 ({bad}), the player will see a look-alike or '?'.", mode.id),
                )
                .about(mode.id.as_str()),
            );
        }
    }
    out
}

// ---- the manifest as written: lines and values ----------------------------------------------

/// One `key = value` line of the file.
struct Line<'a> {
    /// 1-based.
    number: u32,
    /// 1-based, the key's first character.
    column: u32,
    /// The section as written, and without case.
    section: &'a str,
    lower: String,
    key: &'a str,
    value: &'a str,
}

impl Line<'_> {
    /// The problem, put at this line: the key's place.
    fn place(&self, p: Problem) -> Problem {
        let width = u32::try_from(self.key.chars().count()).unwrap_or(1).max(1);
        p.at(self.number, self.column, self.number, self.column + width)
    }
}

/// The section's header, as a place for a problem about a key that is not written.
struct Header {
    number: u32,
    column: u32,
    /// The name as written between the brackets.
    written: String,
}

fn lines_of<'a>(doc: &'a ManifestDoc, raw: &'a [&'a str]) -> (Vec<Line<'a>>, Vec<(String, Header)>) {
    let aligned = raw.len() >= doc.items().count();
    let (mut out, mut headers) = (Vec::new(), Vec::new());
    let (mut section, mut lower) = ("", String::new());
    for (i, item) in doc.items().enumerate() {
        let number = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let column = if aligned {
            raw.get(i).map_or(1, |l| {
                u32::try_from(l.len() - l.trim_start_matches(crate::text::WS).len() + 1).unwrap_or(1)
            })
        } else {
            1
        };
        match item {
            crate::doc::Item::Section(name) => {
                section = name;
                lower = name.to_ascii_lowercase();
                headers.push((lower.clone(), Header { number, column, written: name.to_owned() }));
            }
            crate::doc::Item::Entry { key, value, .. } if !key.is_empty() => {
                out.push(Line { number, column, section, lower: lower.clone(), key, value });
            }
            _ => {}
        }
    }
    (out, headers)
}

/// The place of a key not written: its section's header, else the file's first line.
fn header_place(headers: &[(String, Header)], section: &str, p: Problem) -> Problem {
    match headers.iter().find(|(name, _)| name == section).or_else(|| headers.first()) {
        Some((_, h)) => p.at(h.number, h.column, h.number, h.column + 1),
        None => p.at(1, 1, 1, 2),
    }
}

/// `[section] key = value:`, how a problem about a written value begins.
fn lead(line: &Line<'_>) -> String {
    if trim(line.value).is_empty() {
        format!("[{}] {} is empty:", line.section, line.key)
    } else {
        format!("[{}] {} = {}:", line.section, line.key, line.value)
    }
}

/// The same line with `value`, as the fix of a problem.
fn fixed(line: &Line<'_>, value: &str) -> String {
    format!("{} = {value}", line.key)
}

/// Whether `a` becomes `b` by one letter more, less, changed or two neighbours swapped.
fn one_edit_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (a.get(common..).unwrap_or_default(), b.get(common..).unwrap_or_default());
    match (ra, rb) {
        ([], []) => false,
        ([_, rest_a @ ..], [_, rest_b @ ..]) if rest_a == rest_b => true,
        ([_, rest_a @ ..], rb) if rest_a == rb => true,
        (ra, [_, rest_b @ ..]) if ra == rest_b => true,
        ([x, y, rest_a @ ..], [p, q, rest_b @ ..]) => x == q && y == p && rest_a == rest_b,
        _ => false,
    }
}

/// The key of `keys` a written one is near: the same letters in another case, or - for a key of
/// four letters or more, so that `id` and `api` are not guessed at - one letter off (case aside).
fn near_key<'k>(keys: &[&'k str], written: &str) -> Option<&'k str> {
    keys.iter()
        .find(|k| k.eq_ignore_ascii_case(written))
        .or_else(|| {
            let lower = written.to_ascii_lowercase();
            keys.iter().find(|k| k.chars().count() >= 4 && one_edit_apart(k, &lower))
        })
        .copied()
}

/// A lower-case spelling the key takes, when the one written is no word of it.
fn lowered(check: Check, value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    (lower != value && accepts(check, &lower)).then_some(lower)
}

/// The problems of the values: every key the reference judges, written by hand or by a
/// command.
fn value_problems(line: &Line<'_>, doc_section: &'static SectionDoc, out: &mut Vec<Problem>) {
    let Some(doc) = key_of(doc_section, line.key) else {
        // only the sections an area reads keep a closed list of keys; the others keep what they
        // do not know, as written
        // a key of the module's own sections is kept whatever it is, but one that looks like a
        // key of this page is most likely that key misspelled
        if doc_section.reader == Reader::Manifest && matches!(doc_section.name, "module" | "provides_mode") {
            let keys: Vec<&str> = doc_section.keys.iter().map(|k| k.key).collect();
            if let Some(near) = near_key(&keys, line.key) {
                let case = near.eq_ignore_ascii_case(line.key);
                let text = if case {
                    format!(
                        "[{}] {}: keys are read exactly as spelled, and the key of this page is '{near}'. This one is kept as written and read by nobody.",
                        line.section, line.key
                    )
                } else {
                    format!(
                        "[{}] {}: no reader takes this key. It is kept as written and the game ignores it; the key of this page that it looks like is '{near}'.",
                        line.section, line.key
                    )
                };
                let p = Problem::warning("mod.key", text)
                    .with_fix(fixed(line, line.value).replacen(line.key, near, 1))
                    .in_field(format!("{}.{}", line.lower, line.key));
                out.push(line.place(p));
            }
        }
        if matches!(doc_section.reader, Reader::World | Reader::Screens) {
            let keys: Vec<&str> =
                doc_section.keys.iter().map(|k| k.key).filter(|k| !k.starts_with('<')).collect();
            let spelled = doc_section.keys.iter().find(|k| k.key.eq_ignore_ascii_case(line.key));
            let text = match spelled {
                Some(k) => format!(
                    "[{}] {}: keys are read exactly as spelled, and this one is '{}'. It is ignored as it stands.",
                    line.section, line.key, k.key
                ),
                None => format!(
                    "[{}] {}: no reader takes this key. It is kept as written and the game ignores it; the keys are {}.",
                    line.section,
                    line.key,
                    keys.join(", ")
                ),
            };
            let mut p = Problem::warning("mod.key", text);
            if let Some(k) = spelled {
                p = p.with_fix(fixed(line, line.value).replacen(line.key, k.key, 1));
            }
            out.push(line.place(p.in_field(format!("{}.{}", line.lower, line.key))));
        }
        return;
    };
    let value = trim(line.value);
    // an empty value is the key left out, except where the editor refuses it (the version)
    if (value.is_empty() && doc.check != Check::Version)
        || doc.check == Check::Id
        || doc.check == Check::OwnModeId
    {
        return;
    }
    let field = format!("{}.{}", line.lower, doc.key);
    let takes = format!("{} takes {}", line.key, doc.values);
    match doc.check {
        Check::Tags => tag_problems(line, &field, out),
        Check::Description if accepts(Check::Description, &text_value(value)) => {}
        check if check != Check::Description && accepts(check, value) => {}
        check => {
            let (code, error) = match check {
                Check::Version => ("mod.version", true),
                Check::Website => ("mod.website", true),
                Check::Target => ("mod.target_value", true),
                Check::Mode => ("mod.mode", true),
                Check::Repo => ("mod.github", true),
                Check::Description => ("mod.description", true),
                _ => ("mod.value", false),
            };
            let mut text = format!("{} {takes}.", lead(line));
            if !error {
                text.push_str(" The module still builds; the game does not read this value as written.");
            }
            let mut p = if error { Problem::error(code, text) } else { Problem::warning(code, text) };
            // the mechanical fixes: a pasted address, a word in the wrong case
            let fix = match check {
                Check::Repo => Some(normalize_github_repo(value)).filter(|r| validate_github_repo(r)),
                Check::Flag | Check::Choice(_) | Check::Target => lowered(check, value),
                // the game takes a number outside the range as the nearest end
                Check::Range(lo, hi) => value
                    .parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite())
                    .map(|x| x.clamp(f64::from(lo), f64::from(hi)).to_string()),
                _ => None,
            };
            if let Some(f) = fix {
                p = p.with_fix(fixed(line, &f));
            }
            out.push(line.place(p.in_field(field)));
        }
    }
}

fn tag_problems(line: &Line<'_>, field: &str, out: &mut Vec<Problem>) {
    let tags = split_list(line.value, ',');
    let unknown: Vec<&str> =
        tags.iter().map(String::as_str).filter(|t| !known_tag(&t.to_ascii_lowercase())).collect();
    let cased: Vec<&str> = tags
        .iter()
        .map(String::as_str)
        .filter(|t| known_tag(&t.to_ascii_lowercase()) && !known_tag(t))
        .collect();
    if !unknown.is_empty() {
        let words = unknown.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(", ");
        out.push(
            line.place(
                Problem::error(
                    "mod.tags",
                    format!(
                        "{} {words} {} no tag of the Mod Browser; tags take {}.",
                        lead(line),
                        if unknown.len() == 1 { "is" } else { "are" },
                        TAG_VOCABULARY.join(", ")
                    ),
                )
                .in_field(field),
            ),
        );
    }
    if !cased.is_empty() {
        let mut p = Problem::warning(
            "mod.tags_case",
            format!(
                "{} {} must be written in lower case: the Mod Browser compares tags exactly.",
                lead(line),
                cased.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(", ")
            ),
        )
        .in_field(field);
        if unknown.is_empty() {
            let all: Vec<String> = tags.iter().map(|t| t.to_ascii_lowercase()).collect();
            p = p.with_fix(fixed(line, &all.join(", ")));
        }
        out.push(line.place(p));
    }
}

/// What the game takes as the id of a screen or a track: lower case letters, digits and `_`.
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// The problems of the ids in `[screen.<id>]`, `[track.<id>]` and `[playlist.<id>]` headers: the
/// game reads the section name without case and drops one whose id holds anything but `a-z`,
/// `0-9` and `_`.
fn section_id_problems(headers: &[(String, Header)], out: &mut Vec<Problem>) {
    for (lower, h) in headers {
        let Some((kind, id)) =
            lower.split_once('.').filter(|(k, _)| matches!(*k, "screen" | "track" | "playlist"))
        else {
            continue;
        };
        if id.is_empty() || plain_id(id) {
            continue;
        }
        let made: String =
            id.chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { '_' }).collect();
        let p = Problem::warning(
            "mod.section_id",
            format!(
                "[{}]: the id of a {kind} takes a-z, 0-9 and _. The game drops this {kind} as it stands; screens.set would call it {made}.",
                h.written
            ),
        )
        .with_fix(format!("[{kind}.{made}]"))
        .in_field(lower.as_str());
        out.push(p.at(h.number, h.column, h.number, h.column + 1));
    }
}

/// The problems of `[vars]`: a name the runtime cannot read, a name declared twice.
fn var_problems(lines: &[Line<'_>], out: &mut Vec<Problem>) {
    let mut seen: Vec<&str> = Vec::new();
    for line in lines.iter().filter(|l| l.lower == "vars") {
        if !valid_global_name(line.key) {
            out.push(
                line.place(
                    Problem::error(
                        "mod.var_name",
                        format!("[vars] {}: a global variable's name takes {GLOBAL_NAME_TAKES}.", line.key),
                    )
                    .in_field(format!("vars.{}", line.key)),
                ),
            );
        }
        if seen.contains(&line.key) {
            out.push(
                line.place(
                    Problem::error(
                        "mod.var_twice",
                        format!("[vars] {}: {}.", line.key, global_twice(line.key)),
                    )
                    .in_field(format!("vars.{}", line.key)),
                ),
            );
        }
        seen.push(line.key);
    }
}

/// Where a problem of the cross-key rules belongs: its key's last line in `[module]`, a mode's
/// `id` line, else the section's header.
fn place_rule(p: Problem, lines: &[Line<'_>], headers: &[(String, Header)]) -> Problem {
    let found = if p.code == "mod.mode_title" {
        let id = p.subject.clone().unwrap_or_default();
        lines
            .iter()
            .rev()
            .find(|l| l.lower == "provides_mode" && l.key == "id" && trim(l.value) == id.as_str())
    } else {
        let key = p.field.clone().unwrap_or_default();
        lines.iter().rev().find(|l| l.lower == "module" && l.key == key.as_str())
    };
    let section = if p.code == "mod.mode_title" { "provides_mode" } else { "module" };
    match found {
        Some(line) => line.place(p),
        None => header_place(headers, section, p),
    }
}

/// The manifest's own check before a build, with its messages: the id, the targeting choice,
/// the declared modes, every value the reference judges (version, website, tags, target, the
/// mode ids, the repository, a global's name, the description, and the values and keys of
/// `[world]`, `[start_kit]`, `[screens]`, `[screen.<id>]`, `[track.<id>]`, `[playlist.<id>]` and
/// `[radio]`), the warnings about
/// the mode titles and about a module that turns the game's updates off. Every problem is at its
/// line of `mod.ltx`; a key the file leaves out is at its section's header. Console commands are
/// [`crate::validate_commands`]: they need the project's scripts.
#[must_use]
pub fn validate(doc: &ManifestDoc) -> Vec<Problem> {
    let m = Manifest::parse(doc);
    let text = doc.to_string();
    let raw: Vec<&str> = text.lines().collect();
    let (lines, headers) = lines_of(doc, &raw);
    let file = RelPath::new(FILE).ok();
    let mut out: Vec<Problem> =
        manifest_rules(&m).into_iter().map(|p| place_rule(p, &lines, &headers)).collect();
    for line in &lines {
        if let Some(section) = section_named(line.section) {
            value_problems(line, section, &mut out);
        }
    }
    // a mode the module adds: its id is judged where it is written
    for line in lines.iter().filter(|l| l.lower == "provides_mode" && l.key == "id") {
        let id = trim(line.value);
        if !id.is_empty() && !validate_own_mode_id(id) {
            let takes = key_doc("provides_mode", "id").map_or("", |(_, d)| d.values);
            out.push(
                line.place(
                    Problem::error("mod.mode_id", format!("{} id takes {takes}.", lead(line)))
                        .in_field("provides_mode.id")
                        .about(id),
                ),
            );
        }
    }
    var_problems(&lines, &mut out);
    section_id_problems(&headers, &mut out);
    out.sort_by_key(|p| (p.line.unwrap_or(0), p.column.unwrap_or(0), p.severity));
    if let Some(file) = file {
        for p in &mut out {
            p.file = Some(file.clone());
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The problems of a manifest's text.
    fn problems(text: &str) -> Vec<Problem> {
        validate(&ManifestDoc::parse(text))
    }

    /// A module of the ordinary game with one more line in `[module]`.
    fn with(lines: &str) -> String {
        format!("[module]\r\nid = od\r\ntarget = default\r\n{lines}")
    }

    fn codes(p: &[Problem]) -> Vec<&str> {
        p.iter().map(|p| p.code.as_str()).collect()
    }

    #[test]
    fn ids_and_versions() {
        assert!(validate_id("od.mod-2_x"));
        assert!(!validate_id("Od"));
        assert!(!validate_id(""));
        assert!(validate_mode_id(""));
        assert!(validate_mode_ref("*") && !validate_mode_id("*"));
        assert!(validate_own_mode_id("night") && !validate_own_mode_id(&"a".repeat(65)));
        assert!(validate_version("1.2.0"));
        assert!(validate_version("1"));
        assert!(!validate_version("1.2.3.4.5"));
        assert!(!validate_version("1..2"));
        assert!(!validate_version("1.2."));
        assert!(!validate_version("1234567890"));
        assert!(!validate_version("v1.0"));
        assert!(dotted_number("1.4.1", 3, 3) && !dotted_number("1.4", 3, 3));
    }

    #[test]
    fn websites() {
        assert!(validate_website(""));
        assert!(validate_website("https://ap-pro.ru/stuff/x/od-r123/"));
        assert!(validate_website("https://WWW.ModDB.com"));
        assert!(!validate_website("http://moddb.com/x"));
        assert!(!validate_website("https://moddb.com.evil.com/x"));
        assert!(!validate_website("https://user@moddb.com/x"));
        assert!(!validate_website("https://moddb.com:443/x"));
        assert!(!validate_website("https://moddb.com/a b"));
    }

    #[test]
    fn repositories() {
        assert!(validate_github_repo("Madmer/agroprom-story"));
        assert!(validate_github_repo(""));
        assert!(!validate_github_repo("a/b/c"));
        assert!(!validate_github_repo("a/.."));
        assert!(!validate_github_repo("noslash"));
        assert_eq!(normalize_github_repo(" https://github.com/Madmer/od/tree/main?tab=x "), "Madmer/od");
        assert_eq!(normalize_github_repo("GitHub.com/Madmer/od.git"), "Madmer/od");
        assert_eq!(normalize_github_repo("https://github.com/Madmer/od#readme"), "Madmer/od");
        assert_eq!(normalize_github_repo("Madmer/od"), "Madmer/od");
        assert!(valid_global_name("kills_2") && !valid_global_name("a-b"));
    }

    #[test]
    fn one_rule_for_the_command_the_form_and_the_check() {
        // the refusal of a command is the table's words, the same ones the check reads
        let why = refusal("module", "version", "v1.0").unwrap();
        assert!(why.starts_with("version takes 1 to 4 numbers separated by dots"), "{why}");
        assert!(why.contains("'v1.0'"), "{why}");
        assert!(refusal("module", "version", "1.0").is_none());
        assert!(refusal("module", "website", "http://moddb.com/x").is_some());
        assert!(refusal("module", "mode", "*").is_none() && refusal("module", "mode", "A b").is_some());
        assert!(refusal("provides_mode", "id", "*").is_some());
        assert!(refusal("module", "target", "sideways").unwrap().contains("default | existing | new | any"));
        assert!(refusal("update", "github", "not a repo").is_some());
        assert!(refusal("module", "name", "anything").is_none() && refusal("nothing", "x", "y").is_none());
        assert_eq!(
            global_name_refusal("a-b").unwrap(),
            "a global variable's name takes letters, digits and _, not 'a-b'"
        );
        assert!(parse_flag("yes") == Some(true) && parse_flag("Off").is_none());
    }

    #[test]
    fn the_first_problem_names_the_key_the_values_and_the_command() {
        let p = problems("[module]\r\nid = od\r\n");
        let text = &p[0].message;
        assert_eq!((p[0].code.as_str(), p[0].field.as_deref()), ("mod.target", Some("target")));
        assert_eq!(p[0].fix.as_deref(), Some("target = default"));
        // no target is written: the [module] header is its place
        assert_eq!((p[0].line, p[0].column), (Some(1), Some(1)));
        for word in [
            "target = default",
            "mode = <id>",
            "target = existing",
            "[provides_mode]",
            "target = new",
            "mod.set set=",
        ] {
            assert!(text.contains(word), "{word}: {text}");
        }
        // the key and the command come before the window
        assert!(text.find("mod.set").unwrap() < text.find("Mod Settings").unwrap());
        assert!(text.find("target = default").unwrap() < text.find("Mod Settings").unwrap());
    }

    #[test]
    fn the_build_check() {
        let p = problems("[module]\r\nid = Bad Id\r\n");
        assert_eq!(codes(&p), ["mod.target", "mod.id"]);
        assert!(problems("[module]\r\nid = od\r\ntarget = Default\r\n").is_empty());
        let p = problems(
            "[module]\r\nid = od\r\ntarget = new\r\ndisable_game_updates = true\r\n[provides_mode]\r\nid = Bad\r\ntitle = Ночь ★\r\n",
        );
        assert_eq!(codes(&p), ["mod.updates_off", "mod.mode_id", "mod.mode_title"]);
        // each at the line it is about
        assert_eq!(p.iter().map(|p| p.line.unwrap()).collect::<Vec<_>>(), [4, 6, 6]);
    }

    #[test]
    fn every_manifest_form_that_states_its_game_is_clean() {
        for text in [
            "[module]\r\nid = od\r\ntarget = default\r\n",
            "[module]\r\nid = od\r\ntarget = existing\r\nmode = dar2_mode\r\n",
            "[module]\r\nid = od\r\ntarget = new\r\n[provides_mode]\r\nid = night\r\ntitle = Night\r\n",
            "[module]\r\nid = od\r\ntarget = any\r\nmode = *\r\n",
            "[module]\r\nid = od\r\nmode = night\r\n",
            "[module]\r\nid = od\r\ntarget = default\r\nversion = 1.2.0\r\nwebsite = https://www.moddb.com/mods/x\r\ntags = story, quests\r\n[update]\r\ngithub = Madmer/od\r\n[vars]\r\nkills = 0\r\n",
        ] {
            let p = problems(text);
            assert!(p.is_empty(), "{text}: {p:?}");
        }
    }

    #[test]
    fn one_wrong_value_per_rule_gives_its_problem_at_its_line_with_its_code() {
        // (the lines after the three of `with`, the code, the line it is at, what it names)
        let cases: [(&str, &str, u32, &str); 11] = [
            ("version = v1.2-beta\r\n", "mod.version", 4, "version takes 1 to 4 numbers"),
            (
                "website = http://evil.example/x\r\n",
                "mod.website",
                4,
                "website takes an https link on ap-pro.ru",
            ),
            ("tags = banana\r\n", "mod.tags", 4, "'banana' is no tag"),
            (
                "description = \"x\"\r\n[update]\r\ngithub = not a repo\r\n",
                "mod.github",
                6,
                "github takes owner/repo",
            ),
            ("[vars]\r\nbad-name = 1\r\n", "mod.var_name", 5, "letters, digits and _"),
            ("[vars]\r\nkills = 0\r\nkills = 1\r\n", "mod.var_twice", 6, "declared twice"),
            ("[world]\r\nbogus_rule = true\r\n", "mod.key", 5, "no reader takes this key"),
            ("[world]\r\nmutants = maybe\r\n", "mod.value", 5, "mutants takes true | false"),
            (
                "[screens]\r\nloading_screens = sometimes\r\n",
                "mod.value",
                5,
                "loading_screens takes mix | only",
            ),
            ("[screen.a]\r\nrect = 1 2\r\n", "mod.value", 5, "rect takes x y width height"),
            ("[start_kit]\r\nmoney = rich\r\n", "mod.value", 5, "money takes a number"),
        ];
        for (extra, code, line, says) in cases {
            let p = problems(&with(extra));
            assert_eq!(p.len(), 1, "{extra}: {p:?}");
            assert_eq!(
                (p[0].code.as_str(), p[0].line, p[0].file.as_ref().map(RelPath::as_str)),
                (code, Some(line), Some("mod.ltx")),
                "{extra}"
            );
            assert!(p[0].message.contains(says), "{extra}: {}", p[0].message);
            // the problem names the key's line, the key and what was written
            assert!(p[0].message.starts_with('['), "{}", p[0].message);
        }
    }

    #[test]
    fn a_target_that_is_none_of_the_four_is_wrong_with_a_mode_too() {
        for text in [
            "[module]\r\nid = od\r\ntarget = sideways\r\n",
            "[module]\r\nid = od\r\ntarget = sideways\r\nmode = night\r\n",
        ] {
            let p = problems(text);
            assert_eq!(codes(&p), ["mod.target_value"], "{p:?}");
            assert_eq!(p[0].line, Some(3));
            assert!(
                p[0].message.contains("target = sideways")
                    && p[0].message.contains("default | existing | new | any")
            );
        }
        // a flag written in the wrong case is read as the word it is not
        let p = problems(&with("[world]\r\nmutants = False\r\n"));
        assert_eq!(p[0].fix.as_deref(), Some("mutants = false"));
    }

    #[test]
    fn warnings_are_warnings_and_the_errors_are_the_values_the_editor_refuses() {
        let p = problems(&with(
            "[world]\r\nbogus_rule = true\r\nmutants = maybe\r\n[screens]\r\nloading_screens = sometimes\r\n",
        ));
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|p| p.severity == gz_base::Severity::Warning));
        let p = problems(&with("version = v1.2-beta\r\ntags = banana\r\n"));
        assert!(p.iter().all(Problem::is_error) && p.len() == 2);
    }

    #[test]
    fn fixes_are_the_line_to_write() {
        let p = problems(&with("[update]\r\ngithub = https://github.com/Madmer/od\r\n"));
        assert_eq!(p[0].fix.as_deref(), Some("github = Madmer/od"));
        let p = problems(&with("tags = Story, quests\r\n"));
        assert_eq!(
            (p[0].code.as_str(), p[0].fix.as_deref()),
            ("mod.tags_case", Some("tags = story, quests"))
        );
        // a key spelled in another case is read by nobody but the five rules of [world]
        let p = problems(&with("[screens]\r\nLoading_Screens = only\r\n[world]\r\nMutants = false\r\n"));
        assert_eq!(codes(&p), ["mod.key"]);
        assert_eq!(p[0].fix.as_deref(), Some("loading_screens = only"));
    }

    #[test]
    fn unknown_keys_of_the_sections_the_editor_keeps_are_no_problem() {
        let p = problems(&with(
            "engine_min = 1.4\r\n[budget]\r\nspawn = 3\r\n[redirects]\r\na = b\r\n[nothing]\r\nx = y\r\n[provides_mode]\r\nweird = 1\r\n[start_kit]\r\nwpn_ak74 = 1\r\nmoney = 100\r\n[vfs]\r\nconfigs\\a.ltx = tuning\\a.ltx\r\n",
        ));
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn every_written_line_is_judged_and_the_place_is_the_keys() {
        // an indented line: the column is the key's, not the line's
        let p = problems("[module]\r\nid = od\r\ntarget = default\r\nversion = 1.0\r\n  version = x\r\n");
        assert_eq!((p.len(), p[0].line, p[0].column), (1, Some(5), Some(3)));
    }

    #[test]
    fn a_mode_that_is_no_mode_is_one_error_where_it_is_written() {
        let p = problems("[module]\r\nid = od\r\ntarget = existing\r\nmode = Not A Mode!\r\n");
        assert_eq!(codes(&p), ["mod.mode"]);
        assert_eq!(p[0].line, Some(4));
        assert!(p[0].message.contains("mode takes a game mode id"), "{}", p[0].message);
    }

    /// One more line after the three of [`with`], the problems it makes: code, line, words.
    fn one(extra: &str) -> Vec<(String, u32, String)> {
        problems(&with(extra))
            .iter()
            .map(|p| (p.code.to_string(), p.line.unwrap_or(0), p.message.clone()))
            .collect()
    }

    #[test]
    fn the_values_the_page_lists_and_the_check_let_pass_before() {
        // a flag, a layer's opacity, the api, an empty version: each at its line, in its words
        let p = one("[commands]\r\nod_a = od.a\r\n[command.od_a]\r\ncheat = maybe\r\n");
        assert_eq!((p[0].0.as_str(), p[0].1), ("mod.value", 7));
        assert!(p[0].2.contains("cheat takes true | false"), "{}", p[0].2);
        // the game reads these words in any case
        assert!(one("[commands]\r\nod_a = od.a\r\n[command.od_a]\r\ncheat = True\r\n").is_empty());
        let p = problems(&with("[screen.a]\r\nlayer = a.dds\r\nopacity = 7\r\n"));
        assert_eq!(
            (p[0].code.as_str(), p[0].line, p[0].fix.as_deref()),
            ("mod.value", Some(6), Some("opacity = 1"))
        );
        assert!(p[0].message.contains("opacity takes 0 to 1"), "{}", p[0].message);
        assert!(
            one("[screen.a]\r\nlayer = a.dds\r\nopacity = 0.5\r\nweight = 2000\r\n")[0]
                .2
                .contains("weight takes a number, 0 to 1000")
        );
        assert!(one("[track.t]\r\nfile = a.ogg\r\nvolume = 1.5\r\n")[0].2.contains("volume takes 0 to 1"));
        let p = one("api = banana\r\n");
        assert_eq!((p[0].0.as_str(), p[0].1), ("mod.value", 4));
        assert!(p[0].2.contains("api takes 1"), "{}", p[0].2);
        assert!(one("api = 1\r\n").is_empty());
        let p = problems(&with("version =\r\n"));
        assert_eq!((p[0].code.as_str(), p[0].line), ("mod.version", Some(4)));
        assert!(
            p[0].message.starts_with("[module] version is empty: version takes 1 to 4 numbers"),
            "{}",
            p[0].message
        );
        // an empty value of a key that takes anything else is the key left out
        assert!(one("tags =\r\nwebsite =\r\nmode =\r\n[update]\r\ngithub =\r\n").is_empty());
    }

    #[test]
    fn an_id_the_game_would_drop_is_a_warning_at_its_header() {
        let p = problems(&with(
            "[screen.Bad Id]\r\nlayer = a.dds\r\n[track.Fine_2]\r\nfile = a.ogg\r\n[track.a-b]\r\n",
        ));
        assert_eq!(codes(&p), ["mod.section_id", "mod.section_id"]);
        assert!(p.iter().all(|p| !p.is_error()));
        assert_eq!((p[0].line, p[0].fix.as_deref()), (Some(4), Some("[screen.bad_id]")));
        assert_eq!((p[1].line, p[1].fix.as_deref()), (Some(8), Some("[track.a_b]")));
        assert!(
            p[0].message.starts_with("[screen.Bad Id]: the id of a screen takes a-z, 0-9 and _."),
            "{}",
            p[0].message
        );
        // upper case is read as lower case: the game does not drop it
        assert!(problems(&with("[Screen.Night]\r\nlayer = a.dds\r\n")).is_empty());
        let p = problems(&with("[playlist.Road Songs]\r\ntracks = a\r\n"));
        assert_eq!((codes(&p), p[0].fix.as_deref()), (vec!["mod.section_id"], Some("[playlist.road_songs]")));
    }

    #[test]
    fn the_radio_keys_are_judged_at_their_lines() {
        // what the radio takes passes
        assert!(
            one("[track.t]\r\nfile = a.ogg\r\nplay = menu, radio\r\ncopyrighted = true\r\nstreamer = u\r\n[playlist.p]\r\ntitle = Road\r\ntracks = t\r\norder = list\r\n[radio]\r\nmusic = only\r\n")
                .is_empty()
        );
        // a word of the screens' show is the screens': radio is no surface a screen shows on
        assert!(
            one("[screen.s]\r\nshow = radio\r\nlayer = a.dds\r\n")[0]
                .2
                .contains("show takes loading, menu, pause")
        );
        let p = one("[playlist.p]\r\norder = random\r\n");
        assert_eq!((p[0].0.as_str(), p[0].1), ("mod.value", 5));
        assert!(p[0].2.contains("order takes shuffle | list"), "{}", p[0].2);
        assert!(one("[radio]\r\nmusic = all\r\n")[0].2.contains("music takes mix | only"));
        assert!(one("[track.t]\r\ncopyrighted = maybe\r\n")[0].2.contains("copyrighted takes true | false"));
        assert!(one("[radio]\r\nvolume = 1\r\n")[0].2.contains("no reader takes this key"));
    }

    #[test]
    fn a_key_near_a_key_of_the_module_is_a_warning_and_a_far_one_is_not() {
        for (written, near) in [
            ("versoin", "version"),
            ("Tags", "tags"),
            ("desciption", "description"),
            ("webste", "website"),
            ("authors", "author"),
            ("MODE", "mode"),
        ] {
            let p = problems(&with(&format!("{written} = 2.0\r\n")));
            assert_eq!(p.len(), 1, "{written}: {p:?}");
            assert_eq!((p[0].code.as_str(), p[0].line), ("mod.key", Some(4)), "{written}");
            assert_eq!(p[0].fix.as_deref(), Some(format!("{near} = 2.0").as_str()), "{written}");
            assert!(p[0].message.contains(&format!("'{near}'")), "{}", p[0].message);
            assert!(!p[0].is_error());
        }
        // far from every key, or too short to guess at: kept and said nothing about
        assert!(one("engine_min = 1.4\r\nnotes = x\r\nap = 1\r\nids = 3\r\n").is_empty());
        let p = problems(&with("[provides_mode]\r\nid = night\r\nTitel = x\r\nweird = 1\r\n"));
        assert_eq!(codes(&p), ["mod.key"]);
        assert_eq!(p[0].fix.as_deref(), Some("title = x"));
        // another section keeps what it does not know
        assert!(problems(&with("[update]\r\nGithub = a/b\r\n[order]\r\nafterr = x\r\n")).is_empty());
    }

    /// The problems of a manifest that states `target` and `mode` and, when `own`, a mode of its own.
    fn pair(target: &str, mode: &str, own: bool) -> Vec<Problem> {
        let mut text = format!("[module]\r\nid = od\r\ntarget = {target}\r\n");
        if !mode.is_empty() {
            text.push_str(&format!("mode = {mode}\r\n"));
        }
        if own {
            text.push_str("[provides_mode]\r\nid = night\r\n");
        }
        problems(&text)
    }

    #[test]
    fn a_target_and_a_mode_go_together_as_the_page_pairs_them() {
        // the pairs that do
        for (target, mode, own) in [
            ("default", "", false),
            ("existing", "night", false),
            ("new", "", true),
            ("new", "night", true),
            ("any", "*", false),
            ("any", "*", true),
            ("Default", "", false),
        ] {
            let p = pair(target, mode, own);
            assert!(p.is_empty(), "{target} {mode} {own}: {p:?}");
        }
        // the pairs that do not: each its own code, at the target line, with the two lines said
        let cases = [
            (
                "default",
                "night",
                false,
                "mod.target_default",
                "[module] target = default with mode = night: target = default is the ordinary game and goes with no mode.",
                "target = existing",
            ),
            (
                "default",
                "*",
                false,
                "mod.target_default",
                "[module] target = default with mode = *:",
                "target = any",
            ),
            (
                "existing",
                "",
                false,
                "mod.target_existing",
                "[module] target = existing with no mode: target = existing is a mode of the linked game and goes with that mode's id.",
                "target = default",
            ),
            (
                "existing",
                "*",
                false,
                "mod.target_existing",
                "[module] target = existing with mode = *:",
                "target = any",
            ),
            (
                "new",
                "",
                false,
                "mod.target_new",
                "[module] target = new with no mode: target = new is a mode the module adds and goes with a [provides_mode] section.",
                "target = default",
            ),
            (
                "new",
                "night",
                false,
                "mod.target_new",
                "[module] target = new with mode = night:",
                "target = default",
            ),
            (
                "any",
                "night",
                false,
                "mod.target_any",
                "[module] target = any with mode = night: target = any is every game and goes with mode = *.",
                "target = existing",
            ),
            (
                "any",
                "",
                false,
                "mod.target_any",
                "[module] target = any with no mode: target = any is every game and goes with mode = *.",
                "mode = *",
            ),
            ("any", "", true, "mod.target_any", "[module] target = any with no mode:", "mode = *"),
        ];
        for (target, mode, own, code, says, fix) in cases {
            let p = pair(target, mode, own);
            assert_eq!(codes(&p), [code], "{target} {mode} {own}: {p:?}");
            assert!(p[0].is_error() && p[0].line == Some(3), "{target} {mode}: {:?}", p[0]);
            assert!(p[0].message.starts_with(says), "{}", p[0].message);
            assert_eq!(p[0].fix.as_deref(), Some(fix), "{target} {mode}");
        }
        // target = any alone is no new project's "no target game mode"
        let p = pair("any", "", false);
        assert!(!p[0].message.contains("no target game mode") && p[0].code != "mod.target");
        // a wrong target says that and only that; no target at all is the new project's sentence
        assert_eq!(codes(&pair("sideways", "night", false)), ["mod.target_value"]);
        assert_eq!(codes(&problems("[module]\r\nid = od\r\n")), ["mod.target"]);
    }

    #[test]
    fn the_pairing_is_the_one_sentence_the_check_and_mod_set_say() {
        let p = target_pairing("default", "night", false).unwrap();
        let checked = pair("default", "night", false);
        assert_eq!(checked[0].message, p.text);
        assert!(target_pairing("existing", "night", false).is_none());
        assert!(
            target_pairing("", "night", false).is_none() && target_pairing("sideways", "", false).is_none()
        );
    }
}
