//! The window's language (docs/UI_DESIGN.md, section 9).
//!
//! English is the language of the code: every text the editor says is written in English where it
//! is said, and a catalog of the language the author picked translates it where it is drawn. What
//! the author or the game owns - names, files, game strings, what the author typed - is data and
//! is shown as it is. The kit's widgets take a [`Tx`], and the type makes the caller say which a
//! text is:
//!
//! - a string literal is the editor's own words: `Button::new("Save")` is looked up;
//! - [`tr!`](crate::tr) is a sentence with values: `tr!("Delete {n} {n:file|files}", n = count)`
//!   picks the word by the number in the language shown, then fills the values;
//! - [`Tx::raw`] is data, shown as it is; [`Tx::ui`] is the editor's own English that is not a
//!   literal (a label an area registered, a sentence a domain crate wrote: it is shown translated
//!   when the catalog knows it or one of its templates matches it).
//!
//! A `String` is none of them and does not become a [`Tx`] by itself: a sentence put together
//! with `format!` would reach the Russian window in English unnoticed.
//!
//! The same English can mean two things ("Open" a verb, "Open" a state): an entry may name a
//! context, and a surface says which contexts are in force with [`scope`] - the dialog or panel it
//! draws is one too (`dlg.<name>`, the part of its window name after `###`).
//!
//! The language and its catalog live in the egui context ([`install`]); English needs no
//! catalog. In an audited frame every text the catalog lacks is reported (`ui.audit`'s
//! `untranslated`).

mod catalog;
pub(crate) mod template;

use std::borrow::Cow;
use std::sync::Arc;

use egui::{Context, Id, Response};
use gz_base::SmolStr;
use parking_lot::Mutex;

pub use catalog::{Catalog, CatalogError, Entry, problems};

/// The kit's own words in Russian, with the glossary every other catalog keeps to.
pub const KIT_RU: &str = include_str!("ru.lua");

/// A language of the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

impl Lang {
    /// Every language, in the order Preferences lists them.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ru];

    /// The key of the settings file and of `-lang`: `en`, `ru`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    /// The language's name in itself: what a person looks for in a list of languages.
    #[must_use]
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }

    /// A language by its key or its name (in English or in itself), any case.
    #[must_use]
    pub fn parse(text: &str) -> Option<Lang> {
        let t = text.trim();
        Lang::ALL.into_iter().find(|l| {
            t.eq_ignore_ascii_case(l.key())
                || t.eq_ignore_ascii_case(match l {
                    Lang::En => "english",
                    Lang::Ru => "russian",
                })
                || t.to_lowercase() == l.native_name().to_lowercase()
        })
    }

    /// The form a counted word takes for `n`: English one / other, Russian one / few / many
    /// (CLDR; whole numbers).
    #[must_use]
    pub fn plural(self, n: i64) -> usize {
        let n = n.unsigned_abs();
        match self {
            Lang::En => usize::from(n != 1),
            Lang::Ru => {
                let (d, h) = (n % 10, n % 100);
                if d == 1 && h != 11 {
                    0
                } else if (2..=4).contains(&d) && !(12..=14).contains(&h) {
                    1
                } else {
                    2
                }
            }
        }
    }
}

