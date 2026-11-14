//! The template syntax of the catalog and of [`tr!`](crate::tr): `{name}` is a value, `{name:a|b}`
//! picks a word by the number `name` holds (English: one|other; Russian: one|few|many), `{{` and
//! `}}` are braces. A brace that opens no placeholder (`{ path = }`) is text.

use std::borrow::Cow;

use super::Lang;

/// A piece of a template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Seg<'t> {
    Lit(Cow<'t, str>),
    Var(&'t str),
    /// A word picked by a number: the value's name and the forms.
    Plural(&'t str, Vec<&'t str>),
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The placeholder that starts at `rest` (just after a `{`): its segment and its length up to and
/// including the closing brace.
fn placeholder(rest: &str) -> Option<(Seg<'_>, usize)> {
    let close = rest.find('}')?;
    let inner = &rest[..close];
    match inner.split_once(':') {
        Some((name, forms)) if is_name(name) && !forms.is_empty() => {
            Some((Seg::Plural(name, forms.split('|').collect()), close + 1))
        }
        None if is_name(inner) => Some((Seg::Var(inner), close + 1)),
        _ => None,
    }
}

/// Splits a template into its pieces.
pub(crate) fn parse(t: &str) -> Vec<Seg<'_>> {
    let mut out: Vec<Seg<'_>> = Vec::new();
    let mut lit = String::new();
    let mut lit_start = 0;
    let mut owned = false;
    let mut i = 0;
    let bytes = t.as_bytes();
    fn flush<'t>(out: &mut Vec<Seg<'t>>, lit: &mut String, owned: bool, piece: &'t str) {
        if owned {
            if !lit.is_empty() {
                out.push(Seg::Lit(Cow::Owned(std::mem::take(lit))));
            }
        } else if !piece.is_empty() {
            out.push(Seg::Lit(Cow::Borrowed(piece)));
        }
    }
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                if !owned {
                    lit.push_str(&t[lit_start..i]);
                    owned = true;
                }
                lit.push('{');
                i += 2;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                if !owned {
                    lit.push_str(&t[lit_start..i]);
                    owned = true;
                }
                lit.push('}');
                i += 2;
            }
            b'{' => match placeholder(&t[i + 1..]) {
                Some((seg, len)) => {
                    flush(&mut out, &mut lit, owned, &t[lit_start..i]);
                    owned = false;
                    out.push(seg);
                    i += 1 + len;
                    lit_start = i;
                }
                None => {
                    if owned {
                        lit.push('{');
                    }
                    i += 1;
                }
            },
            _ => {
                let ch = t[i..].chars().next().map_or(1, char::len_utf8);
                if owned {
                    lit.push_str(&t[i..i + ch]);
                }
                i += ch;
            }
        }
    }
    flush(&mut out, &mut lit, owned, &t[lit_start..]);
    out
}

/// Whether a text holds a placeholder at all.
pub(crate) fn has_placeholder(t: &str) -> bool {
    parse(t).iter().any(|s| !matches!(s, Seg::Lit(_)))
}

/// The names a template's placeholders use, each once, in order.
pub(crate) fn names<'t>(segs: &[Seg<'t>]) -> Vec<&'t str> {
    let mut out: Vec<&str> = Vec::new();
    for s in segs {
        if let Seg::Var(n) | Seg::Plural(n, _) = s
            && !out.contains(n)
        {
            out.push(n);
        }
    }
    out
}

/// A value a template is filled with.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Val<'v> {
    Num(i64),
    Text(&'v str),
}

impl Val<'_> {
    fn number(&self) -> Option<i64> {
        match self {
            Val::Num(n) => Some(*n),
            Val::Text(t) => t.trim().parse().ok(),
        }
    }
}

/// Fills a template: a value by its name, a word by the number a value holds, picked by the
/// language's rule. A placeholder without its value stays as it is written.
pub(crate) fn fill<'v>(segs: &[Seg<'_>], lang: Lang, value: impl Fn(&str) -> Option<Val<'v>>) -> String {
    let mut out = String::new();
    for s in segs {
        match s {
            Seg::Lit(t) => out.push_str(t),
            Seg::Var(n) => match value(n) {
                Some(Val::Num(v)) => out.push_str(&v.to_string()),
                Some(Val::Text(t)) => out.push_str(t),
                None => {
                    out.push('{');
                    out.push_str(n);
                    out.push('}');
                }
            },
            Seg::Plural(n, forms) => {
                let count = value(n).and_then(|v| v.number());
                let at = count.map_or(forms.len().saturating_sub(1), |c| lang.plural(c));
                out.push_str(forms.get(at).or(forms.last()).copied().unwrap_or(""));
            }
        }
    }
    out
}

