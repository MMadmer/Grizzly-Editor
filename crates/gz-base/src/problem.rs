use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::path::RelPath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Stops a build; the game refuses or misreads what it is about.
    Error,
    /// A requirement the work meets by itself later (a scan still running). Never shown as optional.
    Required,
    Warning,
    /// Worth knowing, nothing to fix (a key the editor does not know, kept).
    Note,
}

impl Severity {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Required => "required",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }
}

/// A coded finding of a validator, at the place it is about. The code is what a caller branches
/// on; the text is a sentence for the author. On the wire: `{file, line, column, code,
/// severity, text, fix?}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub code: SmolStr,
    pub severity: Severity,
    /// The sentence for the author.
    #[serde(rename = "text", alias = "message")]
    pub message: String,
    /// What the problem is about inside its document: a node id, a character id, a section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SmolStr>,
    /// The field of the subject, when one field is to blame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<SmolStr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<RelPath>,
    /// 1-based, like the column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// In characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Where the thing ends (exclusive), when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
    /// What to write instead, where the fix is mechanical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Problem {
    pub fn new(code: impl Into<SmolStr>, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity,
            message: message.into(),
            subject: None,
            field: None,
            file: None,
            line: None,
            column: None,
            end_line: None,
            end_column: None,
            fix: None,
        }
    }

    pub fn error(code: impl Into<SmolStr>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, message)
    }

    pub fn warning(code: impl Into<SmolStr>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, message)
    }

    pub fn note(code: impl Into<SmolStr>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Note, message)
    }

    #[must_use]
    pub fn about(mut self, subject: impl Into<SmolStr>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    #[must_use]
    pub fn in_field(mut self, field: impl Into<SmolStr>) -> Self {
        self.field = Some(field.into());
        self
    }

    #[must_use]
    pub fn in_file(mut self, file: RelPath) -> Self {
        self.file = Some(file);
        self
    }

    #[must_use]
    pub fn at_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }

    /// The place: where it starts and where it ends (lines and columns 1-based, the end
    /// exclusive).
    #[must_use]
    pub fn at(mut self, line: u32, column: u32, end_line: u32, end_column: u32) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self.end_line = Some(end_line);
        self.end_column = Some(end_column);
        self
    }

    #[must_use]
    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// `file:line:column: severity code: text`, as a shell prints a problem.
    #[must_use]
    pub fn line_text(&self) -> String {
        let mut s = String::new();
        if let Some(f) = &self.file {
            s.push_str(f.as_str());
            if let Some(l) = self.line {
                s.push_str(&format!(":{l}"));
                if let Some(c) = self.column {
                    s.push_str(&format!(":{c}"));
                }
            }
            s.push_str(": ");
        }
        s.push_str(self.severity.word());
        if !self.code.is_empty() {
            s.push(' ');
            s.push_str(&self.code);
        }
        s.push_str(": ");
        s.push_str(&self.message);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_on_the_wire() {
        let p = Problem::error("E030", "'x' is no character of the project")
            .in_file(RelPath::new("quests/a.nqasset").unwrap())
            .at(4, 7, 4, 12)
            .with_fix("character = \"wolf\"");
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j["text"], "'x' is no character of the project");
        assert_eq!(j["line"], 4);
        assert_eq!(j["column"], 7);
        assert_eq!(j["severity"], "error");
        assert_eq!(j["fix"], "character = \"wolf\"");
        assert!(j.get("message").is_none());
        assert_eq!(p.line_text(), "quests/a.nqasset:4:7: error E030: 'x' is no character of the project");
        let back: Problem = serde_json::from_value(j).unwrap();
        assert_eq!(back, p);
    }
}