/// A value [`tr!`](crate::tr) fills a sentence with.
#[derive(Clone, Debug)]
pub enum Arg {
    /// A count: the words that follow it agree with it.
    Num(i64),
    /// Data, shown as it is.
    Text(String),
    /// A word of the editor's own, shown in the window's language.
    Tx(Tx<'static>),
}

/// What a value of [`tr!`](crate::tr) can be: a whole number (a count), a text (data), a [`Tx`] (a
/// word of the editor's own).
pub trait IntoArg {
    fn into_arg(self) -> Arg;
}

macro_rules! num_arg {
    ($($t:ty),*) => {$(
        impl IntoArg for $t {
            fn into_arg(self) -> Arg {
                Arg::Num(i64::try_from(self).unwrap_or(i64::MAX))
            }
        }
        impl IntoArg for &$t {
            fn into_arg(self) -> Arg {
                Arg::Num(i64::try_from(*self).unwrap_or(i64::MAX))
            }
        }
    )*};
}
num_arg!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl IntoArg for &str {
    fn into_arg(self) -> Arg {
        Arg::Text(self.to_owned())
    }
}
impl IntoArg for String {
    fn into_arg(self) -> Arg {
        Arg::Text(self)
    }
}
impl IntoArg for &String {
    fn into_arg(self) -> Arg {
        Arg::Text(self.clone())
    }
}
impl IntoArg for SmolStr {
    fn into_arg(self) -> Arg {
        Arg::Text(self.to_string())
    }
}
impl IntoArg for &SmolStr {
    fn into_arg(self) -> Arg {
        Arg::Text(self.to_string())
    }
}
impl IntoArg for Cow<'_, str> {
    fn into_arg(self) -> Arg {
        Arg::Text(self.into_owned())
    }
}
impl IntoArg for &Cow<'_, str> {
    fn into_arg(self) -> Arg {
        Arg::Text(self.to_string())
    }
}
impl IntoArg for Tx<'_> {
    fn into_arg(self) -> Arg {
        Arg::Tx(self.into_owned())
    }
}

/// A sentence of [`tr!`](crate::tr): its English template, the context it belongs to and its
/// values.
#[derive(Clone, Debug)]
pub struct Fmt {
    ctx: Option<&'static str>,
    en: &'static str,
    args: Vec<(&'static str, Arg)>,
}

#[derive(Clone, Debug)]
enum Repr<'a> {
    /// The editor's own words in English, looked up.
    Ui {
        ctx: Option<&'static str>,
        en: Cow<'a, str>,
    },
    /// Data, shown as it is.
    Raw(Cow<'a, str>),
    Fmt(Box<Fmt>),
}

/// A text a widget shows: the editor's own words (looked up in the window's language), a sentence
/// with values ([`tr!`](crate::tr)), or data ([`Tx::raw`]).
#[derive(Clone, Debug)]
pub struct Tx<'a>(Repr<'a>);

impl Default for Tx<'_> {
    fn default() -> Self {
        Tx::EMPTY
    }
}

/// Two texts are the same when they are of one kind and read the same in English.
impl PartialEq for Tx<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.is_raw() == other.is_raw() && self.en() == other.en()
    }
}

impl From<&'static str> for Tx<'_> {
    fn from(en: &'static str) -> Self {
        Tx(Repr::Ui { ctx: None, en: Cow::Borrowed(en) })
    }
}

impl<'a> From<&'a Tx<'_>> for Tx<'a> {
    fn from(t: &'a Tx<'_>) -> Self {
        t.borrow()
    }
}

impl<'a> Tx<'a> {
    /// No text.
    pub const EMPTY: Tx<'static> = Tx(Repr::Raw(Cow::Borrowed("")));

    /// Data - a name, a path, a number, a game string, what the author typed - shown as it is.
    pub fn raw(text: impl Into<Cow<'a, str>>) -> Self {
        Tx(Repr::Raw(text.into()))
    }

    /// The editor's own English that is no literal here: a label an area registered, a sentence a
    /// domain crate wrote. Shown translated when the catalog knows it or a template matches it.
    pub fn ui(en: impl Into<Cow<'a, str>>) -> Self {
        Tx(Repr::Ui { ctx: None, en: en.into() })
    }

    /// The editor's own word in a context of its own: "Open" as a state, not a verb.
    #[must_use]
    pub fn ctx(ctx: &'static str, en: &'static str) -> Self {
        Tx(Repr::Ui { ctx: Some(ctx), en: Cow::Borrowed(en) })
    }

    /// [`Tx::ui`] in a context of its own: a caption menu's name is looked up in `menu` first.
    pub fn ui_in(ctx: &'static str, en: impl Into<Cow<'a, str>>) -> Self {
        Tx(Repr::Ui { ctx: Some(ctx), en: en.into() })
    }

