//! Ranking for every list that is searched by typing: the Command Palette, the pick lists.
//!
//! Lower is better. A query is words; each must match what a row shows. A word matches best the
//! whole name, then its start, then the start of one of its words ("log" finds Output Log before
//! Catalog), then anywhere in it, and last as letters in order where each letter starts a word or
//! follows the one before ("big" finds Build Into Game; letters scattered through the middle of
//! words are no match at all). The second text of a row (its path, its category, its detail)
//! counts less than its name; its keys least.

/// No match.
pub const NO_MATCH: i32 = 0x3fff_ffff;
/// A match that is only every letter in order: never better than any real one.
const LETTERS: i32 = 160;

fn is_break(c: u8) -> bool {
    matches!(c, b' ' | b'_' | b'-' | b'/' | b'\\' | b'>' | b'.' | b'(' | b':' | b',')
}

/// How well one lower-case `token` matches one lower-case `hay`; `letters` lets the
/// letters-in-order form count.
#[must_use]
pub fn score_in(token: &str, hay: &str, letters: bool) -> i32 {
    if token.is_empty() || hay.is_empty() {
        return NO_MATCH;
    }
    if hay == token {
        return 0;
    }
    let h = hay.as_bytes();
    if hay.starts_with(token) {
        return 4 + ((hay.len() - token.len()) / 8) as i32;
    }
    let mut from = 0;
    while let Some(off) = hay.get(from..).and_then(|rest| rest.find(token)) {
        let at = from + off;
        if at > 0 && h.get(at - 1).copied().is_some_and(is_break) {
            let mut word = 0;
            for i in 0..at {
                if h.get(i).copied().is_some_and(is_break)
                    && h.get(i + 1).copied().is_some_and(|c| !is_break(c))
                {
                    word += 1;
                }
            }
            return 16 + word * 2;
        }
        from = at + 1;
        if from >= hay.len() {
            break;
        }
    }
    if let Some(sub) = hay.find(token) {
        return 48 + sub as i32;
    }
    if !letters {
        return NO_MATCH;
    }
    let t = token.as_bytes();
    let (mut matched, mut first, mut previous, mut gaps) = (0usize, None::<usize>, 0usize, 0i32);
    for (i, &c) in h.iter().enumerate() {
        if matched >= t.len() {
            break;
        }
        if t.get(matched) != Some(&c) {
            continue;
        }
        let follows = first.is_some() && i == previous + 1;
        if !follows && i > 0 && !h.get(i - 1).copied().is_some_and(is_break) {
            continue;
        }
        if first.is_none() {
            first = Some(i);
        } else {
            gaps += (i - previous - 1) as i32;
        }
        previous = i;
        matched += 1;
    }
    match first {
        Some(f) if matched == t.len() => LETTERS + f as i32 + gaps * 3,
        _ => NO_MATCH,
    }
}

/// The query's words, lower case.
#[must_use]
pub fn tokenize(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Every word of the query matches what a row shows: its `name` counts most, its `second` text
/// less, its `keys` least. `NO_MATCH` when one word matches none of them.
#[must_use]
pub fn score(name: &str, second: &str, keys: &str, tokens: &[String]) -> i32 {
    let (name, second, keys) = (name.to_lowercase(), second.to_lowercase(), keys.to_lowercase());
    let mut total = 0;
    for token in tokens {
        let best = score_in(token, &name, true)
            .min(score_in(token, &second, false).saturating_add(12))
            .min(score_in(token, &keys, false).saturating_add(30));
        if best >= NO_MATCH {
            return NO_MATCH;
        }
        total += best;
    }
    total
}

/// The rows that match `query`, best first, as their indices; `row(i)` gives a row's name, its
/// second text and its keys. A tie keeps the shorter name first, then the given order. No query:
/// every row in the given order.
#[must_use]
pub fn rank<'a>(count: usize, query: &str, row: impl Fn(usize) -> (&'a str, &'a str, &'a str)) -> Vec<usize> {
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return (0..count).collect();
    }
    let mut scored: Vec<(i32, usize, usize)> = (0..count)
        .filter_map(|i| {
            let (name, second, keys) = row(i);
            let s = score(name, second, keys, &tokens);
            (s < NO_MATCH).then_some((s, name.len(), i))
        })
        .collect();
    scored.sort_unstable();
    scored.into_iter().map(|(_, _, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::{NO_MATCH, rank, score_in};

    #[test]
    fn words_beat_the_middle() {
        assert_eq!(score_in("log", "log", true), 0);
        assert!(score_in("log", "output log", true) < score_in("log", "catalog", true));
        assert!(score_in("big", "build into game", true) < NO_MATCH);
        assert_eq!(score_in("log", "build into game", true), NO_MATCH);
    }

    #[test]
    fn a_word_prefix_ranks_first_and_letters_in_order_last() {
        let rows = [
            ("Catalog", "Mod"),
            ("Output Log", "Window"),
            ("Build Into Game", "Mod"),
            ("Log Drawer", "Window"),
        ];
        let out = rank(rows.len(), "log", |i| (rows[i].0, rows[i].1, ""));
        // the start of a name, then the start of a word, then the middle
        assert_eq!(out, vec![3, 1, 0]);
        let big = rank(rows.len(), "big", |i| (rows[i].0, rows[i].1, ""));
        assert_eq!(big, vec![2]);
        // the second text counts, less than the name
        let window = rank(rows.len(), "window", |i| (rows[i].0, rows[i].1, ""));
        assert_eq!(window, vec![1, 3]);
        // no query: the given order
        assert_eq!(rank(rows.len(), " ", |i| (rows[i].0, "", "")), vec![0, 1, 2, 3]);
    }
}
