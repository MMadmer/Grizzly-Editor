//! The commands of the manifest store: edits of the document.

use gz_store::CmdError;

use crate::doc::{ManifestDoc, entry_line, key_width};
use crate::exclude::set_excluded;
use crate::model::{Command, Manifest};

/// One edit of `mod.ltx`. A batch of them is applied in order and either all or none land.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Sets a key: its line keeps its spacing and comment, a new line follows its neighbours.
    Set { section: String, key: String, value: String },
    /// Removes every line of a key; a section left with nothing but blank lines goes too.
    Remove { section: String, key: String },
    /// Replaces the entry lines of a section by these raw lines (comments stay); empty clears
    /// it, and a section left with nothing but blank lines goes.
    SetLines { section: String, lines: Vec<String> },
    RemoveSection { section: String },
    /// The browser's Exclude from Build / Include in Build of one project-relative entry.
    SetExcluded { rel: String, excluded: bool },
    /// Adds a console command or replaces the first of its name (any case): its `[commands]`
    /// line and its `[command.<name>]` section.
    SetCommand(Command),
    /// Takes out every command of this name (any case): its lines and its section.
    RemoveCommand(String),
}

impl Edit {
    #[must_use]
    pub fn set(section: &str, key: &str, value: impl Into<String>) -> Self {
        Edit::Set { section: section.to_owned(), key: key.to_owned(), value: value.into() }
    }

    #[must_use]
    pub fn remove(section: &str, key: &str) -> Self {
        Edit::Remove { section: section.to_owned(), key: key.to_owned() }
    }

    #[must_use]
    pub fn lines(section: &str, lines: Vec<String>) -> Self {
        Edit::SetLines { section: section.to_owned(), lines }
    }

    /// `Set` for a non-empty value, `Remove` for an empty one: how `Save` treats an optional key.
    #[must_use]
    pub fn optional(section: &str, key: &str, value: &str) -> Self {
        if value.is_empty() { Edit::remove(section, key) } else { Edit::set(section, key, value) }
    }
}

/// The `[commands]` line of a command, as `Save` writes it.
#[must_use]
pub fn command_line(c: &Command) -> String {
    entry_line(&c.name, &c.function, key_width("commands"))
}

/// Applies one edit. `Err` leaves the caller's copy half edited: the store drops it.
pub fn apply(doc: &mut ManifestDoc, edit: Edit) -> Result<(), CmdError> {
    match edit {
        Edit::Set { section, key, value } => doc.set(&section, &key, &value),
        Edit::Remove { section, key } => {
            doc.remove(&section, &key);
            doc.prune(&section);
        }
        Edit::SetLines { section, lines } => {
            doc.set_lines(&section, &lines);
            doc.prune(&section);
        }
        Edit::RemoveSection { section } => {
            doc.remove_section(&section);
        }
        Edit::SetExcluded { rel, excluded } => {
            let m = Manifest::parse(doc);
            if let Some(next) = set_excluded(&m.exclude, &rel, excluded).map_err(CmdError::invalid)? {
                doc.set_lines("exclude", &next);
                doc.prune("exclude");
            }
        }
        Edit::SetCommand(c) => {
            let line = command_line(&c);
            if !doc.set_line_ci("commands", &c.name, &line) {
                doc.push_line("commands", &line);
            }
            let section = format!("command.{}", c.name);
            // a name declared twice has one section: the first entry's
            doc.remove_section(&section);
            let details = c.section_lines();
            if !details.is_empty() {
                doc.set_lines(&section, &details);
            }
        }
        Edit::RemoveCommand(name) => {
            doc.remove_where("commands", |k| k.eq_ignore_ascii_case(&name));
            doc.prune("commands");
            doc.remove_section(&format!("command.{name}"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(text: &str, edits: Vec<Edit>) -> String {
        let mut doc = ManifestDoc::parse(text);
        for e in edits {
            apply(&mut doc, e).unwrap();
        }
        doc.to_string()
    }

    #[test]
    fn optional_keys_come_and_go() {
        let base = "[module]\r\nid          = od ; mine\r\n\r\n[update]\r\ngithub      = a/b\r\n";
        assert_eq!(run(base, vec![Edit::optional("update", "github", "")]), "[module]\r\nid          = od ; mine\r\n");
        assert_eq!(
            run(base, vec![Edit::optional("module", "author", "Madmer")]),
            "[module]\r\nid          = od ; mine\r\nauthor      = Madmer\r\n\r\n[update]\r\ngithub      = a/b\r\n"
        );
    }

    #[test]
    fn a_batch_keeps_the_authors_file() {
        let base = "; Overnight Delivery - the contest mod\r\n[module]\r\nid          = od\r\nname        = Overnight ; working title\r\nversion     = 1.0.0\r\nengine_min  = 1.4\r\n\r\n; my own notes\r\n[budget]\r\nram = 2\r\n\r\n[order]\r\nafter       = a\r\n";
        let edits = vec![
            Edit::set("module", "version", "1.1.0"),
            Edit::set("module", "name", crate::text::display_value("Overnight; Delivery", false)),
            Edit::optional("order", "after", ""),
            Edit::lines("vars", vec![entry_line("kills", "0", 15)]),
        ];
        let out = run(base, edits);
        assert_eq!(
            out,
            "; Overnight Delivery - the contest mod\r\n[module]\r\nid          = od\r\nname        = \"Overnight; Delivery\" ; working title\r\nversion     = 1.1.0\r\nengine_min  = 1.4\r\n\r\n; my own notes\r\n[budget]\r\nram = 2\r\n\r\n[vars]\r\nkills           = 0\r\n"
        );
        let m = Manifest::parse(&ManifestDoc::parse(&out));
        assert_eq!(m.name, "Overnight; Delivery");
        assert!(m.after.is_empty());
        assert_eq!(m.vars.len(), 1);
    }

    #[test]
    fn exclusions_are_lines() {
        let base = "[module]\nid = od\n\n[exclude]\n; out\ndocs\n";
        assert_eq!(run(base, vec![Edit::SetExcluded { rel: "design\\".into(), excluded: true }]), "[module]\nid = od\n\n[exclude]\n; out\ndocs\ndesign\n");
        let mut doc = ManifestDoc::parse(base);
        let refused = apply(&mut doc, Edit::SetExcluded { rel: "mod.ltx".into(), excluded: true });
        assert!(refused.is_err());
    }

    #[test]
    fn commands_keep_their_sections() {
        let base = "[commands]\nstatus          = od.status\n\n[command.status]\nhelp        = old\n";
        let c = Command { name: "status".into(), function: "od.show".into(), args: "<n>".into(), cheat: true, ..Command::default() };
        let out = run(base, vec![Edit::SetCommand(c)]);
        assert_eq!(out, "[commands]\nstatus          = od.show\n\n[command.status]\nargs        = <n>\ncheat       = true\n");
        let m = Manifest::parse(&ManifestDoc::parse(&out));
        assert_eq!(m.commands.len(), 1);
        assert!(m.commands[0].cheat);
        assert_eq!(run(&out, vec![Edit::RemoveCommand("STATUS".into())]), "");
        let added = run("[module]\nid = od\n", vec![Edit::SetCommand(Command { name: "night".into(), function: "od.night".into(), ..Command::default() })]);
        assert_eq!(added, "[module]\nid = od\n\n[commands]\nnight           = od.night\n");
    }
}
