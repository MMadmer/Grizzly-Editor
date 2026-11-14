//! A language's catalog: the translations of the editor's own words, read from the Lua data files
//! the UI crates embed (`ru.lua`).
//!
//! ```lua
//! return {
//!   i18n = 1,
//!   lang = "ru",
//!   { "Save", "Сохранить" },
//!   { "Open", "Открыта", ctx = "submission" },
//!   { "{n} {n:file|files}", "{n} {n:файл|файла|файлов}" },
//! }
//! ```
//!
//! An entry is the English text exactly as the code says it, its translation and, where the same
//! English means something else, a context. A text with placeholders is also a template: a
//! sentence a domain crate wrote in English ("The file a.ltx cannot be read: ...") that matches it
//! is shown translated, its values kept.

use std::sync::Arc;

use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use super::Lang;
use super::template::{self, Seg, Val};

/// Why a catalog file is not one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogError {
    /// The file among the sources, from 0.
    pub source: usize,
    /// The entry of that file, from 1; 0 for the file itself.
    pub entry: usize,
    pub message: String,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.entry == 0 {
            write!(f, "catalog {}: {}", self.source, self.message)
        } else {
            write!(f, "catalog {}, entry {}: {}", self.source, self.entry, self.message)
        }
    }
}

/// One entry as the files hold it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub en: String,
    pub tr: String,
    pub ctx: Option<String>,
    /// The file among the sources and the entry's place in it, from 1.
    pub source: usize,
    pub at: usize,
}

/// The translations of one English text: without a context, and by context.
#[derive(Clone, Debug, Default)]
struct Variants {
    plain: Option<Arc<str>>,
    by_ctx: Vec<(Box<str>, Arc<str>)>,
}

/// A template entry, parsed once.
struct Template {
    en: Box<str>,
    tr: Arc<str>,
    ctx: Option<Box<str>>,
}

/// The longest messages the memo keeps before it starts again.
const MEMO_KEEP: usize = 4096;

/// The translations of one language.
pub struct Catalog {
    lang: Lang,
    words: FxHashMap<Box<str>, Variants>,
    templates: Vec<Template>,
    /// Templates by the first word of their text; `loose` ones start with a placeholder.
    by_word: FxHashMap<Box<str>, Vec<u32>>,
    loose: Vec<u32>,
    /// Sentences matched against the templates, and what they came to.
    memo: Mutex<FxHashMap<Box<str>, Option<Arc<str>>>>,
    entries: Vec<Entry>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog").field("lang", &self.lang).field("entries", &self.entries.len()).finish()
    }
}

/// The letters a template's own words hold (its text less its placeholders) are enough to tell a
/// sentence it fits from any other: `{label} {why}` would fit every sentence of two words, so it
/// translates only what `tr!` says with it (or what its context is in force for).
fn worded(en: &str) -> bool {
    let letters: usize = template::parse(en)
        .iter()
        .map(|s| match s {
            Seg::Lit(t) => t.chars().filter(|c| c.is_alphabetic()).count(),
            // a counted word is a word: its shortest form
            Seg::Plural(_, forms) => {
                forms.iter().map(|f| f.chars().filter(|c| c.is_alphabetic()).count()).min().unwrap_or(0)
            }
            Seg::Var(_) => 0,
        })
        .sum();
    letters >= 4
}

/// The text before a template's first placeholder up to its first space: what a sentence must
/// start with to be worth matching.
fn first_word(en: &str) -> Option<String> {
    let segs = template::parse(en);
    let Some(Seg::Lit(first)) = segs.first() else { return None };
    // a literal that runs into a placeholder before any space is no whole word
    let (word, _) = first.split_once(' ')?;
    (!word.is_empty()).then(|| word.to_owned())
}

