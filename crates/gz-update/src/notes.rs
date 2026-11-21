//! The release notes as the update dialog shows them (docs/UPDATES.md section 1, "The notes").
//!
//! The notes are a release's body, written by a person in Markdown, and nothing in them is
//! required: the reader takes what is there and never shows less because a line is not where a
//! format would want it. Of a body that speaks several languages it keeps the window's: a
//! language starts at any line that only names one (a heading of any level, a bold line, `EN:`,
//! `English`, `Русский`, `RU` ...) and runs to the next such line. A body that names none is one
//! language. What says how to install the release is left out - the updater installs it - and is
//! a section whose heading names installing; when leaving it out would leave nothing, it stays.
//! The window's language missing, English is taken, then the first language there is.

/// The languages a body may name, with the words a line naming one may be.
const LANGUAGES: &[(&str, &[&str])] = &[
    ("en", &["en", "eng", "english", "английский", "англ"]),
    ("ru", &["ru", "rus", "russian", "русский", "рус", "на русском"]),
];

/// The first words of a heading about installing.
const INSTALL: &[&str] = &["install", "setup", "установ", "инсталл"];

/// What a line says once its Markdown is set aside: the heading marks, the emphasis, a trailing
/// colon, flags and brackets around it. Lower case.
fn bare(line: &str) -> String {
    let t = line.trim().trim_start_matches('#').trim();
    let t: String =
        t.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '[' | ']' | '(' | ')' | '|')).collect();
    // a flag of a country is two regional indicators: never a word
    let t: String = t.chars().filter(|c| !('\u{1F1E6}'..='\u{1F1FF}').contains(c)).collect();
    t.trim().trim_end_matches([':', '.', '-', '—']).trim().to_lowercase()
}

/// The language a line names, when that is all it says.
fn language_of(line: &str) -> Option<&'static str> {
    let t = bare(line);
    if t.is_empty() || t.chars().count() > 24 {
        return None;
    }
    LANGUAGES.iter().find(|(_, words)| words.contains(&t.as_str())).map(|(code, _)| *code)
}

/// The level of a heading line (`#` .. `######`), or none.
fn heading_level(line: &str) -> Option<usize> {
    let t = line.trim_start();
    let n = t.chars().take_while(|&c| c == '#').count();
    (n > 0 && n <= 6 && t[n..].starts_with([' ', '\t'])).then_some(n)
}

/// A line that only separates (`---`, `***`, `___`).
fn rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3
        && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

/// The body without HTML comments: a note to the next author is no note to the reader.
fn without_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        match rest[at..].find("-->") {
            Some(end) => rest = &rest[at + end + 3..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The sections of installing left out of `lines`, unless that leaves nothing to read.
fn without_install(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut skipping: Option<usize> = None;
    for line in lines {
        if let Some(level) = heading_level(line) {
            if skipping.is_some_and(|s| level <= s) {
                skipping = None;
            }
            if skipping.is_none() && INSTALL.iter().any(|w| bare(line).starts_with(w)) {
                skipping = Some(level);
                continue;
            }
        }
        if skipping.is_none() {
            out.push((*line).to_owned());
        }
    }
    let readable =
        |v: &[String]| v.iter().any(|l| !l.trim().is_empty() && heading_level(l).is_none() && !rule(l));
    if readable(&out) { out } else { lines.iter().map(|l| (*l).to_owned()).collect() }
}

/// What the dialog shows of `body` in the language `lang` (`en`, `ru`): see the module's words.
#[must_use]
pub fn for_language(body: &str, lang: &str) -> String {
    let body = without_comments(&body.replace("\r\n", "\n").replace('\r', "\n"));
    let lines: Vec<&str> = body.lines().collect();
    // the languages and where each starts
    let mut parts: Vec<(&'static str, usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(code) = language_of(line) {
            if let Some(last) = parts.last_mut() {
                last.2 = i;
            }
            parts.push((code, i + 1, lines.len()));
        }
    }
    let chosen: Vec<&str> = if parts.is_empty() {
        lines.clone()
    } else {
        let pick = parts
            .iter()
            .find(|p| p.0 == lang)
            .or_else(|| parts.iter().find(|p| p.0 == "en"))
            .or_else(|| parts.first())
            .copied();
        match pick {
            Some((_, from, to)) => lines[from..to].to_vec(),
            None => lines.clone(),
        }
    };
    let kept = without_install(&chosen);
    // a rule or blank lines at either end separate nothing
    let mut kept: Vec<String> = kept;
    while kept.first().is_some_and(|l| l.trim().is_empty() || rule(l)) {
        kept.remove(0);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty() || rule(l)) {
        kept.pop();
    }
    kept.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "## EN\n\n## Changes\n\n* Fixed a crash.\n\n## Installation\n\nChoose one.\n---\n## RU\n\n## Изменения\n\n* Исправлен вылет.\n\n## Установка\n\nВыберите один.\n";

    #[test]
    fn the_windows_language_is_kept_and_the_installation_left_out() {
        assert_eq!(for_language(TWO, "ru"), "## Изменения\n\n* Исправлен вылет.");
        assert_eq!(for_language(TWO, "en"), "## Changes\n\n* Fixed a crash.");
    }

    #[test]
    fn any_way_of_naming_a_language_starts_it() {
        for (en, ru) in [
            ("# English", "# Русский"),
            ("**EN:**", "**RU:**"),
            ("English:", "Russian:"),
            ("### 🇬🇧 English", "### 🇷🇺 Русский"),
            ("[EN]", "[RU]"),
        ] {
            let body = format!("{en}\n- one\n\n{ru}\n- один\n");
            assert_eq!(for_language(&body, "ru"), "- один", "{ru}");
            assert_eq!(for_language(&body, "en"), "- one", "{en}");
        }
    }

    #[test]
    fn a_body_of_one_language_or_none_is_shown_whole() {
        let plain = "Fixed a crash.\n\n- faster start\n";
        assert_eq!(for_language(plain, "ru"), "Fixed a crash.\n\n- faster start");
        // only Russian there: a Russian window and an English one read it alike
        let ru = "## Русский\n- Быстрее старт\n";
        assert_eq!(for_language(ru, "en"), "- Быстрее старт");
    }

    #[test]
    fn nothing_is_lost_to_a_heading_out_of_place() {
        // no "Changes" heading, a theme before the list, deeper headings, a comment for authors
        let body = "<!-- release 0.8 -->\n# English\nA big one.\n### Quests\n- graphs\n#### Fixes\n- a crash\n# Русский\nБольшой.\n### Квесты\n- графы\n";
        assert_eq!(for_language(body, "en"), "A big one.\n### Quests\n- graphs\n#### Fixes\n- a crash");
        // a section only of installing stays when it is all there is
        assert_eq!(for_language("## Installation\nUnpack it.", "en"), "## Installation\nUnpack it.");
        // an installation heading deeper than the language's own goes, the next equal heading comes back
        let deep = "# EN\n## Installation\nsteps\n## Notes\n- kept\n";
        assert_eq!(for_language(deep, "en"), "## Notes\n- kept");
    }

    #[test]
    fn a_word_that_only_starts_with_a_language_is_no_language() {
        let body = "Rus is a short word.\n- English menus now";
        assert_eq!(for_language(body, "ru"), body);
    }
}
