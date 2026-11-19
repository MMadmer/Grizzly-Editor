//! The tokens of a Lua text as the game's LuaJIT reads them: Lua 5.1 with LuaJIT's numbers and
//! the fork's `//` and `/* */` comments. Comments and blanks are skipped; lexing never fails (a
//! string that is not closed ends at its line, a comment that is not closed at the end).

/// What a token is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tk {
    Name,
    Keyword,
    Str,
    Number,
    Punct,
}

/// A token: its kind, its text, its byte range and its 1-based line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Token<'a> {
    pub kind: Tk,
    pub text: &'a str,
    pub start: usize,
    pub end: usize,
    pub line: u32,
}

impl Token<'_> {
    pub fn is(&self, text: &str) -> bool {
        matches!(self.kind, Tk::Keyword | Tk::Punct) && self.text == text
    }

    /// A string's content without its quotes or brackets, escapes left as written.
    pub fn string_body(&self) -> &str {
        let t = self.text;
        if let Some(level) = long_open(t.as_bytes(), 0) {
            let from = level + 2;
            let to = t.len().saturating_sub(level + 2).max(from);
            return t.get(from..to).unwrap_or("");
        }
        let quote = t.get(..1).unwrap_or("");
        let inner = t.get(1..).unwrap_or("");
        inner.strip_suffix(quote).unwrap_or(inner)
    }
}

/// Lua's reserved words (5.1, and `goto` of 5.2 that LuaJIT takes).
pub(crate) const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// The level of a long bracket that opens at `at` (`[[` is 0, `[==[` is 2), if one does.
pub(crate) fn long_open(b: &[u8], at: usize) -> Option<usize> {
    if b.get(at) != Some(&b'[') {
        return None;
    }
    let mut j = at + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    (b.get(j) == Some(&b'[')).then_some(j - at - 1)
}

/// The position after the long bracket of `level` that closes one opened at `at`.
fn long_end(b: &[u8], at: usize, level: usize) -> usize {
    let mut j = at + level + 2;
    while j < b.len() {
        if b.get(j) == Some(&b']') {
            let mut k = j + 1;
            while b.get(k) == Some(&b'=') {
                k += 1;
            }
            if k - j - 1 == level && b.get(k) == Some(&b']') {
                return k + 1;
            }
        }
        j += 1;
    }
    b.len()
}

fn line_end(b: &[u8], at: usize) -> usize {
    b.get(at..).and_then(|r| r.iter().position(|&c| c == b'\n')).map_or(b.len(), |n| at + n)
}

/// The end of a short string opened by the quote at `at`: after its closing quote, or at the
/// end of its line when it is not closed there.
fn short_string_end(b: &[u8], at: usize) -> usize {
    let quote = b.get(at).copied().unwrap_or(b'"');
    let mut j = at + 1;
    loop {
        match b.get(j) {
            None => return b.len(),
            Some(&c) if c == quote => return j + 1,
            // an escaped line break continues the string on the next line
            Some(b'\\') => j += 2,
            Some(b'\n') => return j,
            Some(_) => j += 1,
        }
    }
}

/// The end of a `/* */` comment opened at `at`, as the fork reads it: a `*` that does not close
/// it swallows the character after it.
fn block_comment_end(b: &[u8], at: usize) -> usize {
    let mut j = at + 2;
    while let Some(&d) = b.get(j) {
        j += 1;
        if d == b'*' {
            if b.get(j) == Some(&b'/') {
                return j + 1;
            }
            j += 1;
        }
    }
    b.len()
}

fn number_end(b: &[u8], at: usize) -> usize {
    let mut j = at;
    let hex = b.get(at) == Some(&b'0') && matches!(b.get(at + 1), Some(b'x' | b'X'));
    while let Some(&c) = b.get(j) {
        let exponent = if hex { matches!(c, b'p' | b'P') } else { matches!(c, b'e' | b'E') };
        if exponent && matches!(b.get(j + 1), Some(b'+' | b'-')) {
            j += 2;
        } else if c.is_ascii_alphanumeric() || c == b'.' || c == b'_' {
            j += 1;
        } else {
            break;
        }
    }
    j
}

fn count_lines(b: &[u8], from: usize, to: usize) -> u32 {
    let n = b.get(from..to).map_or(0, |r| r.iter().filter(|&&c| c == b'\n').count());
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// What [`scan`] found besides the tokens: the `/* */` comments that span lines, as the lines
/// they start and end on - the fork counts no line inside one.
#[derive(Debug, Default)]
pub(crate) struct Scan<'a> {
    pub tokens: Vec<Token<'a>>,
    pub folded: Vec<(u32, u32)>,
}