    /// A sentence with values; what [`tr!`](crate::tr) makes.
    #[must_use]
    pub fn fmt(ctx: Option<&'static str>, en: &'static str, args: Vec<(&'static str, Arg)>) -> Tx<'static> {
        Tx(Repr::Fmt(Box::new(Fmt { ctx, en, args })))
    }

    /// The same text, borrowed.
    #[must_use]
    pub fn borrow(&self) -> Tx<'_> {
        Tx(match &self.0 {
            Repr::Ui { ctx, en } => Repr::Ui { ctx: *ctx, en: Cow::Borrowed(en) },
            Repr::Raw(t) => Repr::Raw(Cow::Borrowed(t)),
            Repr::Fmt(f) => Repr::Fmt(f.clone()),
        })
    }

    /// The same text, owning what it borrowed.
    #[must_use]
    pub fn into_owned(self) -> Tx<'static> {
        Tx(match self.0 {
            Repr::Ui { ctx, en } => Repr::Ui { ctx, en: Cow::Owned(en.into_owned()) },
            Repr::Raw(t) => Repr::Raw(Cow::Owned(t.into_owned())),
            Repr::Fmt(f) => Repr::Fmt(f),
        })
    }

    /// Whether there is nothing to show.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match &self.0 {
            Repr::Ui { en, .. } => en.is_empty(),
            Repr::Raw(t) => t.is_empty(),
            Repr::Fmt(f) => f.en.is_empty(),
        }
    }

    /// Whether the text is data, shown as it is.
    #[must_use]
    pub fn is_raw(&self) -> bool {
        matches!(self.0, Repr::Raw(_))
    }

    /// The text in English: what the log and a command's caller read.
    #[must_use]
    pub fn en(&self) -> Cow<'_, str> {
        match &self.0 {
            Repr::Ui { en, .. } => Cow::Borrowed(en),
            Repr::Raw(t) => Cow::Borrowed(t),
            Repr::Fmt(f) => Cow::Owned(format_en(f)),
        }
    }

    /// The text in the window's language.
    #[must_use]
    pub fn get(&self, ctx: &Context) -> Cow<'_, str> {
        match &self.0 {
            Repr::Raw(t) => Cow::Borrowed(t),
            Repr::Ui { ctx: cx, en } => {
                if en.is_empty() {
                    return Cow::Borrowed(en);
                }
                let Some(state) = state(ctx) else { return Cow::Borrowed(en) };
                let Some(cat) = state.catalog.as_ref() else { return Cow::Borrowed(en) };
                let found = cat
                    .word(en, *cx, |c| state.in_force(ctx, c))
                    .or_else(|| cat.sentence(en, |c| state.in_force(ctx, c)));
                match found {
                    Some(t) => Cow::Owned(t.to_string()),
                    None => {
                        crate::audit::untranslated(ctx, en);
                        Cow::Borrowed(en)
                    }
                }
            }
            Repr::Fmt(f) => Cow::Owned(format_in(ctx, f)),
        }
    }

    /// The text in the window's language, owned.
    #[must_use]
    pub fn text(&self, ctx: &Context) -> String {
        self.get(ctx).into_owned()
    }
}

/// The values of a sentence as text in a language: a word of the editor's own translated.
fn values<'f>(f: &'f Fmt, ctx: Option<&Context>) -> Vec<(&'static str, Cow<'f, str>, Option<i64>)> {
    f.args
        .iter()
        .map(|(name, a)| match a {
            Arg::Num(n) => (*name, Cow::Owned(n.to_string()), Some(*n)),
            Arg::Text(t) => (*name, Cow::Borrowed(t.as_str()), None),
            Arg::Tx(t) => (*name, ctx.map_or_else(|| t.en(), |c| t.get(c)), None),
        })
        .collect()
}

fn fill(template: &str, lang: Lang, vals: &[(&'static str, Cow<'_, str>, Option<i64>)]) -> String {
    let segs = template::parse(template);
    template::fill(&segs, lang, |name| {
        vals.iter().find(|(n, ..)| *n == name).map(|(_, text, num)| match num {
            Some(n) => template::Val::Num(*n),
            None => template::Val::Text(text),
        })
    })
}

fn format_en(f: &Fmt) -> String {
    fill(f.en, Lang::En, &values(f, None))
}

