use serde_json::{Map, Value};

/// The request a command is called with, as the wire carries it: the parameters by name, without
/// the command's own name. A command with a card never sees it - the registry reads it into the
/// command's parameter type first - so it is only what a caller builds to run a command by name.
pub type Args = Map<String, Value>;