impl Catalog {
    /// Reads the catalog files of one language. Every file names the same `lang`.
    pub fn parse(sources: &[&str]) -> Result<Catalog, CatalogError> {
        let mut lang = None;
        let mut entries = Vec::new();
        for (si, src) in sources.iter().enumerate() {
            let err = |entry: usize, message: String| CatalogError { source: si, entry, message };
            let value = gz_luadata::parse_named(src, "i18n")
                .map_err(|e| err(0, format!("line {}, column {}: {}", e.line, e.column, e.message)))?;
            if value.get("i18n").and_then(gz_luadata::Value::as_number) != Some(1.0) {
                return Err(err(0, "the header `i18n = 1` is missing".to_owned()));
            }
            let named = value.get("lang").and_then(gz_luadata::Value::as_str).and_then(Lang::parse);
            let Some(named) = named else {
                return Err(err(0, "`lang` names no language the editor has".to_owned()));
            };
            if *lang.get_or_insert(named) != named {
                return Err(err(0, "every file of one catalog names the same `lang`".to_owned()));
            }
            for (i, e) in value.arr().iter().enumerate() {
                let at = i + 1;
                let pair = e.arr();
                let (Some(en), Some(tr)) =
                    (pair.first().and_then(|v| v.as_str()), pair.get(1).and_then(|v| v.as_str()))
                else {
                    return Err(err(at, "an entry is { \"English\", \"translation\" }".to_owned()));
                };
                if pair.len() > 2 {
                    return Err(err(at, format!("'{en}': an entry holds two texts")));
                }
                let ctx = match e.get("ctx") {
                    None => None,
                    Some(c) => match c.as_str() {
                        Some(c) if !c.is_empty() => Some(c.to_owned()),
                        _ => return Err(err(at, format!("'{en}': `ctx` is a name"))),
                    },
                };
                for (k, _) in e.fields() {
                    if k != "ctx" {
                        return Err(err(at, format!("'{en}': `{k}` is no key of an entry")));
                    }
                }
                entries.push(Entry { en: en.to_owned(), tr: tr.to_owned(), ctx, source: si, at });
            }
        }
        let lang = lang.unwrap_or(Lang::En);
        Ok(Self::from_entries(lang, entries))
    }

    fn from_entries(lang: Lang, entries: Vec<Entry>) -> Catalog {
        let mut words: FxHashMap<Box<str>, Variants> = FxHashMap::default();
        let mut templates = Vec::new();
        let mut by_word: FxHashMap<Box<str>, Vec<u32>> = FxHashMap::default();
        let mut loose = Vec::new();
        for e in &entries {
            let tr: Arc<str> = Arc::from(e.tr.as_str());
            let v = words.entry(Box::from(e.en.as_str())).or_default();
            match &e.ctx {
                None => v.plain = Some(tr.clone()),
                Some(c) => v.by_ctx.push((Box::from(c.as_str()), tr.clone())),
            }
            if template::has_placeholder(&e.en) && (e.ctx.is_some() || worded(&e.en)) {
                let i = u32::try_from(templates.len()).unwrap_or(u32::MAX);
                match first_word(&e.en) {
                    Some(w) => by_word.entry(w.into_boxed_str()).or_default().push(i),
                    None => loose.push(i),
                }
                templates.push(Template {
                    en: Box::from(e.en.as_str()),
                    tr,
                    ctx: e.ctx.as_deref().map(Box::from),
                });
            }
        }
        Catalog { lang, words, templates, by_word, loose, memo: Mutex::default(), entries }
    }

    #[must_use]
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Every entry as the files hold it, in their order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether an English text has its own entry, in any context.
    #[must_use]
    pub fn knows(&self, en: &str) -> bool {
        self.words.contains_key(en)
    }