fn format_in(ctx: &Context, f: &Fmt) -> String {
    let vals = values(f, Some(ctx));
    let Some(state) = state(ctx) else { return fill(f.en, Lang::En, &vals) };
    let Some(cat) = state.catalog.as_ref() else { return fill(f.en, Lang::En, &vals) };
    match cat.template(f.en, f.ctx, |c| state.in_force(ctx, c)) {
        Some(t) => fill(&t, cat.lang(), &vals),
        None => {
            crate::audit::untranslated(ctx, f.en);
            fill(f.en, Lang::En, &vals)
        }
    }
}

/// A sentence with values, in the window's language where it is drawn and in English for the
/// log: `tr!("Delete {n} {n:file|files}", n = count)`, `tr!("submission" => "Open")`.
///
/// `{name}` takes a value; `{name:one|other}` picks the English word by the count `name` holds
/// (the catalog's Russian names one|few|many). A value is a whole number, a text (data) or a
/// [`Tx`](crate::i18n::Tx) (a word of the editor's own, translated).
#[macro_export]
macro_rules! tr {
    ($ctx:literal => $en:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::Tx::fmt(
            Some($ctx),
            $en,
            vec![$((stringify!($name), $crate::i18n::IntoArg::into_arg($value))),*],
        )
    };
    ($en:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::Tx::fmt(None, $en, vec![$((stringify!($name), $crate::i18n::IntoArg::into_arg($value))),*])
    };
}

/// The language of a context and what it is drawing.
struct State {
    catalog: Option<Arc<Catalog>>,
    /// The contexts in force, innermost last.
    scopes: Mutex<Vec<SmolStr>>,
}

impl State {
    /// Whether a context is in force: a scope of [`scope`], or the surface being drawn (its
    /// window name, or the part after `###`).
    fn in_force(&self, ctx: &Context, name: &str) -> bool {
        if self.scopes.lock().iter().rev().any(|s| s == name) {
            return true;
        }
        crate::audit::in_surface(ctx, name)
    }
}

#[derive(Clone)]
struct Handle(Arc<State>);

fn id() -> Id {
    Id::unique("gz-ui.i18n")
}

fn state(ctx: &Context) -> Option<Arc<State>> {
    ctx.data(|d| d.get_temp::<Handle>(id())).map(|h| h.0)
}

/// Shows the window in a language: `None` (or an English catalog) is English, the code's own.
pub fn install(ctx: &Context, catalog: Option<Arc<Catalog>>) {
    let catalog = catalog.filter(|c| c.lang() != Lang::En);
    let handle = Handle(Arc::new(State { catalog, scopes: Mutex::default() }));
    ctx.data_mut(|d| d.insert_temp(id(), handle));
    ctx.request_repaint();
}

/// The language the window is shown in.
#[must_use]
pub fn lang(ctx: &Context) -> Lang {
    state(ctx).and_then(|s| s.catalog.as_ref().map(|c| c.lang())).unwrap_or_default()
}

/// The catalog in force, if the window is not in English.
#[must_use]
pub fn catalog(ctx: &Context) -> Option<Arc<Catalog>> {
    state(ctx).and_then(|s| s.catalog.clone())
}

/// Puts a context in force for what `body` draws: entries of that context win over the plain one.
/// An action's title is looked up in its group too (`"Back"` of the Viewport group is a view).
pub fn scope<R>(ctx: &Context, name: &str, body: impl FnOnce() -> R) -> R {
    let Some(s) = state(ctx) else { return body() };
    s.scopes.lock().push(SmolStr::new(name));
    let out = body();
    s.scopes.lock().pop();
    out
}

/// The editor's own word in the window's language: `t(ctx, "Save")`.
#[must_use]
pub fn t(ctx: &Context, en: &'static str) -> String {
    Tx::from(en).text(ctx)
}

/// Whether a text matches a search in the language shown or in English: what a list of the
/// editor's own words (the Command Palette, the Keyboard page) filters by.
#[must_use]
pub fn matches_either(ctx: &Context, query: &str, tx: &Tx<'_>) -> bool {
    crate::search::matches(query, &tx.get(ctx)) || crate::search::matches(query, &tx.en())
}

