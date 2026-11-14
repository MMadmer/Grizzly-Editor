//! Where in `mod.ltx` a problem is about: a section, a key of it, the n-th line of that key, or a
//! key inside one layer of a screen.
//!
//! The areas that read their own sections (`gz-screens`, `gz-world`) say what a problem is
//! about with a [`Spot`] carried by the problem's `subject` and `field`; the manifest's check
//! asks the document where that is ([`ManifestDoc::locate`]) and puts the problem at the key's
//! line, a problem about a whole section at its header. No area looks into the file for it.

use gz_base::Problem;

use crate::doc::{Item, ManifestDoc};

/// A place of the manifest: `[section]`, a key of it, or a key inside the n-th `scope` line (a
/// screen's layer: the lines from one `layer` to the next).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spot {
    /// The section as the file spells it, read without case.
    pub section: String,
    /// The key; none: the section's header (or, with a scope, the scope's own line).
    pub key: Option<String>,
    /// Which line of the key, 0 the first one.
    pub nth: usize,
    /// A key that starts a group of lines (`layer`) and which of them.
    pub scope: Option<(String, usize)>,
}

impl Spot {
    /// The section's header.
    #[must_use]
    pub fn section(section: &str) -> Self {
        Self { section: section.to_owned(), ..Self::default() }
    }

    /// The first line of `key` in the section.
    #[must_use]
    pub fn key(section: &str, key: &str) -> Self {
        Self { section: section.to_owned(), key: Some(key.to_owned()), ..Self::default() }
    }

    /// The `n`-th line (0 based) of `key` in the section.
    #[must_use]
    pub fn nth(section: &str, key: &str, n: usize) -> Self {
        Self { nth: n, ..Self::key(section, key) }
    }

    /// The line of the `n`-th `scope` key (a layer's own line), or with `key` the first line of
    /// that key between it and the next `scope` line.
    #[must_use]
    pub fn within(section: &str, scope: &str, n: usize, key: Option<&str>) -> Self {
        Self {
            section: section.to_owned(),
            key: key.map(str::to_owned),
            nth: 0,
            scope: Some((scope.to_owned(), n)),
        }
    }

    /// The problem about this place: `subject` is `[section]`, `field` the key's address
    /// (`key`, `key#n`, `scope#n/key`, `scope#n/` for the scope's own line).
    #[must_use]
    pub fn mark(&self, p: Problem) -> Problem {
        let p = p.about(format!("[{}]", self.section));
        let field = match (&self.scope, &self.key) {
            (Some((scope, n)), key) => format!("{scope}#{n}/{}", key.as_deref().unwrap_or("")),
            (None, Some(key)) if self.nth > 0 => format!("{key}#{}", self.nth),
            (None, Some(key)) => key.clone(),
            (None, None) => return p,
        };
        p.in_field(field)
    }

    /// The place a problem [marks](Self::mark); none for a problem that does not carry one.
    #[must_use]
    pub fn of(p: &Problem) -> Option<Self> {
        let section = p.subject.as_deref()?.strip_prefix('[')?.strip_suffix(']')?;
        let mut spot = Self::section(section);
        let Some(field) = p.field.as_deref() else { return Some(spot) };
        let numbered = |text: &str| -> (String, usize) {
            match text.split_once('#') {
                Some((key, n)) => (key.to_owned(), n.parse().unwrap_or(0)),
                None => (text.to_owned(), 0),
            }
        };
        if let Some((scope, key)) = field.split_once('/') {
            spot.scope = Some(numbered(scope));
            spot.key = Some(key.to_owned()).filter(|k| !k.is_empty());
        } else {
            let (key, n) = numbered(field);
            spot.key = Some(key);
            spot.nth = n;
        }
        Some(spot)
    }
}

/// What an area found wrong in its sections of `mod.ltx`: its code (`E4nn` stops a build,
/// `W4nn` does not), its sentence, and the [`Spot`] it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub code: &'static str,
    pub text: String,
    pub spot: Spot,
}

impl Issue {
    #[must_use]
    pub fn new(code: &'static str, text: String, spot: Spot) -> Self {
        Self { code, text, spot }
    }

    /// The line the area's own answers print: the code, then the sentence.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{} {}", self.code, self.text)
    }

    /// The problem of the manifest's check, marked with its spot; the check puts it at its line.
    #[must_use]
    pub fn problem(&self) -> Problem {
        let p = if self.code.starts_with('E') {
            Problem::error(self.code, self.text.as_str())
        } else {
            Problem::warning(self.code, self.text.as_str())
        };
        self.spot.mark(p)
    }
}

/// A line of the manifest: where it is and how wide its key is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    /// 1-based.
    pub line: u32,
    /// 1-based, the first character of the key (or of the header).
    pub column: u32,
    /// The key's width in characters; 1 for a header.
    pub width: u32,
}

