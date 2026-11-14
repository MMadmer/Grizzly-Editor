use serde::{Deserialize, Serialize};

/// Who asked for a command. It decides who answers a question the command raises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The author, through the window: a question opens a dialog.
    Ui,
    /// A command's caller: the caller decided, so nothing is asked. Losing data takes the
    /// parameter that says so (`discard`, `overwrite`), through the destroy door.
    Mcp,
    /// The window's own action under `-nodlg`: the logged answers (No for a question, OK for
    /// a note).
    Unattended,
}

impl Origin {
    /// Whether a dialog may open for this origin.
    #[must_use]
    pub fn may_ask(self) -> bool {
        matches!(self, Origin::Ui)
    }
}
