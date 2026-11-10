//! Text helpers shared by the outline, the pickers, the validator and the commands (`NqText`,
//! the string half of `NqUtil`).

use gz_luadata::Value;

/// The display string of a `text` value: a string as it is, a table by `rus`, then `eng`, then
/// its first keyed value (`NqText::Preview`).
#[must_use]
pub fn preview(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        Value::Table(t) => {
            if let Some(r) = t.get("rus") {
                return r.to_text("");
            }
            if let Some(e) = t.get("eng") {
                return e.to_text("");
            }
            t.fields.first().map_or_else(String::new, |(_, f)| f.to_text(""))
        }
        other => other.to_text(""),
    }
}

/// `[a-z0-9_]+`.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// The id a file name stands for: lower case, any byte outside `[a-z0-9_]` as `_`
/// (`NqText::IdFromName`; byte by byte, so a Cyrillic letter is two `_`).
#[must_use]
pub fn id_from_name(name: &str) -> String {
    name.bytes()
        .map(|b| {
            let c = b.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' { c as char } else { '_' }
        })
        .collect()
}

/// At most `max_bytes` of `s`, cut at the start of a code point, with `...` when cut
/// (`NqUtil::ClipUtf8`).
#[must_use]
pub fn clip_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut cut = max_bytes;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut r = s.get(..cut).unwrap_or("").to_owned();
    r.push_str("...");
    r
}

/// Trims spaces, tabs and line ends (`NqUtil::Trim`).
#[must_use]
pub fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\r', '\n'])
}

/// Splits at `sep`, every piece trimmed, empty pieces dropped (`NqUtil::Split`).
#[must_use]
pub fn split(s: &str, sep: char) -> Vec<String> {
    s.split(sep).map(trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
}

/// C's `atoi`: leading white space, a sign, the digits that follow; 0 without any.
#[must_use]
pub fn c_atoi(s: &str) -> i64 {
    let b = s.as_bytes();
    let mut i = 0;
    while b.get(i).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)) {
        i += 1;
    }
    let neg = match b.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut v: i64 = 0;
    while let Some(d) = b.get(i).filter(|c| c.is_ascii_digit()) {
        v = v.saturating_mul(10).saturating_add(i64::from(d - b'0'));
        i += 1;
    }
    if neg { -v } else { v }
}

