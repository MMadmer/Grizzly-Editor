//! What a radio is told to play: the one rule the scene's radios, their bake and the quests'
//! `radio_playlist` share (the game's `xms_radio.script`, `valid_choice`; `RADIO.md`).

/// The game's music alone.
pub const GAME: &str = "game";
/// Silence.
pub const OFF: &str = "off";

/// A binding as written, read against the module it is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding<'a> {
    Game,
    Off,
    /// A playlist of this module: its id (written alone, or after this module's id).
    Own(&'a str),
    /// A playlist of another module, taken as written.
    Other {
        module: &'a str,
        id: &'a str,
    },
}

impl Binding<'_> {
    /// What the game reads in `[xms_radio] playlist`: `<module>.<id>`, `game` or `off`.
    #[must_use]
    pub fn key(&self, module: &str) -> String {
        match self {
            Self::Game => GAME.to_owned(),
            Self::Off => OFF.to_owned(),
            Self::Own(id) => format!("{module}.{id}"),
            Self::Other { module, id } => format!("{module}.{id}"),
        }
    }
}

/// Reads a binding: `game`, `off`, `<module>.<id>` or an id alone for a playlist of `module`.
/// An error says why the text is no binding; the game would play the player's station.
pub fn binding<'a>(text: &'a str, module: &str) -> Result<Binding<'a>, String> {
    let t = text.trim();
    if t == GAME {
        return Ok(Binding::Game);
    }
    if t == OFF {
        return Ok(Binding::Off);
    }
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return Err(format!(
            "'{text}' is no playlist: <module>.<playlist>, a playlist of this module by its id, game or off"
        ));
    }
    match t.split_once('.') {
        None => Ok(Binding::Own(t)),
        Some((m, id)) if !m.is_empty() && !id.is_empty() => {
            if !module.is_empty() && m.eq_ignore_ascii_case(module) {
                Ok(Binding::Own(id))
            } else {
                Ok(Binding::Other { module: m, id })
            }
        }
        _ => Err(format!("'{text}' is no playlist: <module>.<playlist> names both")),
    }
}

/// Whether the module declares a playlist of this id (`[playlist.<id>]`, read without case as the
/// game reads section names).
#[must_use]
pub fn declares(playlists: &[crate::RadioPlaylist], id: &str) -> bool {
    playlists.iter().any(|p| p.id.eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binding_reads_as_the_game_reads_it() {
        assert_eq!(binding("game", "m"), Ok(Binding::Game));
        assert_eq!(binding(" off ", "m"), Ok(Binding::Off));
        assert_eq!(binding("road", "m"), Ok(Binding::Own("road")));
        assert_eq!(binding("m.road", "m"), Ok(Binding::Own("road")));
        assert_eq!(binding("other.road", "m"), Ok(Binding::Other { module: "other", id: "road" }));
        assert!(binding("not a playlist", "m").is_err());
        assert!(binding(".road", "m").is_err());
        assert!(binding("", "m").is_err());
        assert_eq!(Binding::Own("road").key("m"), "m.road");
        assert_eq!(Binding::Off.key("m"), "off");
    }
}