impl Place {
    /// Puts a problem at this place.
    #[must_use]
    pub fn put(self, p: Problem) -> Problem {
        p.at(self.line, self.column, self.line, self.column + self.width)
    }
}

impl ManifestDoc {
    /// Where a spot is written. A key the file does not have is at its section's header; a
    /// section it does not have is nowhere (`None`).
    #[must_use]
    pub fn locate(&self, spot: &Spot) -> Option<Place> {
        let text = self.to_string();
        let raw: Vec<&str> = text.lines().collect();
        let aligned = raw.len() >= self.items().count();
        let place = |i: usize, width: usize| Place {
            line: u32::try_from(i + 1).unwrap_or(u32::MAX),
            column: if aligned {
                raw.get(i).map_or(1, |l| {
                    u32::try_from(l.len() - l.trim_start_matches(crate::text::WS).len() + 1).unwrap_or(1)
                })
            } else {
                1
            },
            width: u32::try_from(width).unwrap_or(1).max(1),
        };
        let mut header: Option<Place> = None;
        let mut in_section = false;
        let (mut seen_key, mut seen_scope) = (0usize, 0usize);
        for (i, item) in self.items().enumerate() {
            match item {
                Item::Section(name) => {
                    in_section = name.eq_ignore_ascii_case(&spot.section);
                    if in_section && header.is_none() {
                        header = Some(place(i, 1));
                    }
                }
                Item::Entry { key, .. } if in_section && !key.is_empty() => {
                    let is = |wanted: &str| key.eq_ignore_ascii_case(wanted);
                    match (&spot.scope, &spot.key) {
                        (Some((scope, n)), wanted) => {
                            if is(scope) {
                                seen_scope += 1;
                                if seen_scope == n + 1 && wanted.is_none() {
                                    return Some(place(i, key.chars().count()));
                                }
                            } else if seen_scope == n + 1 && wanted.as_deref().is_some_and(is) {
                                return Some(place(i, key.chars().count()));
                            }
                        }
                        (None, Some(wanted)) if is(wanted) => {
                            if seen_key == spot.nth {
                                return Some(place(i, key.chars().count()));
                            }
                            seen_key += 1;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        header
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const TEXT: &str = "[module]\r\nid = od\r\n\r\n[screen.night]\r\nshow = loading\r\nlayer = a.dds\r\nopacity = 1\r\n  layer = b.dds\r\ntexture = t.dds\r\n\r\n[track.rain]\r\nfile = r.ogg\r\n";

    #[test]
    fn a_spot_travels_in_the_problem_and_comes_back() {
        for spot in [
            Spot::section("start_kit"),
            Spot::key("world", "area"),
            Spot::nth("screen.a", "layer", 0),
            Spot::within("screen.a", "layer", 2, Some("texture")),
            Spot::within("screen.a", "layer", 1, None),
        ] {
            let p = spot.mark(Problem::error("E000", "x"));
            assert_eq!(Spot::of(&p).unwrap(), spot, "{p:?}");
        }
        assert!(Spot::of(&Problem::error("E000", "x")).is_none());
        assert!(Spot::of(&Problem::error("E000", "x").about("a character")).is_none());
    }

    #[test]
    fn the_document_knows_the_lines() {
        let doc = ManifestDoc::parse(TEXT);
        let at = |spot: Spot| doc.locate(&spot).map(|p| (p.line, p.column, p.width));
        assert_eq!(at(Spot::section("screen.night")), Some((4, 1, 1)));
        assert_eq!(at(Spot::section("SCREEN.NIGHT")), Some((4, 1, 1)));
        assert_eq!(at(Spot::key("screen.night", "show")), Some((5, 1, 4)));
        // the first and the second layer line, and the texture that follows the second
        assert_eq!(at(Spot::within("screen.night", "layer", 0, None)), Some((6, 1, 5)));
        assert_eq!(at(Spot::within("screen.night", "layer", 1, None)), Some((8, 3, 5)));
        assert_eq!(at(Spot::within("screen.night", "layer", 0, Some("texture"))), Some((4, 1, 1)));
        assert_eq!(at(Spot::within("screen.night", "layer", 1, Some("texture"))), Some((9, 1, 7)));
        assert_eq!(at(Spot::within("screen.night", "layer", 0, Some("opacity"))), Some((7, 1, 7)));
        assert_eq!(at(Spot::nth("screen.night", "layer", 1)), Some((8, 3, 5)));
        // a key the file lacks is at its header, a section it lacks is nowhere
        assert_eq!(at(Spot::key("track.rain", "play")), Some((11, 1, 1)));
        assert_eq!(at(Spot::section("world")), None);
    }
}