/// The tokens of `src`, comments and blanks left out.
pub(crate) fn scan(src: &str) -> Scan<'_> {
    let b = src.as_bytes();
    let mut out = Scan::default();
    let mut i = 0;
    let mut line = 1u32;
    while let Some(&c) = b.get(i) {
        let next = b.get(i + 1).copied();
        let (kind, end) = match c {
            b'\n' => {
                line += 1;
                i += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c' => {
                i += 1;
                continue;
            }
            b'-' if next == Some(b'-') => {
                let end =
                    long_open(b, i + 2).map_or_else(|| line_end(b, i), |level| long_end(b, i + 2, level));
                line += count_lines(b, i, end);
                i = end;
                continue;
            }
            b'/' if next == Some(b'/') => {
                i = line_end(b, i);
                continue;
            }
            b'/' if next == Some(b'*') => {
                let end = block_comment_end(b, i);
                let lines = count_lines(b, i, end);
                if lines > 0 {
                    out.folded.push((line, line + lines));
                }
                line += lines;
                i = end;
                continue;
            }
            b'"' | b'\'' => (Tk::Str, short_string_end(b, i)),
            b'[' => match long_open(b, i) {
                Some(level) => (Tk::Str, long_end(b, i, level)),
                None => (Tk::Punct, i + 1),
            },
            b'0'..=b'9' => (Tk::Number, number_end(b, i)),
            b'.' if next.is_some_and(|n| n.is_ascii_digit()) => (Tk::Number, number_end(b, i)),
            c if is_name_start(c) => {
                let mut j = i + 1;
                while b.get(j).is_some_and(|&c| is_name(c)) {
                    j += 1;
                }
                (Tk::Name, j)
            }
            _ => {
                let three = b.get(i..i + 3);
                let two = b.get(i..i + 2);
                let len = if three == Some(b"...") {
                    3
                } else if matches!(
                    two,
                    Some(b"..") | Some(b"==") | Some(b"~=") | Some(b"<=") | Some(b">=") | Some(b"::")
                ) {
                    2
                } else {
                    // a whole character: a stray non-ASCII byte never splits one
                    src.get(i..).and_then(|r| r.chars().next()).map_or(1, char::len_utf8)
                };
                (Tk::Punct, i + len)
            }
        };
        let end = end.clamp(i + 1, b.len());
        let Some(text) = src.get(i..end) else {
            i = end;
            continue;
        };
        let kind = if kind == Tk::Name && KEYWORDS.contains(&text) { Tk::Keyword } else { kind };
        out.tokens.push(Token { kind, text, start: i, end, line });
        line += count_lines(b, i, end);
        i = end;
    }
    out
}

/// The tokens of `src`.
pub(crate) fn tokens(src: &str) -> Vec<Token<'_>> {
    scan(src).tokens
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn strings_comments_and_the_forks_comments_are_no_tokens_of_code() {
        let src = "local a = \"x -- y\" -- c\n// d\nb = [[\nlong]] /* e\nf */ c = 'q\\'r' .. 1e+5 ... ~=";
        let s = scan(src);
        let words: Vec<(&str, u32)> = s.tokens.iter().map(|t| (t.text, t.line)).collect();
        assert_eq!(
            words,
            [
                ("local", 1),
                ("a", 1),
                ("=", 1),
                ("\"x -- y\"", 1),
                ("b", 3),
                ("=", 3),
                ("[[\nlong]]", 3),
                ("c", 5),
                ("=", 5),
                ("'q\\'r'", 5),
                ("..", 5),
                ("1e+5", 5),
                ("...", 5),
                ("~=", 5),
            ]
        );
        assert_eq!(s.folded, [(4, 5)]);
        assert_eq!(s.tokens[0].kind, Tk::Keyword);
        assert_eq!(s.tokens[6].string_body(), "\nlong");
        assert_eq!(s.tokens[9].string_body(), "q\\'r");
    }

    #[test]
    fn an_open_string_ends_at_its_line() {
        let t = tokens("a = \"open\nb = 1");
        assert_eq!(t.iter().map(|t| t.text).collect::<Vec<_>>(), ["a", "=", "\"open", "b", "=", "1"]);
        assert_eq!(t[3].line, 2);
    }
}
