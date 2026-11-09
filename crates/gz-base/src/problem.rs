use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::path::RelPath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Stops a build.
    Error,
    /// A requirement the work meets by itself later (a scan still running). Never shown as optional.
    Required,
    Warning,
    Note,
}

/// A coded finding of a validator. The code is for MCP and the build; the message is a sentence
/// for the author.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub code: SmolStr,
    pub severity: Severity,
    pub message: String,
    /// What the problem is about inside its document: a node id, a character id, a section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SmolStr>,
    /// The field of the subject, when one field is to blame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<SmolStr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<RelPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl Problem {
    pub fn new(code: impl Into<SmolStr>, severity: Severity, message: impl Into<String>) -> Self {
        Self { code: code.into(), severity, message: message.into(), subject: None, field: None, file: None, line: None }
    }

    pub fn error(code: impl Into<SmolStr>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, message)
    }

    pub fn warning(code: impl Into<SmolStr>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, message)
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

    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
