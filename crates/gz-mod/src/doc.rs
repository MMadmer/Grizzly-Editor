//! `mod.ltx` as written: `gz_ltx`'s lossless line model read by the manifest's rules.
//!
//! The lines are `gz_ltx::LtxDoc`'s, read with [`Syntax::Manifest`] - the way the manifest
//! loader and the game's XMS reader read them: the line is trimmed, a `;` outside a
//! quoted value starts a comment (`\"` escapes inside, so `website = https://...` keeps its
//! `//`), a `[` starts a header whose name ends at the first `]`, a key ends at the first `=`.
//! What this crate adds is the manifest's meaning: where a section and a
//! key are written, and the widths keys are padded to.

use std::fmt;

use gz_ltx::{LtxDoc, Placement, Syntax};

pub use gz_ltx::LineItem as Item;

/// A section `gz-screens` reads (lower case): the screens, the music and the radio.
fn screens_section(s: &str) -> bool {
    s == "screens"
        || s == "radio"
        || s.starts_with("screen.")
        || s.starts_with("track.")
        || s.starts_with("playlist.")
}

/// The pad of a key in the editor's own lines: 11 ("description"), 15 for the
/// longer keys of `[world]` and its kind, 17 for the screens, the music and the radio.
#[must_use]
pub fn key_width(section: &str) -> usize {
    let s = section.to_ascii_lowercase();
    if matches!(s.as_str(), "world" | "start_kit" | "vars" | "commands") {
        15
    } else if screens_section(&s) {
        17
    } else {
        11
    }
}

/// Where a section is written; sections it does not know close the file.
fn section_rank(section: &str) -> usize {
    let s = section.to_ascii_lowercase();
    const ORDER: [&str; 12] = [
        "module",
        "update",
        "links",
        "requires",
        "order",
        "conflicts",
        "provides_mode",
        "exclude",
        "world",
        "start_kit",
        "vars",
        "pda",
    ];
    if let Some(i) = ORDER.iter().position(|o| *o == s) {
        return i;
    }
    if screens_section(&s) {
        ORDER.len()
    } else if s == "commands" {
        ORDER.len() + 1
    } else if s.starts_with("command.") {
        ORDER.len() + 2
    } else {
        usize::MAX
    }
}

/// Where a key is written inside its section; `usize::MAX` for a key written after the
/// known ones (an item of the start kit, a line it keeps as it was).
fn key_rank(section: &str, key: &str) -> usize {
    let s = section.to_ascii_lowercase();
    let order: &[&str] = match s.as_str() {
        "module" => &[
            "id",
            "name",
            "version",
            "author",
            "description",
            "website",
            "disable_game_updates",
            "thumbnail",
            "tags",
            "api",
            "mode",
            "target",
        ],
        "update" => &["github"],
        "links" => &["ap_pro", "moddb", "telegram", "discord", "boosty", "patreon"],
        "order" => &["after", "before"],
        "provides_mode" => &["id", "title"],
        "world" => &[
            "original_npcs",
            "random_stalkers",
            "mutants",
            "npc_quests",
            "story_quests",
            "player_choice",
            "levels",
            "area",
            "locked_levels",
            "locked",
        ],
        "start_kit" => &["money", "perks"],
        _ if s.starts_with("command.") => &["help", "args", "cheat"],
        _ => &[],
    };
    order.iter().position(|o| *o == key).unwrap_or(usize::MAX)
}

/// The manifest's placement of new lines: the section order and the key widths.
const PLACEMENT: Placement = Placement { section_rank, key_rank, key_width };

/// `key = value` with the key padded to `width`.
#[must_use]
pub fn entry_line(key: &str, value: &str, width: usize) -> String {
    gz_ltx::doc::padded_entry(key, value, width)
}

/// An LTX file that writes back exactly what it read, except for the lines an edit touched.
#[derive(Clone, Debug)]
pub struct ManifestDoc(LtxDoc);

impl Default for ManifestDoc {
    fn default() -> Self {
        Self(LtxDoc::new_as(Syntax::Manifest, PLACEMENT))
    }
}

