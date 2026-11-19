//! Find and Replace over a text: the matches of a query (Match Case, Whole Word) as byte ranges,
//! the one after or before the caret, a replacement of one match and of all of them.

use std::ops::Range;

/// What the find bar asks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub text: String,
    pub case: bool,
    pub whole_word: bool,
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every match of the query in `hay`, in order, none overlapping. Without Match Case the text and
/// the query are compared case-folded (Cyrillic too).
#[must_use]
pub fn matches(hay: &str, q: &Query) -> Vec<Range<usize>> {
    if q.text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if q.case {
        let mut from = 0usize;
        while let Some(i) = hay[from..].find(&q.text) {
            let (s, e) = (from + i, from + i + q.text.len());
            if !q.whole_word || bounded(hay, s, e) {
                out.push(s..e);
            }
            from = e.max(s + 1);
            while from < hay.len() && !hay.is_char_boundary(from) {
                from += 1;
            }
            if from >= hay.len() {
                break;
            }
        }
        return out;
    }
    // folded comparison, char by char: lower-casing can change a character's byte length
    let needle: Vec<char> = q.text.chars().flat_map(char::to_lowercase).collect();
    let chars: Vec<(usize, char)> = hay.char_indices().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let mut k = 0usize;
        let mut j = i;
        let mut ok = true;
        while k < needle.len() {
            let Some(&(_, c)) = chars.get(j) else {
                ok = false;
                break;
            };
            let mut low = c.to_lowercase();
            for n in low.by_ref() {
                if needle.get(k) != Some(&n) {
                    ok = false;
                    break;
                }
                k += 1;
            }
            if !ok {
                break;
            }
            j += 1;
        }
        if ok && k == needle.len() {
            let s = chars[i].0;
            let e = chars.get(j).map_or(hay.len(), |c| c.0);
            if !q.whole_word || bounded(hay, s, e) {
                out.push(s..e);
                i = j.max(i + 1);
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Whether a match stands as a word of its own.
fn bounded(hay: &str, s: usize, e: usize) -> bool {
    let before = hay[..s].chars().next_back().is_none_or(|c| !word(c));
    let after = hay[e..].chars().next().is_none_or(|c| !word(c));
    before && after
}

/// The match after the caret (F3), or before it (Shift+F3), going round the text's end.
#[must_use]
pub fn next(found: &[Range<usize>], caret: usize, back: bool, current: Option<usize>) -> Option<usize> {
    if found.is_empty() {
        return None;
    }
    if back {
        let before = current.map_or(caret, |c| found[c.min(found.len() - 1)].start);
        Some(found.iter().rposition(|r| r.start < before).unwrap_or(found.len() - 1))
    } else {
        let after = current.map_or(caret, |c| found[c.min(found.len() - 1)].start + 1);
        Some(found.iter().position(|r| r.start >= after).unwrap_or(0))
    }
}

/// The text with one match replaced; the byte after the replacement.
#[must_use]
pub fn replace_one(hay: &str, at: &Range<usize>, with: &str) -> (String, usize) {
    let mut out = String::with_capacity(hay.len() + with.len());
    out.push_str(&hay[..at.start]);
    out.push_str(with);
    out.push_str(&hay[at.end..]);
    (out, at.start + with.len())
}

/// The text with every match replaced, and how many there were.
#[must_use]
pub fn replace_all(hay: &str, found: &[Range<usize>], with: &str) -> (String, usize) {
    let mut out = String::with_capacity(hay.len());
    let mut at = 0usize;
    for r in found {
        out.push_str(&hay[at..r.start]);
        out.push_str(with);
        at = r.end;
    }
    out.push_str(&hay[at..]);
    (out, found.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str, case: bool, whole_word: bool) -> Query {
        Query { text: text.to_owned(), case, whole_word }
    }

    #[test]
    fn matches_by_case_and_word() {
        let t = "Actor actor actors ACTOR";
        assert_eq!(matches(t, &q("actor", false, false)).len(), 4);
        assert_eq!(matches(t, &q("actor", true, false)).len(), 2);
        assert_eq!(matches(t, &q("actor", false, true)).len(), 3);
        assert_eq!(matches("Сталкер сталкер", &q("СТАЛКЕР", false, false)).len(), 2);
        assert!(matches(t, &q("", false, false)).is_empty());
        let r = matches("aaa", &q("aa", true, false));
        assert_eq!(r, vec![0..2]);
    }

    #[test]
    fn next_goes_round() {
        let f = vec![0..1, 5..6, 9..10];
        assert_eq!(next(&f, 3, false, None), Some(1));
        assert_eq!(next(&f, 3, true, None), Some(0));
        assert_eq!(next(&f, 0, false, Some(2)), Some(0));
        assert_eq!(next(&f, 0, true, Some(0)), Some(2));
        assert_eq!(next(&[], 0, false, None), None);
    }

    #[test]
    fn replaces() {
        let t = "a b a";
        let f = matches(t, &q("a", true, true));
        assert_eq!(replace_all(t, &f, "xy"), ("xy b xy".to_owned(), 2));
        assert_eq!(replace_one(t, &f[1], "z"), ("a b z".to_owned(), 5));
    }
}
