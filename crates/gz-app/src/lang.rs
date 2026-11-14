//! The window's language (docs/UI_DESIGN.md, section 9): Preferences > General > Language, `-lang`,
//! and the catalogs the UI crates embed.
//!
//! The kit, the shell and every area hand their catalog (`ru.lua`, Lua data) to the builder; the
//! shell reads them on the CPU pool the first time the window is to speak that language - a start
//! in Russian while the window and its device come up, never a start in English - and installs
//! the language in the egui context. The choice is read every frame from
//! the settings, so Preferences and `app.prefs` change it at once; `-lang` holds a language for
//! the run until the setting is changed.

use std::sync::Arc;
use std::time::Instant;

use egui::Context;
use gz_ui::Lang;
use gz_ui::i18n::{self, Catalog};
use rustc_hash::FxHashMap;

use crate::settings::EditorSettings;

/// The shell's own words in Russian.
pub const SHELL_RU: &str = include_str!("ru.lua");

/// What the shell knows of the window's language.
pub(crate) struct Language {
    /// Every catalog file, Russian ones today: the kit's, the shell's, the areas'.
    sources: Vec<&'static str>,
    catalogs: FxHashMap<Lang, Arc<Catalog>>,
    /// The language Windows speaks to the user, as an ISO 639-1 code.
    system: &'static str,
    /// `-lang`: the language of this run until the setting changes.
    held: Option<Lang>,
    /// The setting's revision `held` was given at.
    seen_rev: u64,
    /// What the window shows now; `None` before the first frame.
    applied: Option<Lang>,
    /// The catalogs of a language being read on the CPU pool.
    reading: Option<(Lang, std::sync::mpsc::Receiver<Option<Arc<Catalog>>>)>,
    /// Languages whose catalogs did not read: the window stays in English for them.
    failed: Vec<Lang>,
}

impl Language {
    pub(crate) fn new(mut sources: Vec<&'static str>, held: Option<Lang>) -> Self {
        sources.insert(0, SHELL_RU);
        sources.insert(0, i18n::KIT_RU);
        Self {
            sources,
            catalogs: FxHashMap::default(),
            system: gz_win::user_ui_language(),
            held,
            seen_rev: u64::MAX,
            applied: None,
            reading: None,
            failed: Vec::new(),
        }
    }

    /// The language the window is to show now: `-lang` until the setting changes, else the
    /// setting (the system's language for `system`).
    pub(crate) fn wanted(&mut self, settings: &EditorSettings) -> Lang {
        let rev = settings.language_rev();
        if self.seen_rev == u64::MAX {
            self.seen_rev = rev;
        } else if rev != self.seen_rev {
            // the author or a caller picked a language: it wins over the command line
            self.seen_rev = rev;
            self.held = None;
        }
        self.held.unwrap_or_else(|| settings.language().resolve(self.system))
    }

    /// Starts reading the catalogs of a language on the CPU pool, once: a start in Russian reads
    /// them while the window and its device come up, a change of language while the next frame is
    /// drawn. The UI thread never parses them.
    pub(crate) fn prefetch(&mut self, lang: Lang, editor: &gz_editor::Editor) {
        if lang == Lang::En
            || self.catalogs.contains_key(&lang)
            || self.reading.as_ref().is_some_and(|(l, _)| *l == lang)
        {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let sources = self.sources.clone();
        let log = editor.log.clone();
        editor.exec.spawn_cpu(move || {
            let started = Instant::now();
            let read = Catalog::parse(&sources);
            let read = match read {
                Ok(c) if c.lang() == lang => {
                    // a line for the record, not a report the status bar would show
                    log.plain(format!(
                        "the {} catalog: {} entries read in {:.1} ms",
                        lang.key(),
                        c.len(),
                        started.elapsed().as_secs_f64() * 1000.0
                    ));
                    Some(Arc::new(c))
                }
                Ok(_) => {
                    log.warn(format!(
                        "the editor has no catalog of {}: the window stays in English",
                        lang.key()
                    ));
                    None
                }
                Err(e) => {
                    log.error(format!("the translation does not read ({e}): the window stays in English"));
                    None
                }
            };
            let _ = tx.send(read);
        });
        self.reading = Some((lang, rx));
    }

    /// Installs the language wanted when it is not the one shown. Every frame, before anything is
    /// drawn: while its catalogs are being read the window keeps the language it shows.
    pub(crate) fn apply(&mut self, ctx: &Context, settings: &EditorSettings, editor: &gz_editor::Editor) {
        let wanted = self.wanted(settings);
        if self.applied == Some(wanted) {
            return;
        }
        let catalog = if wanted == Lang::En {
            None
        } else {
            if let Some((lang, rx)) = &self.reading {
                match rx.try_recv() {
                    Ok(read) => {
                        if let Some(c) = read {
                            self.catalogs.insert(*lang, c);
                        } else {
                            self.failed.push(*lang);
                        }
                        self.reading = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => self.reading = None,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
            }
            match self.catalogs.get(&wanted) {
                Some(c) => Some(c.clone()),
                None if self.failed.contains(&wanted) => None,
                None => {
                    self.prefetch(wanted, editor);
                    ctx.request_repaint_after(std::time::Duration::from_millis(5));
                    return;
                }
            }
        };
        self.applied = Some(wanted);
        i18n::install(ctx, catalog);
    }
}

/// The name Preferences gives the system's choice: "System (Русский)".
#[must_use]
pub fn system_choice_name() -> gz_ui::Tx<'static> {
    let lang = crate::settings::system_lang(gz_win::user_ui_language());
    gz_ui::tr!("System ({lang})", lang = lang.native_name())
}

#[cfg(test)]
mod tests {
    use gz_ui::i18n::{Catalog, problems};

    #[test]
    fn the_shells_catalog_reads_with_the_kits() {
        let c = Catalog::parse(&[gz_ui::i18n::KIT_RU, super::SHELL_RU]).expect("the catalogs read");
        let p = problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
    }
}
