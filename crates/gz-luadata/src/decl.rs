//! The declarative source scan of `NqLua.cpp` (`DeclarativeProblem`): the empty environment
//! only stops code that touches a global, so literal arithmetic, `local` and concatenation are
//! refused here, the way the game's loader refuses them (`xms_nq_load.script`).

fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
}
fn digit(c: u8) -> bool {
    c.is_ascii_digit()
}
fn hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}
fn alpha(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn punct(c: u8) -> bool {
    matches!(c, b'{' | b'}' | b'[' | b']' | b'=' | b',' | b';' | b'-')
}

/// `s[i] == '['`: the level of the long bracket and the offset after it.
fn long_bracket_at(s: &[u8], i: usize) -> Option<(usize, usize)> {
    let mut j = i + 1;
    let mut level = 0;
    while s.get(j) == Some(&b'=') {
        level += 1;
        j += 1;
    }
    (s.get(j) == Some(&b'[')).then_some((level, j + 1))
}

fn skip_long(s: &[u8], from: usize, level: usize, what: &str) -> Result<usize, Vec<u8>> {
    let mut close = Vec::with_capacity(level + 2);
    close.push(b']');
    close.resize(level + 1, b'=');
    close.push(b']');
    let cl = close.len();
    let mut j = from;
    while j + cl <= s.len() {
        if s.get(j..j + cl) == Some(close.as_slice()) {
            return Ok(j + cl);
        }
        j += 1;
    }
    Err(format!("unterminated {what}").into_bytes())
}

#[derive(PartialEq, Eq)]
enum T<'a> {
    Str,
    Num,
    Word(&'a [u8]),
    Punct(u8),
}

fn quoted(t: &T<'_>) -> Vec<u8> {
    match t {
        T::Str => b"\x01str".to_vec(),
        T::Num => b"\x01num".to_vec(),
        T::Word(w) => w.to_vec(),
        T::Punct(c) => vec![*c],
    }
}

/// The problem when the source is something other than one literal table, as bytes.
pub(crate) fn problem(s: &[u8]) -> Option<Vec<u8>> {
    let n = s.len();
    let mut toks: Vec<T<'_>> = Vec::new();
    let mut i = 0;
    while let Some(&c) = s.get(i) {
        if space(c) {
            i += 1;
            continue;
        }
        if c == b'-' && s.get(i + 1) == Some(&b'-') {
            let long = if s.get(i + 2) == Some(&b'[') { long_bracket_at(s, i + 2) } else { None };
            if let Some((level, after)) = long {
                match skip_long(s, after, level, "long comment") {
                    Ok(next) => i = next,
                    Err(p) => return Some(p),
                }
            } else {
                while i < n && s.get(i) != Some(&b'\n') {
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
            }
            continue;
        }
        if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while let Some(&d) = s.get(j) {
                if d == b'\\' {
                    j += 2;
                } else if d == c || d == b'\n' {
                    break;
                } else {
                    j += 1;
                }
            }
            if s.get(j) != Some(&c) {
                return Some(b"unterminated string".to_vec());
            }
            toks.push(T::Str);
            i = j + 1;
            continue;
        }
        if c == b'[' {
            if let Some((level, after)) = long_bracket_at(s, i) {
                match skip_long(s, after, level, "long string") {
                    Ok(next) => i = next,
                    Err(p) => return Some(p),
                }
                toks.push(T::Str);
            } else {
                toks.push(T::Punct(b'['));
                i += 1;
            }
            continue;
        }
        if digit(c) || (c == b'.' && s.get(i + 1).is_some_and(|&d| digit(d))) {
            let mut j = i;
            while let Some(&d) = s.get(j) {
                let prev = if j > i { s.get(j - 1).copied() } else { None };
                if hex(d)
                    || d == b'x'
                    || d == b'X'
                    || d == b'.'
                    || ((d == b'+' || d == b'-') && matches!(prev, Some(b'e' | b'E' | b'p' | b'P')))
                {
                    j += 1;
                } else {
                    break;
                }
            }
            if s.get(i..j).is_some_and(|t| t.windows(2).any(|w| w == b"..")) {
                return Some(b"concatenation is code, not data".to_vec());
            }
            toks.push(T::Num);
            i = j;
            continue;
        }
        if alpha(c) {
            let mut j = i;
            while s.get(j).is_some_and(|&d| alpha(d) || digit(d)) {
                j += 1;
            }
            toks.push(T::Word(s.get(i..j).unwrap_or(&[])));
            i = j;
            continue;
        }
        if punct(c) {
            toks.push(T::Punct(c));
            i += 1;
            continue;
        }
        let mut m = b"'".to_vec();
        m.push(c);
        m.extend(b"' outside a string is code, not data");
        return Some(m);
    }
    let Some(first) = toks.first() else { return Some(b"no 'return' statement".to_vec()) };
    if *first != T::Word(b"return") {
        let mut m = b"'".to_vec();
        m.extend(quoted(first));
        m.extend(b"' before the table is code, not data");
        return Some(m);
    }
    for (k, t) in toks.iter().enumerate().skip(1) {
        let T::Word(w) = t else { continue };
        if matches!(*w, b"true" | b"false" | b"nil") {
            continue;
        }
        if toks.get(k + 1) == Some(&T::Punct(b'=')) {
            continue;
        }
        let mut m = b"'".to_vec();
        m.extend(*w);
        m.extend(b"' is code, not data (only literals and 'key =' names are allowed)");
        return Some(m);
    }
    None
}
