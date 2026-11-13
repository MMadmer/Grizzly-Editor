//! The syntax tree of a declarative Lua file: every table with the byte offsets of its braces,
//! every entry with the offsets of its key, value and separator. Whatever lies between two of
//! those offsets is trivia - spaces, line ends, comments - and is the text's own: the tree never
//! holds a copy of it, so the text is always the document byte for byte.

/// A token of the data subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tk {
    Name,
    Num,
    Str,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Eq,
    Comma,
    Semi,
    Minus,
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Token {
    pub kind: Tk,
    pub start: u32,
    pub end: u32,
}

/// A refusal with the offset it is about.
#[derive(Clone, Debug)]
pub(crate) struct PErr {
    pub at: usize,
    pub msg: String,
}

fn perr(at: usize, msg: impl Into<String>) -> PErr {
    PErr { at, msg: msg.into() }
}

pub(crate) fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The level of a long bracket opening at `i` (`[[` is 0, `[==[` is 2) and the offset after it.
pub(crate) fn long_open(s: &[u8], i: usize) -> Option<(usize, usize)> {
    let mut j = i + 1;
    while s.get(j) == Some(&b'=') {
        j += 1;
    }
    (s.get(j) == Some(&b'[')).then_some((j - i - 1, j + 1))
}

/// The offset after the long bracket of `level` that closes from `from`.
fn long_close(s: &[u8], from: usize, level: usize) -> Option<usize> {
    let mut j = from;
    while let Some(rel) = s.get(j..).and_then(|r| r.iter().position(|&c| c == b']')) {
        let at = j + rel;
        let mut k = at + 1;
        let mut n = 0;
        while s.get(k) == Some(&b'=') {
            n += 1;
            k += 1;
        }
        if n == level && s.get(k) == Some(&b']') {
            return Some(k + 1);
        }
        j = at + 1;
    }
    None
}

/// The text of a token as a message quotes it.
fn shown(s: &[u8], t: Token) -> String {
    if t.kind == Tk::Eof {
        return "<eof>".into();
    }
    let b = s.get(t.start as usize..t.end as usize).unwrap_or(&[]);
    let b = b.get(..b.len().min(40)).unwrap_or(b);
    String::from_utf8_lossy(b).into_owned()
}

pub(crate) struct Lex<'a> {
    pub s: &'a [u8],
    pub p: usize,
}

impl<'a> Lex<'a> {
    pub fn new(s: &'a [u8], from: usize) -> Self {
        Self { s, p: from }
    }