/// What a text a template matched holds: each placeholder's name and the text it stood for.
pub(crate) type Captures<'m> = Vec<(&'m str, &'m str)>;

/// Matches a text against a template's pieces: the values its placeholders stood for, or `None`.
/// A value is never empty; a name used twice stands for the same text both times.
pub(crate) fn matches<'m>(segs: &[Seg<'m>], text: &'m str) -> Option<Captures<'m>> {
    let mut caps = Vec::new();
    step(segs, text, &mut caps).then_some(caps)
}

fn step<'m>(segs: &[Seg<'m>], text: &'m str, caps: &mut Captures<'m>) -> bool {
    let Some((first, rest)) = segs.split_first() else { return text.is_empty() };
    match first {
        Seg::Lit(l) => text.strip_prefix(l.as_ref()).is_some_and(|t| step(rest, t, caps)),
        Seg::Plural(_, forms) => {
            // the longest form first: "files" before "file"
            let mut sorted: Vec<&&str> = forms.iter().collect();
            sorted.sort_by_key(|f| std::cmp::Reverse(f.len()));
            sorted.into_iter().any(|f| text.strip_prefix(*f).is_some_and(|t| step(rest, t, caps)))
        }
        Seg::Var(name) => {
            if let Some(&(_, known)) = caps.iter().find(|(n, _)| n == name) {
                return text.strip_prefix(known).is_some_and(|t| step(rest, t, caps));
            }
            if text.is_empty() {
                return false;
            }
            // the shortest value that lets the rest match
            let ends = text.char_indices().skip(1).map(|(at, _)| at).chain(std::iter::once(text.len()));
            for at in ends {
                let (value, tail) = text.split_at(at);
                caps.push((name, value));
                if step(rest, tail, caps) {
                    return true;
                }
                caps.pop();
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Seg, Val, fill, matches, names, parse};
    use crate::i18n::Lang;

    #[test]
    fn placeholders_plurals_and_braces() {
        let segs = parse("Delete {n} {n:file|files} in {{root}}");
        assert_eq!(
            segs,
            vec![
                Seg::Lit("Delete ".into()),
                Seg::Var("n"),
                Seg::Lit(" ".into()),
                Seg::Plural("n", vec!["file", "files"]),
                Seg::Lit(" in {root}".into()),
            ]
        );
        assert_eq!(names(&segs), vec!["n"]);
        // a brace that opens no placeholder is text
        assert_eq!(parse("{ path = }"), vec![Seg::Lit("{ path = }".into())]);
    }

    #[test]
    fn a_word_follows_the_number_in_each_language() {
        let en = parse("{n} {n:file|files}");
        let ru = parse("{n} {n:файл|файла|файлов}");
        let one = |segs: &[Seg<'_>], lang, n| fill(segs, lang, |_| Some(Val::Num(n)));
        assert_eq!(one(&en, Lang::En, 1), "1 file");
        assert_eq!(one(&en, Lang::En, 2), "2 files");
        assert_eq!(one(&ru, Lang::Ru, 1), "1 файл");
        assert_eq!(one(&ru, Lang::Ru, 3), "3 файла");
        assert_eq!(one(&ru, Lang::Ru, 5), "5 файлов");
        assert_eq!(one(&ru, Lang::Ru, 11), "11 файлов");
        assert_eq!(one(&ru, Lang::Ru, 21), "21 файл");
        assert_eq!(one(&ru, Lang::Ru, 112), "112 файлов");
        assert_eq!(one(&ru, Lang::Ru, 0), "0 файлов");
    }

    #[test]
    fn a_sentence_gives_back_what_its_placeholders_stood_for() {
        let t = parse("The file {path} cannot be read: {reason}");
        let caps = matches(&t, "The file a/b.ltx cannot be read: it is locked").expect("matches");
        assert_eq!(caps, vec![("path", "a/b.ltx"), ("reason", "it is locked")]);
        assert!(matches(&t, "The file cannot be read: x").is_none(), "a value is never empty");
        let p = parse("{n} {n:object|objects} moved");
        assert_eq!(matches(&p, "12 objects moved"), Some(vec![("n", "12")]));
        assert_eq!(matches(&p, "1 object moved"), Some(vec![("n", "1")]));
        assert!(matches(&p, "12 things moved").is_none());
    }
}