impl ManifestDoc {
    /// Reads a document. Never fails: a line that is not LTX is kept as it is. A BOM is dropped.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self(LtxDoc::parse_as(text, Syntax::Manifest, PLACEMENT))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every line, in file order.
    pub fn items(&self) -> impl Iterator<Item = Item<'_>> {
        self.0.items()
    }

    /// The section headers in file order (a section written twice is named twice).
    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.0.sections()
    }

    /// The sections whose name starts with `prefix` (ASCII case ignored), each once, in file
    /// order: `screen.` gives every `[screen.<id>]`.
    #[must_use]
    pub fn sections_with_prefix(&self, prefix: &str) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for name in self.0.sections() {
            let hit = name.len() >= prefix.len()
                && name.get(..prefix.len()).is_some_and(|h| h.eq_ignore_ascii_case(prefix));
            if hit && !out.iter().any(|o| o.eq_ignore_ascii_case(name)) {
                out.push(name);
            }
        }
        out
    }

    #[must_use]
    pub fn has_section(&self, section: &str) -> bool {
        self.0.has_section(section)
    }

    /// The `(key, value)` lines of a section in file order, every header of it and duplicates
    /// included; values as written (quotes included).
    #[must_use]
    pub fn section_entries(&self, section: &str) -> Vec<(&str, &str)> {
        self.0.entries(section).collect()
    }

    /// The entry lines of a section without their comments: what is kept of a raw-line
    /// section (`[requires]`, `[exclude]`).
    #[must_use]
    pub fn section_lines(&self, section: &str) -> Vec<&str> {
        self.0.section_lines(section)
    }

    /// The value of a key as written, the last one when written twice.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.0.get(section, key)
    }

    /// Sets a key. An existing line (the last of that key) keeps its key, its spacing and its
    /// comment; a new line goes where the section's order puts it, shaped like its neighbours; a
    /// missing section is added where its order puts it.
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        self.0.set(section, key, value);
    }

    /// Replaces the first entry whose key matches `key` without ASCII case by `raw`. `false`
    /// when there is none.
    pub fn set_line_ci(&mut self, section: &str, key: &str, raw: &str) -> bool {
        self.0.set_line_ci(section, key, raw)
    }

    /// Adds `raw` after the section's last entry.
    pub fn push_line(&mut self, section: &str, raw: &str) {
        self.0.push_line(section, raw);
    }

    /// Removes every entry of `section` whose key `pred` accepts. Returns whether one went.
    pub fn remove_where(&mut self, section: &str, pred: impl Fn(&str) -> bool) -> bool {
        self.0.remove_where(section, pred)
    }

    /// Removes every line of a key. Returns whether there was one.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        self.0.remove(section, key)
    }

    /// Replaces the entries of a section by `lines` (raw entry text, in that order), where its
    /// first entry was. Comments in the section stay where they are; a section written twice
    /// keeps its second header with no entries. A missing section is added unless `lines` is
    /// empty.
    pub fn set_lines(&mut self, section: &str, lines: &[String]) {
        self.0.set_lines(section, lines);
    }

    /// Removes a section (every header of that name) with its body. A comment block that leans
    /// on the next header stays with that header. Returns whether the section existed.
    pub fn remove_section(&mut self, section: &str) -> bool {
        self.0.remove_section(section)
    }

    /// Removes a section that holds nothing but blank lines: what the editor leaves when its
    /// last key goes. A section with a comment of the author's stays.
    pub fn prune(&mut self, section: &str) -> bool {
        self.0.prune(section)
    }
}