    /// The translation of a text: the entry of the context given, else of a context in force that
    /// has one, else the one without a context.
    pub(crate) fn word(
        &self,
        en: &str,
        explicit: Option<&str>,
        in_force: impl Fn(&str) -> bool,
    ) -> Option<Arc<str>> {
        let v = self.words.get(en)?;
        if let Some(c) = explicit
            && let Some((_, t)) = v.by_ctx.iter().find(|(k, _)| &**k == c)
        {
            return Some(t.clone());
        }
        if !v.by_ctx.is_empty() {
            // the contexts in force: a scope, the surface being drawn
            if let Some((_, t)) = v.by_ctx.iter().find(|(k, _)| in_force(k)) {
                return Some(t.clone());
            }
        }
        v.plain.clone()
    }

    /// A text's translation away from the window (a scene worked out on a pool): the entry of
    /// `ctx`, else the plain one, else a template it matches; `None` when the catalog lacks it.
    #[must_use]
    pub fn lookup(&self, en: &str, ctx: Option<&str>) -> Option<Arc<str>> {
        self.word(en, ctx, |_| false).or_else(|| self.sentence(en, |c| ctx == Some(c)))
    }

    /// A template: its translation, by context as [`Catalog::word`].
    pub(crate) fn template(
        &self,
        en: &str,
        explicit: Option<&str>,
        in_force: impl Fn(&str) -> bool,
    ) -> Option<Arc<str>> {
        self.word(en, explicit, in_force)
    }

    /// A sentence a domain crate wrote, translated by the template it matches; `None` when none
    /// does. A context-bound template matches only where its context is in force.
    pub(crate) fn sentence(&self, text: &str, in_force: impl Fn(&str) -> bool) -> Option<Arc<str>> {
        if text.is_empty() {
            return None;
        }
        if let Some(hit) = self.memo.lock().get(text) {
            return hit.clone();
        }
        let word = text.split(' ').next().unwrap_or("");
        let candidates = self.by_word.get(word).into_iter().flatten().chain(self.loose.iter());
        let mut found = None;
        for &i in candidates {
            let Some(t) = self.templates.get(i as usize) else { continue };
            if t.ctx.as_deref().is_some_and(|c| !in_force(c)) {
                continue;
            }
            let segs = template::parse(&t.en);
            if let Some(caps) = template::matches(&segs, text) {
                let tr = template::parse(&t.tr);
                let filled = template::fill(&tr, self.lang, |name| {
                    caps.iter().find(|(n, _)| *n == name).map(|(_, v)| Val::Text(v))
                });
                found = Some(Arc::from(filled.as_str()));
                break;
            }
        }
        let mut memo = self.memo.lock();
        if memo.len() >= MEMO_KEEP {
            memo.clear();
        }
        memo.insert(Box::from(text), found.clone());
        found
    }
}