/// C's `atof`: the longest prefix that reads as a decimal number; 0 without one.
#[must_use]
pub fn c_atof(s: &str) -> f64 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r']);
    let b = t.as_bytes();
    let mut end = 0;
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i = 1;
    }
    let mut digits = false;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
        digits = true;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
            digits = true;
        }
    }
    if digits {
        end = i;
        if matches!(b.get(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(b.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            let start = j;
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            if j > start {
                end = j;
            }
        }
    }
    t.get(..end).and_then(|p| p.parse::<f64>().ok()).unwrap_or(0.0)
}

/// Whether every character of a UTF-8 text has a cp1251 form; the first six that do not, joined
/// with spaces (`NqUtil::Cp1251Covers`).
#[must_use]
pub fn cp1251_covers(text: &str) -> (bool, String) {
    let mut seen: Vec<char> = Vec::new();
    let mut bad = String::new();
    let mut buf = [0u8; 4];
    for c in text.chars() {
        if c.is_ascii() {
            continue;
        }
        if gz_base::text::fits_cp1251(c.encode_utf8(&mut buf)) {
            continue;
        }
        if seen.contains(&c) {
            continue;
        }
        seen.push(c);
        if seen.len() > 6 {
            break;
        }
        if !bad.is_empty() {
            bad.push(' ');
        }
        bad.push(c);
    }
    (seen.is_empty(), bad)
}

/// ASCII and Russian letters folded to lower case, byte by byte the way the C++ folded UTF-8
/// (`FoldSearch` / `FoldNoCase`: А..Я to а..я, Ё to ё).
#[must_use]
pub fn fold(text: &str) -> String {
    let b = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c.is_ascii_uppercase() {
            out.push(c + 0x20);
            i += 1;
            continue;
        }
        if c == 0xD0
            && let Some(&d) = b.get(i + 1)
        {
            if d == 0x81 {
                out.extend_from_slice(&[0xD1, 0x91]);
                i += 2;
                continue;
            }
            if (0x90..=0x9F).contains(&d) {
                out.extend_from_slice(&[0xD0, d + 0x20]);
                i += 2;
                continue;
            }
            if (0xA0..=0xAF).contains(&d) {
                out.extend_from_slice(&[0xD1, d - 0x20]);
                i += 2;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// `hay` holds `needle`, both folded; an empty needle is in everything (`ContainsNoCase`).
#[must_use]
pub fn contains_no_case(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if hay.len() < needle.len() {
        return false;
    }
    fold(hay).contains(&fold(needle))
}

/// One line of a reference table for the outline: `story:esc_m_trader`, `ref:boars`,
/// `spawn simulation_boar@smart ref=boars`, `level:x pos=(..) r=5` (`NqText::RefSummary`).
#[must_use]
pub fn ref_summary(v: &Value) -> String {
    if !v.is_table() {
        return v.to_text("");
    }
    if let Some(sp) = v.get("spawn") {
        let mut out = String::from("spawn ");
        out.push_str(&sp.get_string("section", ""));
        if sp.has("smart") {
            out.push('@');
            out.push_str(&sp.get_string("smart", ""));
        }
        if sp.has("ref") {
            out.push_str(" ref=");
            out.push_str(&sp.get_string("ref", ""));
        }
        return out;
    }
    const ALT: [&str; 7] = ["story", "ref", "character", "profile", "smart", "restrictor", "community"];
    for alt in ALT {
        if v.has(alt) {
            let mut out = format!("{alt}:{}", v.get_string(alt, ""));
            if alt == "community" && v.has("level") {
                out.push('@');
                out.push_str(&v.get_string("level", ""));
            }
            return out;
        }
    }
    if v.has("level") {
        let mut out = format!("level:{}", v.get_string("level", ""));
        if let Some(pos) = v.get("pos")
            && pos.is_table()
            && let [x, y, z, ..] = pos.arr()
        {
            out.push_str(&format!(" pos=({},{},{})", x.to_text(""), y.to_text(""), z.to_text("")));
        }
        if v.has("radius") {
            out.push_str(" r=");
            out.push_str(&v.get_string("radius", ""));
        }
        return out;
    }
    if v.has("seconds") {
        return format!("{}s", v.get_string("seconds", ""));
    }
    if v.has("game_hours") {
        return format!("{}gh", v.get_string("game_hours", ""));
    }
    if v.has("game_minutes") {
        return format!("{}gm", v.get_string("game_minutes", ""));
    }
    value_summary(v, 48)
}

/// A short summary of a value: strings quoted and clipped, tables as `k=v`
/// (`NqText::ValueSummary`).
#[must_use]
pub fn value_summary(v: &Value, max_len: usize) -> String {
    match v {
        Value::Nil => "nil".to_owned(),
        Value::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
        Value::Number(_) => v.to_text(""),
        Value::String(s) => format!("\"{}\"", clip_utf8(s, max_len)),
        Value::Table(t) => {
            if v.has("rus") || v.has("eng") {
                return value_summary(&Value::from(preview(v)), max_len);
            }
            if v.has("character") && t.fields.len() == 1 {
                return ref_summary(v);
            }
            if v.has("character") {
                return v.get_string("character", "");
            }
            if v.has("section") && v.has("ref") && !v.has("smart") && !v.has("place") {
                return v.get_string("section", "");
            }
            const REFS: [&str; 11] =
                ["story", "ref", "profile", "smart", "spawn", "restrictor", "community", "level", "seconds", "game_hours", "game_minutes"];
            if REFS.iter().any(|k| v.has(k)) {
                return ref_summary(v);
            }
            let mut out = String::from("{");
            for (i, e) in t.arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if e.is_table() && e.has("name") {
                    out.push_str(&e.get_string("name", ""));
                } else {
                    out.push_str(&value_summary(e, 16));
                }
            }
            for (i, (k, f)) in t.fields.iter().enumerate() {
                if i > 0 || !t.arr.is_empty() {
                    out.push(',');
                }
                out.push_str(k);
                out.push('=');
                out.push_str(&value_summary(f, 16));
            }
            out.push('}');
            if out.len() > max_len + 8 {
                out = clip_utf8(&out, max_len);
                out.push('}');
            }
            out
        }
    }
}

/// `dialog.npc_phrase` / `dialog.actor_phrase`.
#[must_use]
pub fn is_phrase_kind(kind: &str) -> bool {
    kind == "dialog.npc_phrase" || kind == "dialog.actor_phrase"
}

/// `dialog.topic`.
#[must_use]
pub fn is_topic_kind(kind: &str) -> bool {
    kind == "dialog.topic"
}

/// A topic or a phrase.
#[must_use]
pub fn is_dialog_kind(kind: &str) -> bool {
    is_topic_kind(kind) || is_phrase_kind(kind)
}

/// `trigger.*`: the catalog-free shortcut of [`crate::Catalog::is_trigger`].
#[must_use]
pub fn is_trigger_kind(kind: &str) -> bool {
    kind.starts_with("trigger.")
}

/// `quest` / `Quest` (and their plurals) as whole words turned into `graph`: how a behaviour
/// graph speaks of itself (`Regraph`).
#[must_use]
pub fn regraph(text: &str) -> String {
    let b = text.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let start = i == 0 || b.get(i - 1).is_none_or(|&c| !word(c));
        let here = b.get(i..i + 5);
        if start && (here == Some(b"quest") || here == Some(b"Quest")) {
            let mut after = i + 5;
            let plural = b.get(after) == Some(&b's');
            if plural {
                after += 1;
            }
            if b.get(after).is_none_or(|&c| !word(c)) {
                out.push_str(if b.get(i) == Some(&b'Q') { "Graph" } else { "graph" });
                if plural {
                    out.push('s');
                }
                i = after;
                continue;
            }
        }
        // copy one whole character
        let len = text.get(i..).and_then(|r| r.chars().next()).map_or(1, char::len_utf8);
        out.push_str(text.get(i..i + len).unwrap_or(""));
        i += len;
    }
    out
}

/// The title of a kind as the document speaks: a behaviour graph never says "quest"
/// (`NqLook::KindTitle`).
#[must_use]
pub fn kind_title(catalog: &crate::Catalog, kind: &str, behaviour: bool) -> String {
    let title = catalog.find(kind).map_or_else(|| kind.to_owned(), |k| k.title.clone());
    if !behaviour {
        return title;
    }
    if let Some(rest) = title.strip_prefix("Quest ")
        && !rest.is_empty()
    {
        let mut r = rest.to_owned();
        if let Some(first) = r.get(..1)
            && first.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        {
            let up = first.to_ascii_uppercase();
            r.replace_range(..1, &up);
        }
        return r;
    }
    regraph(&title)
}

/// What the document is called in a sentence: `quest` or `graph` (`NqLook::DocNoun`).
#[must_use]
pub fn doc_noun(behaviour: bool) -> &'static str {
    if behaviour { "graph" } else { "quest" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_clips() {
        assert!(valid_id("wolf_debt2"));
        assert!(!valid_id("Wolf"));
        assert!(!valid_id(""));
        assert_eq!(id_from_name("NewQuest"), "newquest");
        assert_eq!(id_from_name("a-b c"), "a_b_c");
        assert_eq!(id_from_name("Кв"), "____");
        assert_eq!(clip_utf8("abcdef", 3), "abc...");
        assert_eq!(clip_utf8("абв", 3), "а...");
        assert_eq!(clip_utf8("ab", 3), "ab");
    }

    #[test]
    fn numbers_the_c_way() {
        assert_eq!(c_atoi(" 42x"), 42);
        assert_eq!(c_atoi("-7"), -7);
        assert_eq!(c_atoi("x"), 0);
        assert_eq!(c_atof("0.25"), 0.25);
        assert_eq!(c_atof("1e3z"), 1000.0);
        assert_eq!(c_atof("-.5"), -0.5);
        assert_eq!(c_atof("abc"), 0.0);
    }

    #[test]
    fn summaries() {
        let mut r = Value::table();
        r.set("story", "esc_wolf");
        assert_eq!(ref_summary(&r), "story:esc_wolf");
        let mut sp = Value::table();
        sp.set("section", "simulation_boar");
        sp.set("smart", "esc_3_16");
        sp.set("ref", "boars");
        let mut k = Value::table();
        k.set("spawn", sp);
        assert_eq!(ref_summary(&k), "spawn simulation_boar@esc_3_16 ref=boars");
        assert_eq!(value_summary(&Value::from("x"), 48), "\"x\"");
        let mut t = Value::table();
        t.set("rus", "Привет");
        assert_eq!(value_summary(&t, 48), "\"Привет\"");
        let mut d = Value::table();
        d.set("seconds", 30);
        assert_eq!(value_summary(&d, 48), "30s");
    }

    #[test]
    fn folding_and_cp1251() {
        assert_eq!(fold("СидоровичA"), "сидоровичa");
        assert_eq!(fold("Ёж"), "ёж");
        assert!(contains_no_case("Сидорович", "сид"));
        assert!(cp1251_covers("Привет").0);
        let (ok, bad) = cp1251_covers("a😀b");
        assert!(!ok);
        assert_eq!(bad, "😀");
    }

    #[test]
    fn regraphing() {
        assert_eq!(regraph("Ends the quest. Quests run"), "Ends the graph. Graphs run");
        assert_eq!(regraph("questions stay"), "questions stay");
    }
}
