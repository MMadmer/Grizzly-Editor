use serde::{Deserialize, Serialize};

/// Who asked for a command. It decides who answers a question the command raises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The author, through the window: a question opens a dialog.
    Ui,
    /// A caller over MCP: the caller decided. A confirmation is Yes, and losing data takes the
    /// explicit `*_discard` command.
    Mcp,
    /// `-nodlg`: the logged answers (No / OK).
    Unattended,
}

impl Origin {
    /// Whether a dialog may open for this origin.
    #[must_use]
    pub fn may_ask(self) -> bool {
        matches!(self, Origin::Ui)
    }
}