    /// Skips spaces and comments from `p`.
    fn trivia(&mut self) -> Result<(), PErr> {
        let s = self.s;
        while let Some(&c) = s.get(self.p) {
            if is_space(c) {
                self.p += 1;
            } else if c == b'-' && s.get(self.p + 1) == Some(&b'-') {
                let at = self.p;
                self.p += 2;
                if s.get(self.p) == Some(&b'[')
                    && let Some((level, after)) = long_open(s, self.p)
                {
                    self.p =
                        long_close(s, after, level).ok_or_else(|| perr(at, "unfinished long comment"))?;
                    continue;
                }
                while let Some(&d) = s.get(self.p) {
                    if d == b'\n' || d == b'\r' {
                        break;
                    }
                    self.p += 1;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn string(&mut self, start: usize, delim: u8) -> Result<(), PErr> {
        let s = self.s;
        let mut j = start + 1;
        loop {
            match s.get(j) {
                None => return Err(perr(start, "unfinished string")),
                Some(&c) if c == delim => {
                    self.p = j + 1;
                    return Ok(());
                }
                Some(b'\n' | b'\r') => return Err(perr(start, "unfinished string")),
                Some(b'\\') => {
                    j += 1;
                    match s.get(j) {
                        Some(&nl @ (b'\n' | b'\r')) => {
                            j += 1;
                            if let Some(&o @ (b'\n' | b'\r')) = s.get(j)
                                && o != nl
                            {
                                j += 1;
                            }
                        }
                        Some(b'z') => {
                            j += 1;
                            while s.get(j).is_some_and(|&c| is_space(c)) {
                                j += 1;
                            }
                        }
                        Some(_) => j += 1,
                        None => return Err(perr(start, "unfinished string")),
                    }
                }
                Some(_) => j += 1,
            }
        }
    }

    fn number(&mut self, start: usize) {
        let s = self.s;
        let mut j = start;
        let xp = if s.get(j) == Some(&b'0') && s.get(j + 1).is_some_and(|&c| c | 0x20 == b'x') {
            b'p'
        } else {
            b'e'
        };
        let mut prev = 0u8;
        while let Some(&c) = s.get(j) {
            if is_ident(c) || c >= 0x80 || c == b'.' || ((c == b'-' || c == b'+') && (prev | 0x20) == xp) {
                prev = c;
                j += 1;
            } else {
                break;
            }
        }
        self.p = j;
    }

    pub fn next(&mut self) -> Result<Token, PErr> {
        self.trivia()?;
        let s = self.s;
        let start = self.p;
        let tok = |kind, end: usize| Token { kind, start: start as u32, end: end as u32 };
        let Some(&c) = s.get(start) else { return Ok(tok(Tk::Eof, start)) };
        let single = |kind| Ok(tok(kind, start + 1));
        self.p = start + 1;
        match c {
            b'{' => single(Tk::LBrace),
            b'}' => single(Tk::RBrace),
            b']' => single(Tk::RBrack),
            b'=' => single(Tk::Eq),
            b',' => single(Tk::Comma),
            b';' => single(Tk::Semi),
            b'-' => single(Tk::Minus),
            b'[' => {
                if let Some((level, after)) = long_open(s, start) {
                    self.p =
                        long_close(s, after, level).ok_or_else(|| perr(start, "unfinished long string"))?;
                    Ok(tok(Tk::Str, self.p))
                } else if s.get(start + 1) == Some(&b'=') {
                    Err(perr(start, "invalid long string delimiter"))
                } else {
                    single(Tk::LBrack)
                }
            }
            b'"' | b'\'' => {
                self.string(start, c)?;
                Ok(tok(Tk::Str, self.p))
            }
            b'0'..=b'9' => {
                self.number(start);
                Ok(tok(Tk::Num, self.p))
            }
            b'.' if s.get(start + 1).is_some_and(u8::is_ascii_digit) => {
                self.number(start);
                Ok(tok(Tk::Num, self.p))
            }
            c if is_ident_start(c) => {
                let mut j = start + 1;
                while s.get(j).is_some_and(|&c| is_ident(c)) {
                    j += 1;
                }
                self.p = j;
                Ok(tok(Tk::Name, j))
            }
            _ => {
                let ch = std::str::from_utf8(s.get(start..).unwrap_or(&[]))
                    .ok()
                    .and_then(|r| r.chars().next())
                    .map_or_else(|| char::from(c).to_string(), |ch| ch.to_string());
                Err(perr(start, format!("'{ch}' outside a string is code, not data")))
            }
        }
    }
}

/// A key as written: positional, a bare name, or `[expr]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum KeyN {
    Pos,
    Name { start: u32, end: u32 },
    Bracket { expr: ExprN },
}

/// An expression: `table` when it is exactly one table constructor, `lit` the kind of its
/// token when it is exactly one literal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExprN {
    pub start: u32,
    pub end: u32,
    pub table: Option<u32>,
    pub lit: Option<Tk>,
}

impl ExprN {
    /// The literal token the expression is, if it is one.
    pub fn token(&self) -> Option<Token> {
        self.lit.map(|kind| Token { kind, start: self.start, end: self.end })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntryN {
    pub start: u32,
    pub key: KeyN,
    pub val: ExprN,
    /// The offset of the `,` or `;` after the entry.
    pub sep: Option<u32>,
}

impl EntryN {
    /// The end of the entry with its separator.
    pub fn end_with_sep(&self) -> u32 {
        self.sep.map_or(self.val.end, |s| s + 1)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TableN {
    pub open: u32,
    pub close: u32,
    pub entries: Vec<EntryN>,
}

/// The parsed document.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Tree {
    /// Every table constructor of the file, in the order their `{` appear.
    pub tables: Vec<TableN>,
    pub root: u32,
    pub ret_at: u32,
    /// The offsets where lines start.
    pub lines: Vec<u32>,
}

/// The stack guard of the parser; the walk's limit on nesting is the reader's.
const MAX_LEVELS: u32 = 200;

struct P<'a> {
    lx: Lex<'a>,
    tok: Token,
    ahead: Option<Token>,
    prev_end: u32,
    tables: Vec<TableN>,
    level: u32,
}

impl P<'_> {
    fn advance(&mut self) -> Result<(), PErr> {
        self.prev_end = self.tok.end;
        self.tok = match self.ahead.take() {
            Some(t) => t,
            None => self.lx.next()?,
        };
        Ok(())
    }

    fn peek(&mut self) -> Result<Token, PErr> {
        if let Some(t) = self.ahead {
            return Ok(t);
        }
        let t = self.lx.next()?;
        self.ahead = Some(t);
        Ok(t)
    }

    fn text(&self, t: Token) -> &[u8] {
        self.lx.s.get(t.start as usize..t.end as usize).unwrap_or(&[])
    }

    fn near(&self, msg: &str) -> PErr {
        perr(self.tok.start as usize, format!("{msg} near '{}'", shown(self.lx.s, self.tok)))
    }

    fn line_of(&self, at: u32) -> usize {
        1 + self.lx.s.get(..at as usize).unwrap_or(&[]).iter().filter(|&&c| c == b'\n').count()
    }

    fn primary(&mut self) -> Result<Option<u32>, PErr> {
        match self.tok.kind {
            Tk::Num | Tk::Str => {
                self.advance()?;
                Ok(None)
            }
            Tk::Name => {
                let w = self.text(self.tok);
                if matches!(w, b"true" | b"false" | b"nil") {
                    self.advance()?;
                    return Ok(None);
                }
                let w = String::from_utf8_lossy(w).into_owned();
                if crate::write::is_keyword(&w) {
                    return Err(self.near("unexpected symbol"));
                }
                Err(perr(
                    self.tok.start as usize,
                    format!("'{w}' is code, not data (only literals and 'key =' names are allowed)"),
                ))
            }
            Tk::LBrace => self.table().map(Some),
            _ => Err(self.near("unexpected symbol")),
        }
    }

    fn unary(&mut self) -> Result<Option<u32>, PErr> {
        if self.tok.kind == Tk::Minus {
            self.advance()?;
            self.unary()?;
            return Ok(None);
        }
        self.primary()
    }

    fn expr(&mut self) -> Result<ExprN, PErr> {
        let start = self.tok.start;
        let first = self.tok.kind;
        let mut table = self.unary()?;
        let mut lit = (first != Tk::Minus && table.is_none()).then_some(first);
        while self.tok.kind == Tk::Minus {
            table = None;
            lit = None;
            self.advance()?;
            self.unary()?;
        }
        Ok(ExprN { start, end: self.prev_end, table, lit })
    }

    fn table(&mut self) -> Result<u32, PErr> {
        let open = self.tok.start;
        self.level += 1;
        if self.level > MAX_LEVELS {
            return Err(perr(open as usize, "chunk has too many syntax levels"));
        }
        let id = u32::try_from(self.tables.len()).unwrap_or(u32::MAX);
        self.tables.push(TableN::default());
        self.advance()?;
        let mut entries = Vec::new();
        while self.tok.kind != Tk::RBrace {
            let start = self.tok.start;
            let key = if self.tok.kind == Tk::LBrack {
                self.advance()?;
                let expr = self.expr()?;
                if self.tok.kind != Tk::RBrack {
                    return Err(self.near("']' expected"));
                }
                self.advance()?;
                if self.tok.kind != Tk::Eq {
                    return Err(self.near("'=' expected"));
                }
                self.advance()?;
                KeyN::Bracket { expr }
            } else if self.tok.kind == Tk::Name
                && (self.text(self.tok) == b"goto"
                    || !crate::write::is_keyword(&String::from_utf8_lossy(self.text(self.tok))))
                && self.peek()?.kind == Tk::Eq
            {
                let k = KeyN::Name { start: self.tok.start, end: self.tok.end };
                self.advance()?;
                self.advance()?;
                k
            } else {
                KeyN::Pos
            };
            let val = self.expr()?;
            let sep = if matches!(self.tok.kind, Tk::Comma | Tk::Semi) {
                let s = self.tok.start;
                self.advance()?;
                Some(s)
            } else {
                None
            };
            entries.push(EntryN { start, key, val, sep });
            if sep.is_none() {
                break;
            }
        }
        if self.tok.kind != Tk::RBrace {
            let ol = self.line_of(open);
            return Err(if ol == self.line_of(self.tok.start) {
                self.near("'}' expected")
            } else {
                self.near(&format!("'}}' expected (to close '{{' at line {ol})"))
            });
        }
        let close = self.tok.start;
        self.advance()?;
        self.level -= 1;
        if let Some(t) = self.tables.get_mut(id as usize) {
            *t = TableN { open, close, entries };
        }
        Ok(id)
    }
}

/// The offsets where lines start (after every `\n`).
pub(crate) fn line_starts(s: &[u8]) -> Vec<u32> {
    let mut v = Vec::with_capacity(s.len() / 32 + 1);
    v.push(0);
    for (i, &c) in s.iter().enumerate() {
        if c == b'\n' {
            v.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
        }
    }
    v
}

/// Parses the text from `from` (after a byte order mark).
pub(crate) fn parse(s: &[u8], from: usize) -> Result<Tree, PErr> {
    if u32::try_from(s.len()).is_err() {
        return Err(perr(0, "the file is larger than 4 GB"));
    }
    let mut lx = Lex::new(s, from);
    let first = lx.next()?;
    let mut p = P { lx, tok: first, ahead: None, prev_end: first.start, tables: Vec::new(), level: 0 };
    if p.tok.kind == Tk::Eof {
        return Err(perr(p.tok.start as usize, "no 'return' statement"));
    }
    if !(p.tok.kind == Tk::Name && p.text(p.tok) == b"return") {
        let w = shown(s, p.tok);
        return Err(perr(p.tok.start as usize, format!("'{w}' before the table is code, not data")));
    }
    let ret_at = p.tok.start;
    p.advance()?;
    let e = p.expr()?;
    if p.tok.kind == Tk::Comma {
        return Err(perr(p.tok.start as usize, "the file must return one table (return { ... })"));
    }
    let Some(root) = e.table else {
        return Err(perr(e.start as usize, "the file must return one table (return { ... })"));
    };
    if p.tok.kind == Tk::Semi {
        p.advance()?;
    }
    if p.tok.kind != Tk::Eof {
        return Err(p.near("'<eof>' expected"));
    }
    Ok(Tree { tables: p.tables, root, ret_at, lines: line_starts(s) })
}

/// Brings `tree` to `new`, a text that differs from the one the tree was read from only inside
/// `lo..hi` (offsets of the old text), by `delta` bytes: only the smallest table holding the
/// change is read again, everything after it moves. False, with the tree untouched, when the
/// change is no inner table's own (the whole text is then read again).
pub(crate) fn reparse_within(tree: &mut Tree, new: &[u8], lo: u32, hi: u32, delta: i64) -> bool {
    let shift = |x: u32| -> u32 { u32::try_from(i64::from(x) + delta).unwrap_or(x) };
    // the tables are in the order of their `{`: the last one that holds the change is the
    // smallest
    let Some(tid) = tree.tables.iter().rposition(|t| t.open < lo && hi <= t.close) else { return false };
    if tid as u32 == tree.root {
        return false;
    }
    let (open, close) = (tree.tables[tid].open, tree.tables[tid].close);
    let old_count = tree.tables[tid..].iter().take_while(|x| x.open <= close).count();
    let mut lx = Lex::new(new, open as usize);
    let Ok(first) = lx.next() else { return false };
    if first.kind != Tk::LBrace || first.start != open {
        return false;
    }
    let mut p = P { lx, tok: first, ahead: None, prev_end: first.start, tables: Vec::new(), level: 0 };
    if p.table().is_err() {
        return false;
    }
    let mut sub = p.tables;
    if sub.first().map(|s| s.close) != Some(shift(close)) {
        return false;
    }
    let base = tid as u32;
    let moved = sub.len() as i64 - old_count as i64;
    let after = (tid + old_count) as u32;
    let outer = |e: &mut ExprN| {
        if e.start > close {
            e.start = shift(e.start);
        }
        if e.end > close {
            e.end = shift(e.end);
        }
        if let Some(x) = e.table.as_mut()
            && *x >= after
        {
            *x = u32::try_from(i64::from(*x) + moved).unwrap_or(*x);
        }
    };
    let fix_outer = |x: &mut TableN| {
        if x.open > close {
            x.open = shift(x.open);
        }
        if x.close > close {
            x.close = shift(x.close);
        }
        for e in &mut x.entries {
            if e.start > close {
                e.start = shift(e.start);
            }
            if let Some(s) = e.sep.as_mut()
                && *s > close
            {
                *s = shift(*s);
            }
            match &mut e.key {
                KeyN::Name { start, end } if *start > close => {
                    *start = shift(*start);
                    *end = shift(*end);
                }
                KeyN::Bracket { expr } => outer(expr),
                _ => {}
            }
            outer(&mut e.val);
        }
    };
    for x in &mut sub {
        for e in &mut x.entries {
            if let KeyN::Bracket { expr } = &mut e.key {
                expr.table = expr.table.map(|t| t + base);
            }
            e.val.table = e.val.table.map(|t| t + base);
        }
    }
    for x in &mut tree.tables[..tid] {
        fix_outer(x);
    }
    for x in &mut tree.tables[tid + old_count..] {
        fix_outer(x);
    }
    tree.tables.splice(tid..tid + old_count, sub);
    // the line starts before the table stay, those inside are read again, those after move
    let new_close = shift(close) as usize;
    let keep = tree.lines.partition_point(|&l| l <= open);
    let tail: Vec<u32> = tree.lines.iter().copied().filter(|&l| l > close).map(shift).collect();
    tree.lines.truncate(keep);
    for (i, &c) in new.get(open as usize..new_close).unwrap_or(&[]).iter().enumerate() {
        if c == b'\n' {
            tree.lines.push(open + i as u32 + 1);
        }
    }
    tree.lines.extend(tail);
    true
}
