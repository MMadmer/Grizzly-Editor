//! What stands between the project and its build.
//!
//! This crate knows no quests, characters, PDA or screens: each area registers a
//! [`BuildCheck`] here, and the build runs every one of them in parallel over the same walk.
//! A check answers problems; an `Error` refuses the build with the area's text, warnings ride
//! along in the answer. What refuses a build is said here, where the author can still fix it:
//! the assets ship as they are, so a broken one would only be found by the player.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Problem, Severity};
use gz_editor::{Editor, ProjectSession};
use parking_lot::RwLock;

use crate::walk::BuildWalk;

/// One area's check of the module before it is built.
pub trait BuildCheck: Send + Sync {
    /// The Task Center's stage while it runs: `Checking the quests`.
    fn stage(&self) -> &'static str;

    /// The line the refusal starts with when this check stops the build (`The PDA layout has
    /// errors:`), its error lines below it. `None`: the lines alone.
    fn headline(&self) -> Option<&'static str> {
        None
    }

    /// The problems of the module as `walk` says it will ship. Reads what the area's open tabs
    /// read; must not ask anything.
    fn check(&self, editor: Arc<Editor>, session: Arc<ProjectSession>, walk: Arc<BuildWalk>) -> BoxFuture<'static, Vec<Problem>>;
}

/// The scene's bake, run before anything else: a build ships what the author sees. It answers
/// a note - a line per layer, and a `WARNING: ` line per thing left out.
pub type BakeHook = Arc<dyn Fn(Arc<Editor>, Arc<ProjectSession>) -> BoxFuture<'static, String> + Send + Sync>;

/// The checks of every area. Lives in `editor.services`.
#[derive(Default)]
pub struct BuildChecks {
    checks: RwLock<Vec<Arc<dyn BuildCheck>>>,
    bake: RwLock<Option<BakeHook>>,
}

impl BuildChecks {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Adds a check; checks report in the order they were added.
    pub fn add(&self, check: Arc<dyn BuildCheck>) {
        self.checks.write().push(check);
    }

    #[must_use]
    pub fn checks(&self) -> Vec<Arc<dyn BuildCheck>> {
        self.checks.read().clone()
    }

    pub fn set_bake(&self, bake: impl Fn(Arc<Editor>, Arc<ProjectSession>) -> BoxFuture<'static, String> + Send + Sync + 'static) {
        *self.bake.write() = Some(Arc::new(bake));
    }

    #[must_use]
    pub fn bake(&self) -> Option<BakeHook> {
        self.bake.read().clone()
    }
}

/// A problem as a line of the build's answer: a C++ coded problem (`E404`, `W120`) keeps its
/// code in front, as the validators wrote their lines; any other code stays out of the text.
#[must_use]
pub fn problem_line(p: &Problem) -> String {
    let b = p.code.as_bytes();
    let coded = b.len() == 4 && b.first().is_some_and(|c| matches!(c, b'E' | b'W' | b'N')) && b.iter().skip(1).all(u8::is_ascii_digit);
    if coded { format!("{} {}", p.code, p.message) } else { p.message.clone() }
}

/// What one check said, sorted the way the C++ `SortProblems` did.
#[derive(Clone, Debug, Default)]
pub(crate) struct Verdict {
    /// The refusal, empty when the check let the build through.
    pub err: String,
    pub warnings: Vec<String>,
}

impl Verdict {
    /// `first_only`: the manifest's check refused with its first error alone, as the C++ did.
    pub(crate) fn of(problems: &[Problem], headline: Option<&str>, first_only: bool) -> Self {
        let mut errors: Vec<String> = problems.iter().filter(|p| p.severity == Severity::Error).map(problem_line).collect();
        if first_only {
            errors.truncate(1);
        }
        let warnings = problems.iter().filter(|p| p.severity == Severity::Warning).map(problem_line).collect();
        let err = match (errors.is_empty(), headline) {
            (true, _) => String::new(),
            (false, Some(head)) => format!("{head}\n{}", errors.join("\n")),
            (false, None) => errors.join("\n"),
        };
        Self { err, warnings }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_verdicts() {
        let coded = Problem::error("E404", "commands: 'x': no function");
        let plain = Problem::error("mod.id", "module id missing");
        let warn = Problem::warning("W301", "a texture");
        assert_eq!(problem_line(&coded), "E404 commands: 'x': no function");
        assert_eq!(problem_line(&plain), "module id missing");
        let v = Verdict::of(&[coded.clone(), warn.clone(), plain.clone()], Some("The PDA layout has errors:"), false);
        assert_eq!(v.err, "The PDA layout has errors:\nE404 commands: 'x': no function\nmodule id missing");
        assert_eq!(v.warnings, ["W301 a texture"]);
        assert_eq!(Verdict::of(&[plain, coded], None, true).err, "module id missing");
        assert!(Verdict::of(&[warn], None, false).err.is_empty());
    }
}