/// What is wrong with the entries of a catalog: the same English twice in one context, an empty
/// text, a translation whose placeholders are not the English one's, a plural with the wrong
/// number of forms for its language, spaces at the ends that the English does not have. One line
/// per problem; empty when the catalog is sound.
#[must_use]
pub fn problems(catalog: &Catalog) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: FxHashMap<(&str, Option<&str>), (usize, usize)> = FxHashMap::default();
    let forms = |lang: Lang| if lang == Lang::Ru { 3 } else { 2 };
    for e in catalog.entries() {
        let at = format!("catalog {} entry {} '{}'", e.source, e.at, e.en);
        if let Some((s, a)) = seen.insert((e.en.as_str(), e.ctx.as_deref()), (e.source, e.at)) {
            out.push(format!("{at}: the same English as catalog {s} entry {a}"));
        }
        if e.en.trim().is_empty() || e.tr.trim().is_empty() {
            out.push(format!("{at}: an empty text"));
        }
        if e.tr.starts_with(' ') != e.en.starts_with(' ') || e.tr.ends_with(' ') != e.en.ends_with(' ') {
            out.push(format!("{at}: spaces at the ends differ"));
        }
        let en = template::parse(&e.en);
        let tr = template::parse(&e.tr);
        let mut en_names = template::names(&en);
        let mut tr_names = template::names(&tr);
        en_names.sort_unstable();
        tr_names.sort_unstable();
        if en_names != tr_names {
            out.push(format!("{at}: placeholders {en_names:?} in English, {tr_names:?} in the translation"));
        }
        for s in &en {
            if let Seg::Plural(n, f) = s
                && f.len() != 2
            {
                out.push(format!(
                    "{at}: the English plural of {n} takes 2 forms (one|other), not {}",
                    f.len()
                ));
            }
        }
        for s in &tr {
            if let Seg::Plural(n, f) = s
                && f.len() != forms(catalog.lang())
            {
                out.push(format!(
                    "{at}: the plural of {n} takes {} forms in this language, not {}",
                    forms(catalog.lang()),
                    f.len()
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Catalog, problems};
    use crate::i18n::Lang;

    const SAMPLE: &str = r#"return {
  i18n = 1,
  lang = "ru",
  { "Save", "Сохранить" },
  { "Open", "Открыть" },
  { "Open", "Открыта", ctx = "submission" },
  { "{n} {n:file|files}", "{n} {n:файл|файла|файлов}" },
  { "The file {path} cannot be read: {reason}", "Не удалось прочитать файл {path}: {reason}" },
}"#;

    #[test]
    fn words_by_context_and_sentences_by_template() {
        let c = Catalog::parse(&[SAMPLE]).expect("parses");
        assert_eq!(c.lang(), Lang::Ru);
        assert_eq!(c.len(), 5);
        assert!(problems(&c).is_empty(), "{:?}", problems(&c));
        assert_eq!(c.word("Save", None, |_| false).as_deref(), Some("Сохранить"));
        assert_eq!(c.word("Open", None, |_| false).as_deref(), Some("Открыть"));
        assert_eq!(c.word("Open", None, |k| k == "submission").as_deref(), Some("Открыта"));
        assert_eq!(c.word("Open", Some("submission"), |_| false).as_deref(), Some("Открыта"));
        assert_eq!(c.word("Close", None, |_| false), None);
        assert_eq!(
            c.sentence("The file a/b.ltx cannot be read: it is locked", |_| false).as_deref(),
            Some("Не удалось прочитать файл a/b.ltx: it is locked")
        );
        assert_eq!(c.sentence("7 files", |_| false).as_deref(), Some("7 файлов"));
        assert_eq!(c.sentence("Nothing like it", |_| false), None);
        // a template of placeholders alone fits every sentence: it translates only what tr! says
        let loose = Catalog::parse(&[r#"return { i18n = 1, lang = "ru", { "{a} {b}", "{a}: {b}" } }"#])
            .expect("parses");
        assert_eq!(loose.sentence("Done in 00:04", |_| false), None);
        // asked again, the memo answers
        assert_eq!(c.sentence("7 files", |_| false).as_deref(), Some("7 файлов"));
    }

    #[test]
    fn a_broken_entry_is_named() {
        let dup = SAMPLE.replace("{ \"Open\", \"Открыть\" },", "{ \"Save\", \"Сохранение\" },");
        let c = Catalog::parse(&[&dup]).expect("parses");
        assert_eq!(problems(&c).len(), 1, "{:?}", problems(&c));
        let bad = SAMPLE.replace("{n:файл|файла|файлов}", "{n:файл|файлы}");
        assert_eq!(problems(&Catalog::parse(&[&bad]).expect("parses")).len(), 1);
        let missing = SAMPLE.replace("Не удалось прочитать файл {path}", "Не удалось прочитать файл");
        assert_eq!(problems(&Catalog::parse(&[&missing]).expect("parses")).len(), 1);
        assert!(Catalog::parse(&["return { lang = \"ru\" }"]).is_err(), "the header");
        assert!(Catalog::parse(&["return { i18n = 1, lang = \"ru\", { \"a\" } }"]).is_err());
    }
}
