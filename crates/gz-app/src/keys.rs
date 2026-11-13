//! Keys: a chord and how it reads ("Ctrl+Shift+P"). What a chord does is an action's
//! (`actions.rs`); the order in which takers see it is the shell's (`shell.rs`, `keys`).

use egui::{Event, Key, Modifiers};

/// A key with its modifiers, as a shortcut binds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    #[must_use]
    pub const fn new(key: Key) -> Self {
        Self { key, ctrl: false, shift: false, alt: false }
    }

    #[must_use]
    pub const fn ctrl(key: Key) -> Self {
        Self { key, ctrl: true, shift: false, alt: false }
    }

    #[must_use]
    pub const fn ctrl_shift(key: Key) -> Self {
        Self { key, ctrl: true, shift: true, alt: false }
    }

    #[must_use]
    pub const fn shift(key: Key) -> Self {
        Self { key, ctrl: false, shift: true, alt: false }
    }

    #[must_use]
    pub const fn alt(key: Key) -> Self {
        Self { key, ctrl: false, shift: false, alt: true }
    }

    /// The chord a key event is, with the modifiers it came with.
    #[must_use]
    pub fn of(key: Key, modifiers: Modifiers) -> Self {
        Self { key, ctrl: modifiers.ctrl || modifiers.command, shift: modifiers.shift, alt: modifiers.alt }
    }

    /// Reads "Ctrl+Shift+S", "F5", "Del", "Num Del", "Ctrl+`". `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut chord = Chord::new(Key::A);
        let mut key = None;
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // a trailing "+" is the key itself ("Ctrl++")
        let (body, plus_key) = match text.strip_suffix("++") {
            Some(rest) => (rest, true),
            None => (text, false),
        };
        for part in body.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" => chord.ctrl = true,
                "shift" => chord.shift = true,
                "alt" => chord.alt = true,
                "" => return None,
                _ if key.is_some() => return None,
                _ => key = Some(key_from_label(part)?),
            }
        }
        if plus_key {
            if key.is_some() {
                return None;
            }
            key = Some(Key::Plus);
        }
        chord.key = key?;
        Some(chord)
    }

    /// How a menu and the Keyboard page show it.
    #[must_use]
    pub fn label(&self) -> String {
        let mut out = String::with_capacity(16);
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        out.push_str(key_label(self.key));
        out
    }
}

/// The name a key is shown by.
#[must_use]
pub fn key_label(key: Key) -> &'static str {
    match key {
        Key::Delete => "Del",
        Key::Escape => "Esc",
        Key::Insert => "Ins",
        Key::PageUp => "PgUp",
        Key::PageDown => "PgDn",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::Backtick => "`",
        Key::Space => "Space",
        Key::Plus => "+",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        other => other.name(),
    }
}

fn key_from_label(part: &str) -> Option<Key> {
    let lower = part.to_ascii_lowercase();
    let stripped = lower.strip_prefix("num ").unwrap_or(&lower);
    match stripped {
        "del" | "delete" => Some(Key::Delete),
        "esc" | "escape" => Some(Key::Escape),
        "ins" | "insert" => Some(Key::Insert),
        "pgup" | "page up" | "pageup" => Some(Key::PageUp),
        "pgdn" | "page down" | "pagedown" => Some(Key::PageDown),
        "space" => Some(Key::Space),
        "enter" | "return" => Some(Key::Enter),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "up" => Some(Key::ArrowUp),
        "down" => Some(Key::ArrowDown),
        "left" => Some(Key::ArrowLeft),
        "right" => Some(Key::ArrowRight),
        "~" | "tilde" => Some(Key::Backtick),
        _ => {
            if stripped.len() == 1 {
                Key::from_name(&stripped.to_ascii_uppercase())
            } else if let Some(n) = stripped.strip_prefix('f') {
                n.parse::<u8>()
                    .ok()
                    .filter(|n| (1..=35).contains(n))
                    .and_then(|_| Key::from_name(&stripped.to_ascii_uppercase()))
            } else {
                Key::from_name(part)
            }
        }
    }
}

/// The key presses of this frame that nobody took yet, as chords, in order.
#[must_use]
pub fn pressed(events: &[Event]) -> Vec<Chord> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                Some(Chord::of(*key, *modifiers))
            }
            _ => None,
        })
        .collect()
}

/// Takes the press of `chord` out of this frame's input: the next taker does not see it.
pub fn consume(input: &mut egui::InputState, chord: &Chord) -> bool {
    let at = input.events.iter().position(|e| matches!(e, Event::Key { key, pressed: true, repeat: false, modifiers, .. } if Chord::of(*key, *modifiers) == *chord));
    match at {
        Some(i) => {
            input.events.remove(i);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use egui::Key;

    use super::Chord;

    #[test]
    fn labels_read_back() {
        for text in ["Ctrl+Shift+P", "F11", "Del", "Ctrl+`", "Ctrl+Space", "Alt+F4", "Ctrl+Z"] {
            let chord = Chord::parse(text);
            assert!(chord.is_some(), "{text}");
            assert_eq!(chord.map(|c| c.label()).as_deref(), Some(text));
        }
    }

    #[test]
    fn spellings() {
        assert_eq!(Chord::parse("ctrl+shift+s"), Some(Chord::ctrl_shift(Key::S)));
        assert_eq!(Chord::parse("Num Del"), Some(Chord::new(Key::Delete)));
        assert_eq!(Chord::parse("F5"), Some(Chord::new(Key::F5)));
        assert_eq!(Chord::parse("Ctrl++"), Some(Chord::ctrl(Key::Plus)));
        assert_eq!(Chord::parse(""), None);
        assert_eq!(Chord::parse("Ctrl+"), None);
        assert_eq!(Chord::parse("Ctrl+A+B"), None);
        assert_eq!(Chord::parse("Banana"), None);
    }
}