/// The tooltip of a response, in the window's language.
pub trait TipExt: Sized {
    /// A hover hint; nothing for an empty text.
    fn tip<'a>(self, text: impl Into<Tx<'a>>) -> Self;
    /// A hover hint on a disabled widget: why it is off.
    fn tip_disabled<'a>(self, text: impl Into<Tx<'a>>) -> Self;
}

impl TipExt for Response {
    fn tip<'a>(self, text: impl Into<Tx<'a>>) -> Self {
        let text = text.into();
        if text.is_empty() {
            return self;
        }
        // an audited frame reads every tip, so that one the catalog lacks is reported unhovered
        if crate::audit::wanted(&self.ctx) {
            let _ = text.get(&self.ctx);
        }
        self.on_hover_ui(|ui| {
            ui.add(egui::Label::new(text.get(ui.ctx()).as_ref()));
        })
    }

    fn tip_disabled<'a>(self, text: impl Into<Tx<'a>>) -> Self {
        let text = text.into();
        if text.is_empty() {
            return self;
        }
        if crate::audit::wanted(&self.ctx) {
            let _ = text.get(&self.ctx);
        }
        self.on_disabled_hover_ui(|ui| {
            ui.add(egui::Label::new(text.get(ui.ctx()).as_ref()));
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Catalog, Lang, Tx, install, problems, scope};

    #[test]
    fn languages_by_key_and_name() {
        assert_eq!(Lang::parse("ru"), Some(Lang::Ru));
        assert_eq!(Lang::parse("Russian"), Some(Lang::Ru));
        assert_eq!(Lang::parse("Русский"), Some(Lang::Ru));
        assert_eq!(Lang::parse("EN"), Some(Lang::En));
        assert_eq!(Lang::parse("de"), None);
        assert_eq!(Lang::Ru.plural(1), 0);
        assert_eq!(Lang::Ru.plural(22), 1);
        assert_eq!(Lang::Ru.plural(12), 2);
        assert_eq!(Lang::En.plural(0), 1);
    }

    #[test]
    fn the_kit_catalog_is_sound() {
        let c = Catalog::parse(&[super::KIT_RU]).expect("the kit's catalog parses");
        assert_eq!(c.lang(), Lang::Ru);
        let p = problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
    }

    #[test]
    fn a_text_shows_in_the_language_installed() {
        let ctx = egui::Context::default();
        let save = Tx::from("Save");
        let n = |k: usize| crate::tr!("{n} {n:file|files}", n = k);
        let name = Tx::raw("Save");
        // English: as the code says it, no catalog needed
        assert_eq!(save.get(&ctx), "Save");
        assert_eq!(n(1).get(&ctx), "1 file");
        assert_eq!(n(4).get(&ctx), "4 files");
        let src = r#"return { i18n = 1, lang = "ru",
            { "Save", "Сохранить" },
            { "Open", "Открыть" },
            { "Open", "Открыта", ctx = "submission" },
            { "{n} {n:file|files}", "{n} {n:файл|файла|файлов}" },
        }"#;
        install(&ctx, Some(Arc::new(Catalog::parse(&[src]).expect("parses"))));
        assert_eq!(save.get(&ctx), "Сохранить");
        assert_eq!(n(1).get(&ctx), "1 файл");
        assert_eq!(n(3).get(&ctx), "3 файла");
        assert_eq!(n(25).get(&ctx), "25 файлов");
        // the log reads English whatever the window shows
        assert_eq!(n(3).en(), "3 files");
        // data is never translated
        assert_eq!(name.get(&ctx), "Save");
        // a context in force picks its entry
        let open = Tx::from("Open");
        assert_eq!(open.get(&ctx), "Открыть");
        assert_eq!(scope(&ctx, "submission", || open.get(&ctx).into_owned()), "Открыта");
        assert_eq!(Tx::ctx("submission", "Open").get(&ctx), "Открыта");
        // a sentence from a domain crate the catalog has no word for stays English
        assert_eq!(Tx::ui("Built 3 steps").get(&ctx), "Built 3 steps");
        assert_eq!(Tx::ui("7 files").get(&ctx), "7 файлов");
        install(&ctx, None);
        assert_eq!(save.get(&ctx), "Save");
    }
}
