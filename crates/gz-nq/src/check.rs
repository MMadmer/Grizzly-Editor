//! The build gate of the graphs: every graph the build ships, as it is on disk, validated in
//! parallel; then `E113`, a behaviour graph that runs itself through a chain of others, which no
//! single document can see. The build asks it through a [`gz_build::BuildCheck`]; `asset.check`
//! gives the same problems per file, positioned.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::Problem;
use gz_build::{BuildCheck, BuildWalk};
use gz_editor::{Editor, ProjectSession};

use crate::service::NqService;
use crate::validate::NqProblem;

/// What the check found: one line per problem (`<path>: <code> <node>: <message>`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Report {
    pub log: Vec<String>,
    pub errors: usize,
    pub warnings: usize,
}

impl Report {
    /// The refusal text, empty when the graphs pass.
    pub fn error(&self) -> String {
        if self.errors > 0 { format!("quest graph errors: {} (see log)", self.errors) } else { String::new() }
    }
}

/// One graph checked: its path, its problems, why it could not be read, and - for a behaviour
/// graph - its id and the graphs it runs.
type Checked = (String, Vec<NqProblem>, Option<String>, Option<(String, Vec<String>)>);

/// Runs the check over what is on disk; open documents with unsaved edits are noted, not
/// checked in their place.
pub(crate) async fn run(editor: &Editor, svc: &NqService) -> Report {
    svc.refresh_tree().await;
    let env = svc.env(editor, false).await;
    let entries = env.index.entries.clone();
    let dirty: Vec<String> = svc.docs().iter().filter(|d| d.is_dirty()).map(|d| d.path().key).collect();
    let env2 = env.clone();
    let checked: Vec<Checked> = editor
        .exec
        .cpu({
            let exec = editor.exec.clone();
            move || {
                exec.par_cpu(entries, |e| {
                    if !e.usable() {
                        return (e.rel.clone(), Vec::new(), Some(e.error.clone()), None);
                    }
                    let problems = env2.validate(&e.graph, &e.rel);
                    // what E113 looks through, taken as each behaviour graph is read
                    let calls = e.graph.is_behaviour().then(|| {
                        let out: Vec<String> = e
                            .graph
                            .nodes
                            .iter()
                            .filter(|n| n.kind == "behaviour.run")
                            .map(|n| n.params.get_string("asset", ""))
                            .filter(|a| !a.is_empty() && !a.contains('.'))
                            .collect();
                        (e.graph.id.clone(), out)
                    });
                    (e.rel.clone(), problems, None, calls)
                })
            }
        })
        .await;
    let mut report = Report::default();
    let mut ids: Vec<(String, String, Vec<String>)> = Vec::new();
    for ((rel, problems, unreadable, calls), entry) in checked.into_iter().zip(&env.index.entries) {
        if dirty.contains(&entry.key) {
            report
                .log
                .push(format!("{rel}: ~ has unsaved changes in the editor - the file on disk is what ships"));
        }
        if let Some(err) = unreadable {
            report.errors += 1;
            report.log.push(format!("{rel}: E003 {err}"));
            continue;
        }
        for p in &problems {
            if p.is_error() {
                report.errors += 1;
            } else {
                report.warnings += 1;
            }
            report.log.push(format!("{rel}: {}", p.text()));
        }
        if let Some((id, out)) = calls {
            ids.push((rel, id, out));
        }
    }
    // E113: can a graph be reached again from what it calls? The direct case is the single
    // document's check; a chain of two or more lands here
    for (s, (rel, id, calls)) in ids.iter().enumerate() {
        let mut seen = vec![false; ids.len()];
        let mut stack: Vec<usize> = Vec::new();
        for c in calls {
            for (k, other) in ids.iter().enumerate() {
                if other.1 == *c && k != s && !seen[k] {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
        let mut loops = false;
        while let Some(at) = stack.pop() {
            let Some(here) = ids.get(at) else { continue };
            for c in &here.2 {
                if c == id {
                    loops = true;
                    break;
                }
                for (k, other) in ids.iter().enumerate() {
                    if other.1 == *c && !seen[k] {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
            if loops {
                break;
            }
        }
        if loops {
            report.errors += 1;
            report
                .log
                .push(format!("{rel}: E113 behaviour graph '{id}' runs itself through the graphs it calls"));
        }
    }
    for line in &report.log {
        if line.contains(": E") {
            svc.log().error(format!("[nq] {line}"));
        } else {
            svc.log().warn(format!("[nq] {line}"));
        }
    }
    report
}

/// What the build says while the graphs' check runs.
pub(crate) const STAGE: &str = "Checking the quests";

/// The graphs' check of the build.
pub(crate) struct GraphCheck;

impl BuildCheck for GraphCheck {
    fn stage(&self) -> &'static str {
        STAGE
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        _session: Arc<ProjectSession>,
        _walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            // the graphs the index lists are the graphs of the build: it skips what the build leaves out
            let Ok(svc) = NqService::of(&editor) else { return Vec::new() };
            let report = run(&editor, &svc).await;
            let mut out = Vec::new();
            if report.errors > 0 {
                out.push(Problem::error("nq", report.error()));
            }
            if report.warnings > 0 {
                out.push(Problem::warning("nq", format!("quest warnings: {}", report.warnings)));
                for line in report.log.iter().filter(|l| !l.contains(": E")) {
                    out.push(Problem::warning("nq", format!("  {line}")));
                }
            }
            out
        })
    }
}
