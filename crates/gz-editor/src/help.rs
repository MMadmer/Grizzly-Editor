//! `help`: the commands as text, three levels deep - the areas, an area's commands, one
//! command's parameters. It is the whole documentation a caller needs to run anything, read
//! off the registry, so it never falls behind the commands.

use std::fmt::Write as _;

use gz_store::CmdError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::editor::Editor;
use crate::registry::{CommandInfo, Registry, Reply};
use crate::spec::Spec;

/// What `help` is asked about.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelpParams {
    /// An area (`scene`), a command (`scene.place`) or a topic (`assets`). Left out: the areas
    /// and the topics.
    pub about: Option<String>,
}

impl Registry {
    /// Registers `help`. Called once, after every area registered its commands.
    pub fn install_help(&mut self) {
        self.cmd_sync(
            Spec::read("help", "The commands and how to use them: help about=<area|command|topic>."),
            |ed: &Editor, p: &HelpParams, _| help(ed, p.about.as_deref().unwrap_or("").trim()),
        );
    }

    fn summary_of(&self, area: &str) -> &'static str {
        self.areas().iter().find(|a| a.name == area).map_or("", |a| a.summary)
    }

    /// The areas that have commands, described ones first, in registration order.
    fn area_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.areas().iter().map(|a| a.name).collect();
        for c in self.commands() {
            if c.spec.is_some() && c.name.contains('.') && !names.contains(&c.area) {
                names.push(c.area);
            }
        }
        names.retain(|n| {
            self.commands().iter().any(|c| c.spec.is_some() && c.area == *n && c.name.contains('.'))
        });
        names
    }

    fn overview(&self) -> String {
        let mut out = format!(
            "Grizzly Editor {} - commands.\nAsk further: help about=<area>, help about=<command>, help about=<topic>.\n",
            env!("CARGO_PKG_VERSION")
        );
        let areas = self.area_names();
        let width = areas.iter().map(|a| a.len()).max().unwrap_or(0);
        out.push_str("\nAreas\n");
        for a in &areas {
            let n = self.commands().iter().filter(|c| c.spec.is_some() && c.area == *a).count();
            let _ = writeln!(out, "  {a:width$}  {} ({n})", self.summary_of(a));
        }
        let top: Vec<&CommandInfo> =
            self.commands().iter().filter(|c| c.spec.is_some() && !c.name.contains('.')).collect();
        if !top.is_empty() {
            out.push_str("\nGeneral\n");
            lines(&mut out, &top);
        }
        if !self.topics().is_empty() {
            let width = self.topics().iter().map(|t| t.name.len()).max().unwrap_or(0);
            out.push_str("\nTopics\n");
            for t in self.topics() {
                let _ = writeln!(out, "  {:width$}  {}", t.name, t.summary);
            }
        }
        let bare: Vec<&str> = self.commands().iter().filter(|c| c.spec.is_none()).map(|c| c.name).collect();
        if !bare.is_empty() {
            let _ = writeln!(out, "\nWithout a card ({}): {}", bare.len(), bare.join(" "));
        }
        out
    }

    fn area_text(&self, area: &str) -> Option<String> {
        let list: Vec<&CommandInfo> = self
            .commands()
            .iter()
            .filter(|c| c.spec.is_some() && c.area == area && c.name.contains('.'))
            .collect();
        if list.is_empty() {
            return None;
        }
        let mut out = String::new();
        let summary = self.summary_of(area);
        if summary.is_empty() {
            let _ = writeln!(out, "{area}");
        } else {
            let _ = writeln!(out, "{area} - {summary}");
        }
        lines(&mut out, &list);
        out.push_str("Parameters of one: help about=<command>.\n");
        Some(out)
    }

    fn command_text(&self, name: &str) -> Option<String> {
        let (spec, shape) = self.card(name)?;
        let mut out = format!("{} ({}) - {}\n", spec.name, spec.effect.word(), spec.summary);
        out.push_str(&shape.lines());
        if !spec.destroys_with.is_empty() {
            let _ = writeln!(out, "Loses data with: {}.", spec.destroys_with.join(", "));
        }
        if !spec.example.is_empty() {
            let _ = writeln!(out, "Example: {} {}", spec.name, spec.example);
        }
        Some(out)
    }
}

/// `  name  effect  summary`, the names padded to one column.
fn lines(out: &mut String, list: &[&CommandInfo]) {
    let width = list.iter().map(|c| c.name.len()).max().unwrap_or(0);
    for c in list {
        if let Some(s) = &c.spec {
            let _ = writeln!(out, "  {:width$}  {:7}  {}", c.name, s.effect.word(), s.summary);
        }
    }
}

fn help(ed: &Editor, about: &str) -> Reply {
    let reg = ed.registry();
    let text = if about.is_empty() {
        reg.overview()
    } else if let Some(t) = reg.command_text(about) {
        t
    } else if let Some(t) = reg.area_text(about) {
        t
    } else if let Some(topic) = reg.topics().iter().find(|t| t.name == about) {
        (topic.text)(ed)
    } else {
        let near: Vec<&str> = reg
            .commands()
            .iter()
            .filter(|c| c.spec.is_some() && c.name.contains(about))
            .map(|c| c.name)
            .take(12)
            .collect();
        let hint = if near.is_empty() {
            String::new()
        } else {
            format!(" Commands with that word: {}.", near.join(", "))
        };
        return Err(CmdError::not_found(format!(
            "help knows no area, command or topic '{about}'.{hint} help without parameters lists the areas."
        )));
    };
    Ok(json!({ "text": text }))
}