impl fmt::Display for ManifestDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SAMPLE: &str = "; my module\r\n[module]\r\nid          = demo\r\nname        = \"Over; Night\" ; the title\r\nversion     = 1.2.0\r\nwebsite     = https://www.moddb.com/mods/demo\r\n\r\n; what is left out\r\n[exclude]\r\ndocs\r\n*.psd\r\n";

    #[test]
    fn untouched_is_identical() {
        for text in [SAMPLE, "", "a = 1", "[s]\nk=v\n", "\r\n\r\n", "[s]\r\nk = \"x\" ; c\r\nz"] {
            assert_eq!(ManifestDoc::parse(text).to_string(), text);
        }
        assert_eq!(ManifestDoc::parse("\u{feff}[s]\n").to_string(), "[s]\n");
    }

    #[test]
    fn reads_like_the_manifest_loader() {
        let doc = ManifestDoc::parse(SAMPLE);
        assert_eq!(doc.get("MODULE", "website"), Some("https://www.moddb.com/mods/demo"));
        assert_eq!(doc.get("module", "name"), Some("\"Over; Night\""));
        assert_eq!(doc.section_lines("exclude"), ["docs", "*.psd"]);
        assert_eq!(doc.get("module", "ID"), None);
        let twice = ManifestDoc::parse("[s]\nk = 1\n[t]\n[S]\nk = 2\n");
        assert_eq!(twice.get("s", "k"), Some("2"));
        assert_eq!(twice.section_entries("s").len(), 2);
    }

    #[test]
    fn new_keys_and_sections_go_where_the_manifest_order_puts_them() {
        let text = "[module]\r\nid          = demo\r\nwebsite     = x\r\ntags        = story\r\nmy_key      = 1\r\n\r\n[update]\r\ngithub      = a/b\r\n\r\n; the mode\r\n[provides_mode]\r\nid          = demo\r\n\r\n[custom]\r\nk = v\r\n";
        let mut doc = ManifestDoc::parse(text);
        doc.set("module", "disable_game_updates", "true");
        doc.push_line("requires", "base");
        doc.set("module", "target", "new");
        doc.set_lines("vars", &["night           = 0".to_owned()]);
        assert_eq!(
            doc.to_string(),
            "[module]\r\nid          = demo\r\nwebsite     = x\r\ndisable_game_updates = true\r\ntags        = story\r\ntarget      = new\r\nmy_key      = 1\r\n\r\n[update]\r\ngithub      = a/b\r\n\r\n[requires]\r\nbase\r\n\r\n; the mode\r\n[provides_mode]\r\nid          = demo\r\n\r\n[vars]\r\nnight           = 0\r\n\r\n[custom]\r\nk = v\r\n"
        );
    }

    #[test]
    fn an_edit_touches_its_line_only() {
        let mut doc = ManifestDoc::parse(SAMPLE);
        doc.set("module", "version", "1.3.0");
        assert_eq!(doc.to_string(), SAMPLE.replace("1.2.0", "1.3.0"));
        doc.set("module", "name", "\"New\"");
        assert!(doc.to_string().contains("name        = \"New\" ; the title\r\n"));
    }

    #[test]
    fn a_new_key_follows_the_column() {
        let mut doc = ManifestDoc::parse(SAMPLE);
        doc.set("module", "author", "Madmer");
        assert!(
            doc.to_string().contains(
                "version     = 1.2.0\r\nauthor      = Madmer\r\nwebsite     = https://www.moddb.com/mods/demo\r\n\r\n; what"
            )
        );
        doc.set("update", "github", "Madmer/demo");
        // before [exclude] and the comment that leans on it, as the sections are ordered
        assert!(doc.to_string().contains(
            "demo\r\n\r\n[update]\r\ngithub      = Madmer/demo\r\n\r\n; what is left out\r\n[exclude]\r\n"
        ));
        let mut fresh = ManifestDoc::parse("[module]\nid=x");
        fresh.set("module", "api", "1");
        assert_eq!(fresh.to_string(), "[module]\nid=x\napi=1\n");
    }

    #[test]
    fn lines_replace_and_keep_comments() {
        let mut doc = ManifestDoc::parse("[vars]\n; counters\nkills = 0\nmet   = false\n\n[next]\n");
        doc.set_lines("vars", &["stage = 1".to_owned(), "met = true".to_owned()]);
        assert_eq!(doc.to_string(), "[vars]\n; counters\nstage = 1\nmet = true\n\n[next]\n");
        doc.set_lines("vars", &[]);
        assert_eq!(doc.to_string(), "[vars]\n; counters\n\n[next]\n");
        assert!(!doc.prune("vars"));
        let mut empty = ManifestDoc::parse("[a]\nk = 1\n\n[exclude]\ndocs\n");
        empty.set_lines("exclude", &[]);
        assert!(empty.prune("exclude"));
        assert_eq!(empty.to_string(), "[a]\nk = 1\n");
    }

    #[test]
    fn removing_and_case_blind_lines() {
        let mut doc =
            ManifestDoc::parse("[commands]\nStatus = a.b\nother = c.d\n\n[command.status]\nhelp = x\n");
        assert!(doc.set_line_ci("commands", "status", "status          = a.c"));
        assert!(doc.remove_section("COMMAND.STATUS"));
        assert_eq!(doc.to_string(), "[commands]\nstatus          = a.c\nother = c.d\n");
        assert!(doc.remove_where("commands", |k| k.eq_ignore_ascii_case("OTHER")));
        doc.push_line("commands", "next            = e.f");
        assert_eq!(doc.to_string(), "[commands]\nstatus          = a.c\nnext            = e.f\n");
        assert_eq!(
            ManifestDoc::parse("[screen.a]\n[track.x]\n[Screen.B]\n[screen.a]\n")
                .sections_with_prefix("screen."),
            ["screen.a", "Screen.B"]
        );
    }
}
