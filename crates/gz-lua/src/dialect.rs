//! The editor's LuaJIT dialect in the vendored LuaJIT.
//!
//! The C++ editor's LuaJIT is a fork that also reads `//` line comments and `/* */` block
//! comments (`gz_luadata` ports its lexer). The vendored LuaJIT reads neither, so a text the
//! editor accepts is rewritten before it is compiled here: each such comment becomes one space.
//! The fork counts no line inside `/* */`, so the line breaks of a block comment go with it and
//! every later line keeps the number the fork gives it; and like the fork, a `*` that does not
//! close the comment swallows the character after it.

use std::borrow::Cow;

/// The position after a short string that opens at `at` (its quote), or the end of the line
/// when it is not closed there.
fn short_string_end(b: &[u8], at: usize) -> usize {
    let quote = b.get(at).copied().unwrap_or(b'"');
    let mut j = at + 1;
    loop {
        match b.get(j) {
            None => return b.len(),
            Some(&c) if c == quote => return j + 1,
            Some(b'\\') => j += 2,
            Some(b'\n' | b'\r') => return j,
            Some(_) => j += 1,
        }
    }
}

/// The level of a long bracket `[==[` opening at `at`, when one opens there.
fn long_open(b: &[u8], at: usize) -> Option<usize> {
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
    b.get(at..).and_then(|r| r.iter().position(|&c| c == b'\n' || c == b'\r')).map_or(b.len(), |n| at + n)
}

/// `src` with the fork's `//` and `/* */` comments made spaces; borrowed when it has none.
#[must_use]
pub fn vanilla_source(src: &str) -> Cow<'_, str> {
    let b = src.as_bytes();
    if !b.windows(2).any(|w| w == b"//" || w == b"/*") {
        return Cow::Borrowed(src);
    }
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut changed = false;
    while let Some(&c) = b.get(i) {
        let next = b.get(i + 1).copied();
        let end = match c {
            b'"' | b'\'' => short_string_end(b, i),
            b'[' => long_open(b, i).map_or(i + 1, |level| long_end(b, i, level)),
            b'-' if next == Some(b'-') => long_open(b, i + 2).map_or_else(|| line_end(b, i), |level| long_end(b, i + 2, level)),
            b'/' if next == Some(b'/') => {
                changed = true;
                out.push(b' ');
                i = line_end(b, i);
                continue;
            }
            b'/' if next == Some(b'*') => {
                changed = true;
                let mut j = i + 2;
                while let Some(&d) = b.get(j) {
                    j += 1;
                    if d == b'*' {
                        if b.get(j) == Some(&b'/') {
                            j += 1;
                            break;
                        }
                        j += 1;
                    }
                }
                out.push(b' ');
                i = j.min(b.len());
                continue;
            }
            _ => i + 1,
        };
        let end = end.min(b.len()).max(i + 1);
        out.extend_from_slice(b.get(i..end).unwrap_or(&[]));
        i = end;
    }
    if !changed {
        return Cow::Borrowed(src);
    }
    String::from_utf8(out).map_or(Cow::Borrowed(src), Cow::Owned)
}
