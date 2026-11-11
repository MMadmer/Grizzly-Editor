use gz_base::SmolStr;
use serde::Serialize;

/// Why a command did not happen. `code` is what a caller branches on; `message` is a sentence
/// for the author; `data` rides along in the MCP answer (the dirty documents of `unsaved`).
#[derive(Clone, Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CmdError {
    pub code: SmolStr,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl CmdError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: SmolStr::new(code), message: message.into(), data: None }
    }

    /// The arguments make no sense.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    /// What the command names does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    /// The command would lose unsaved work and was not told to. `dirty` names the documents.
    /// The wire has always answered the bare word `unsaved`: callers branch on it.
    pub fn unsaved(dirty: serde_json::Value) -> Self {
        Self {
            code: SmolStr::new("unsaved"),
            message: "unsaved".to_owned(),
            data: Some(serde_json::json!({ "dirty": dirty })),
        }
    }

    /// Something that must not be busy is busy.
    pub fn busy(message: impl Into<String>) -> Self {
        Self::new("busy", message)
    }

    /// A file or the network failed.
    pub fn io(message: impl Into<String>) -> Self {
        Self::new("io", message)
    }

    /// A bug: the command panicked or met a state it cannot be in.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }

    /// No project is open, and the command needs one.
    pub fn no_project() -> Self {
        Self::new("no_project", "no active project")
    }

    #[must_use]
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl From<std::io::Error> for CmdError {
    fn from(e: std::io::Error) -> Self {
        Self::io(e.to_string())
    }
}

impl From<gz_base::PathError> for CmdError {
    fn from(e: gz_base::PathError) -> Self {
        Self::invalid(e.to_string())
    }
}

impl From<gz_task::Cancelled> for CmdError {
    fn from(_: gz_task::Cancelled) -> Self {
        Self::new("stopped", "Stopped.")
    }
}
