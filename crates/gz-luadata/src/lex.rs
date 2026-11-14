//! LuaJIT 2.1's lexer (with `//` and `/* */` comments), modelled so a syntax error reads word
//! for word as LuaJIT writes it: the game reads these files with LuaJIT.

use crate::num::{Scanned, strscan};

pub(crate) type Tok = i32;

pub(crate) const TK_OFS: Tok = 256;
pub(crate) const TK_AND: Tok = 257;
pub(crate) const TK_BREAK: Tok = 258;
pub(crate) const TK_DO: Tok = 259;
pub(crate) const TK_ELSE: Tok = 260;
pub(crate) const TK_ELSEIF: Tok = 261;
pub(crate) const TK_END: Tok = 262;
pub(crate) const TK_FALSE: Tok = 263;
pub(crate) const TK_FOR: Tok = 264;
pub(crate) const TK_FUNCTION: Tok = 265;
pub(crate) const TK_GOTO: Tok = 266;
pub(crate) const TK_IF: Tok = 267;
pub(crate) const TK_IN: Tok = 268;
pub(crate) const TK_LOCAL: Tok = 269;
pub(crate) const TK_NIL: Tok = 270;
pub(crate) const TK_NOT: Tok = 271;
pub(crate) const TK_OR: Tok = 272;
pub(crate) const TK_REPEAT: Tok = 273;
pub(crate) const TK_RETURN: Tok = 274;
pub(crate) const TK_THEN: Tok = 275;
pub(crate) const TK_TRUE: Tok = 276;
pub(crate) const TK_UNTIL: Tok = 277;
pub(crate) const TK_WHILE: Tok = 278;
pub(crate) const TK_CONCAT: Tok = 279;
pub(crate) const TK_DOTS: Tok = 280;
pub(crate) const TK_EQ: Tok = 281;
pub(crate) const TK_GE: Tok = 282;
pub(crate) const TK_LE: Tok = 283;
pub(crate) const TK_NE: Tok = 284;
pub(crate) const TK_LABEL: Tok = 285;
pub(crate) const TK_NUMBER: Tok = 286;
pub(crate) const TK_NAME: Tok = 287;
pub(crate) const TK_STRING: Tok = 288;
pub(crate) const TK_EOF: Tok = 289;

const TOKEN_NAMES: [&str; 33] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "..", "...", "==", ">=", "<=",
    "~=", "::", "<number>", "<name>", "<string>", "<eof>",
];

/// The reserved words, in token order.
const RESERVED: usize = 22;

const LEX_EOF: i32 = -1;

/// `lj_char_bits`, indexed by `c + 1` (so -1, the end, is 0).
const CHAR_BITS: [u8; 257] = {
    let mut t = [0u8; 257];
    let mut c = 0usize;
    while c < 256 {
        t[c + 1] = match c {
            0..=8 | 14..=31 | 127 => 1,
            9..=13 => 3,
            32 => 2,
            48..=57 => 152,
            65..=70 => 176,
            71..=90 => 160,
            95 => 132,
            97..=102 => 208,
            103..=122 => 192,
            128..=255 => 128,
            _ => 4,
        };
        c += 1;
    }
    t
};

fn bits(c: i32) -> u8 {
    usize::try_from(c + 1).ok().and_then(|i| CHAR_BITS.get(i)).copied().unwrap_or(0)
}
pub(crate) fn is_cntrl(c: i32) -> bool {
    bits(c) & 0x01 != 0
}
pub(crate) fn is_space(c: i32) -> bool {
    bits(c) & 0x02 != 0
}
pub(crate) fn is_digit(c: i32) -> bool {
    bits(c) & 0x08 != 0
}
pub(crate) fn is_xdigit(c: i32) -> bool {
    bits(c) & 0x10 != 0
}
pub(crate) fn is_ident(c: i32) -> bool {
    bits(c) & 0x80 != 0
}

/// The value a token carries.
#[derive(Clone, Debug, Default)]
pub(crate) enum TokVal {
    #[default]
    None,
    Num(f64),
    /// An FFI number (`1LL`, `2i`): not data, but it compiles.
    Cdata,
    /// A name or a string, as bytes.
    Str(Vec<u8>),
}

/// A compile error before it is given its location prefix: the message, the line the lexer
/// stood on, the token it was near (`None` for the errors LuaJIT raises without one) and the
/// byte offset where that token starts.
#[derive(Clone, Debug)]
pub(crate) enum SynErr {
    At {
        line: u32,
        msg: Vec<u8>,
        near: Option<Vec<u8>>,
        at: usize,
    },
    /// A message that carries no location (the bytecode loader's).
    Raw(Vec<u8>),
}

pub(crate) type R<T> = Result<T, SynErr>;

pub(crate) struct Lexer<'a> {
    src: &'a [u8],
    p: usize,
    c: i32,
    /// `ls->sb`: the text of the token being (or last) scanned.
    pub sb: Vec<u8>,
    pub line: u32,
    pub lastline: u32,
    pub tok: Tok,
    pub tokval: TokVal,
    lookahead: Tok,
    lookaheadval: TokVal,
    /// Where the current token, the lookahead and the token being scanned start.
    tok_at: usize,
    look_at: usize,
    scan_at: usize,
    scanning: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self {
            src,
            p: 0,
            c: LEX_EOF,
            sb: Vec::new(),
            line: 1,
            lastline: 1,
            tok: 0,
            tokval: TokVal::None,
            lookahead: TK_EOF,
            lookaheadval: TokVal::None,
            tok_at: 0,
            look_at: 0,
            scan_at: 0,
            scanning: false,
        }
    }

    /// The offset of the character the lexer stands on.
    fn here(&self) -> usize {
        if self.c == LEX_EOF { self.src.len() } else { self.p.saturating_sub(1) }
    }

    fn next(&mut self) -> i32 {
        self.c = match self.src.get(self.p) {
            Some(&b) => {
                self.p += 1;
                i32::from(b)
            }
            None => LEX_EOF,
        };
        self.c
    }

    fn save(&mut self, c: i32) {
        // lj_buf_putb stores the low byte
        self.sb.push((c & 0xff) as u8);
    }

    fn savenext(&mut self) -> i32 {
        self.save(self.c);
        self.next()
    }

    fn iseol(&self) -> bool {
        self.c == i32::from(b'\n') || self.c == i32::from(b'\r')
    }

    fn newline(&mut self) {
        let old = self.c;
        self.next();
        if self.iseol() && self.c != old {
            self.next();
        }
        self.line += 1;
    }

    /// `lj_lex_setup`: the first character, a BOM, a `#` line. `Ok(true)` means the text is a
    /// bytecode dump.
    pub fn setup(&mut self) -> R<bool> {
        let mut header = false;
        self.next();
        if self.c == 0xef
            && self.p + 2 <= self.src.len()
            && self.src.get(self.p) == Some(&0xbb)
            && self.src.get(self.p + 1) == Some(&0xbf)
        {
            self.p += 2;
            self.next();
            header = true;
        }
        if self.c == i32::from(b'#') {
            loop {
                self.next();
                if self.c == LEX_EOF {
                    return Ok(false);
                }
                if self.iseol() {
                    break;
                }
            }
            self.newline();
            header = true;
        }
        if self.c == 0x1b {
            if header {
                return Err(SynErr::Raw(b"cannot load malformed bytecode".to_vec()));
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// The token as LuaJIT names it in a message (`lj_lex_token2str`).
    pub fn token2str(tok: Tok) -> Vec<u8> {
        if tok > TK_OFS {
            let i = usize::try_from(tok - TK_OFS - 1).unwrap_or(0);
            TOKEN_NAMES.get(i).map_or_else(Vec::new, |s| s.as_bytes().to_vec())
        } else if !is_cntrl(tok) {
            vec![(tok & 0xff) as u8]
        } else {
            format!("char({tok})").into_bytes()
        }
    }

    /// `lj_lex_error`: the error at the lexer's current line, near `tok` (0: near nothing).
    pub fn error(&self, tok: Tok, msg: impl Into<Vec<u8>>) -> SynErr {
        let near = if tok == 0 {
            None
        } else if tok == TK_NAME || tok == TK_STRING || tok == TK_NUMBER {
            // a C string: it ends at the first NUL the token holds
            let end = self.sb.iter().position(|&b| b == 0).unwrap_or(self.sb.len());
            Some(self.sb.get(..end).unwrap_or(&[]).to_vec())
        } else {
            Some(Self::token2str(tok))
        };
        let at = if self.scanning { self.scan_at } else { self.tok_at };
        SynErr::At { line: self.line, msg: msg.into(), near, at }
    }

    fn number(&mut self) -> R<TokVal> {
        let first = self.c;
        let mut xp = i32::from(b'e');
        if first == i32::from(b'0') && (self.savenext() | 0x20) == i32::from(b'x') {
            xp = i32::from(b'p');
        }
        let mut prev = first;
        while is_ident(self.c)
            || self.c == i32::from(b'.')
            || ((self.c == i32::from(b'-') || self.c == i32::from(b'+')) && (prev | 0x20) == xp)
        {
            prev = self.c;
            self.savenext();
        }
        let scanned = strscan(&self.sb, true);
        self.save(0);
        match scanned {
            Some(Scanned::Num(n)) => Ok(TokVal::Num(n)),
            Some(_) => Ok(TokVal::Cdata),
            None => Err(self.error(TK_NUMBER, "malformed number")),
        }
    }

    fn skipeq(&mut self) -> i64 {
        let mut count: i64 = 0;
        let s = self.c;
        while self.savenext() == i32::from(b'=') && count < 0x2000_0000 {
            count += 1;
        }
        if self.c == s { count } else { -count - 1 }
    }

    fn longstring(&mut self, is_str: bool, sep: i64) -> R<TokVal> {
        self.savenext();
        if self.iseol() {
            self.newline();
        }
        loop {
            match self.c {
                LEX_EOF => {
                    let msg = if is_str { "unfinished long string" } else { "unfinished long comment" };
                    return Err(self.error(TK_EOF, msg));
                }
                c if c == i32::from(b']') => {
                    if self.skipeq() == sep {
                        self.savenext();
                        break;
                    }
                }
                c if c == i32::from(b'\n') || c == i32::from(b'\r') => {
                    self.save(i32::from(b'\n'));
                    self.newline();
                    if !is_str {
                        self.sb.clear();
                    }
                }
                _ => {
                    self.savenext();
                }
            }
        }
        if !is_str {
            return Ok(TokVal::None);
        }
        let skip = usize::try_from(2 + sep).unwrap_or(0);
        let end = self.sb.len().saturating_sub(skip);
        Ok(TokVal::Str(self.sb.get(skip..end).unwrap_or(&[]).to_vec()))
    }

    fn string(&mut self) -> R<TokVal> {
        let delim = self.c;
        self.savenext();
        while self.c != delim {
            match self.c {
                LEX_EOF => return Err(self.error(TK_EOF, "unfinished string")),
                c if c == i32::from(b'\n') || c == i32::from(b'\r') => {
                    return Err(self.error(TK_STRING, "unfinished string"));
                }
                c if c == i32::from(b'\\') => {
                    let esc = self.next();
                    let mut c: i32 = match u8::try_from(esc).unwrap_or(0) {
                        b'a' => 7,
                        b'b' => 8,
                        b'f' => 12,
                        b'n' => 10,
                        b'r' => 13,
                        b't' => 9,
                        b'v' => 11,
                        b'x' if esc >= 0 => {
                            let mut v = ((self.next() as u32) & 15) << 4;
                            if !is_digit(self.c) {
                                if !is_xdigit(self.c) {
                                    return Err(self.error(TK_STRING, "invalid escape sequence"));
                                }
                                v += 9 << 4;
                            }
                            v += (self.next() as u32) & 15;
                            if !is_digit(self.c) {
                                if !is_xdigit(self.c) {
                                    return Err(self.error(TK_STRING, "invalid escape sequence"));
                                }
                                v += 9;
                            }
                            v as i32
                        }
                        b'u' if esc >= 0 => {
                            if self.next() != i32::from(b'{') {
                                return Err(self.error(TK_STRING, "invalid escape sequence"));
                            }
                            self.next();
                            let mut v: u32 = 0;
                            loop {
                                v = (v << 4) | ((self.c as u32) & 15);
                                if !is_digit(self.c) {
                                    if !is_xdigit(self.c) {
                                        return Err(self.error(TK_STRING, "invalid escape sequence"));
                                    }
                                    v += 9;
                                }
                                if v >= 0x11_0000 {
                                    return Err(self.error(TK_STRING, "invalid escape sequence"));
                                }
                                if self.next() == i32::from(b'}') {
                                    break;
                                }
                            }
                            if v < 0x800 {
                                if v >= 0x80 {
                                    self.save((0xc0 | (v >> 6)) as i32);
                                    v = 0x80 | (v & 0x3f);
                                }
                            } else {
                                if v >= 0x1_0000 {
                                    self.save((0xf0 | (v >> 18)) as i32);
                                    self.save((0x80 | ((v >> 12) & 0x3f)) as i32);
                                } else {
                                    if (0xd800..0xe000).contains(&v) {
                                        return Err(self.error(TK_STRING, "invalid escape sequence"));
                                    }
                                    self.save((0xe0 | (v >> 12)) as i32);
                                }
                                self.save((0x80 | ((v >> 6) & 0x3f)) as i32);
                                v = 0x80 | (v & 0x3f);
                            }
                            v as i32
                        }
                        b'z' if esc >= 0 => {
                            self.next();
                            while is_space(self.c) {
                                if self.iseol() {
                                    self.newline();
                                } else {
                                    self.next();
                                }
                            }
                            continue;
                        }
                        b'\n' | b'\r' if esc >= 0 => {
                            self.save(i32::from(b'\n'));
                            self.newline();
                            continue;
                        }
                        b'\\' | b'"' | b'\'' if esc >= 0 => esc,
                        _ if esc == LEX_EOF => continue,
                        _ => {
                            if !is_digit(esc) {
                                return Err(self.error(TK_STRING, "invalid escape sequence"));
                            }
                            let mut v = esc - i32::from(b'0');
                            if is_digit(self.next()) {
                                v = v * 10 + (self.c - i32::from(b'0'));
                                if is_digit(self.next()) {
                                    v = v * 10 + (self.c - i32::from(b'0'));
                                    if v > 255 {
                                        return Err(self.error(TK_STRING, "invalid escape sequence"));
                                    }
                                    self.next();
                                }
                            }
                            self.save(v);
                            continue;
                        }
                    };
                    c &= 0xff;
                    self.save(c);
                    self.next();
                }
                _ => {
                    self.savenext();
                }
            }
        }
        self.savenext();
        let end = self.sb.len().saturating_sub(1);
        Ok(TokVal::Str(self.sb.get(1..end).unwrap_or(&[]).to_vec()))
    }

    fn scan(&mut self) -> R<(Tok, TokVal)> {
        self.sb.clear();
        loop {
            self.scan_at = self.here();
            let c = self.c;
            if is_ident(c) {
                if is_digit(c) {
                    let v = self.number()?;
                    return Ok((TK_NUMBER, v));
                }
                loop {
                    self.savenext();
                    if !is_ident(self.c) {
                        break;
                    }
                }
                if let Some(i) =
                    TOKEN_NAMES.iter().take(RESERVED).position(|w| w.as_bytes() == self.sb.as_slice())
                {
                    return Ok((TK_OFS + 1 + i as Tok, TokVal::Str(self.sb.clone())));
                }
                return Ok((TK_NAME, TokVal::Str(self.sb.clone())));
            }
            let Ok(b) = u8::try_from(c) else { return Ok((TK_EOF, TokVal::None)) };
            match b {
                b'\n' | b'\r' => {
                    self.newline();
                }
                b' ' | b'\t' | 0x0b | 0x0c => {
                    self.next();
                }
                b'-' => {
                    self.next();
                    if self.c != i32::from(b'-') {
                        return Ok((Tok::from(b'-'), TokVal::None));
                    }
                    self.next();
                    if self.c == i32::from(b'[') {
                        let sep = self.skipeq();
                        self.sb.clear();
                        if sep >= 0 {
                            self.longstring(false, sep)?;
                            self.sb.clear();
                            continue;
                        }
                    }
                    while !self.iseol() && self.c != LEX_EOF {
                        self.next();
                    }
                }
                b'/' => {
                    self.next();
                    if self.c == i32::from(b'/') {
                        while !self.iseol() && self.c != LEX_EOF {
                            self.next();
                        }
                        continue;
                    } else if self.c == i32::from(b'*') {
                        // this LuaJIT counts no line inside /* */ and swallows the
                        // character after a '*' that does not close the comment
                        self.next();
                        while self.c != LEX_EOF {
                            if self.c == i32::from(b'*') {
                                self.next();
                                if self.c == i32::from(b'/') {
                                    self.next();
                                    break;
                                }
                            }
                            self.next();
                        }
                        continue;
                    }
                    return Ok((Tok::from(b'/'), TokVal::None));
                }
                b'[' => {
                    let sep = self.skipeq();
                    if sep >= 0 {
                        let v = self.longstring(true, sep)?;
                        return Ok((TK_STRING, v));
                    } else if sep == -1 {
                        return Ok((Tok::from(b'['), TokVal::None));
                    }
                    return Err(self.error(TK_STRING, "invalid long string delimiter"));
                }
                b'=' | b'<' | b'>' | b'~' | b':' => {
                    self.next();
                    let (second, two) = match b {
                        b'=' => (b'=', TK_EQ),
                        b'<' => (b'=', TK_LE),
                        b'>' => (b'=', TK_GE),
                        b'~' => (b'=', TK_NE),
                        _ => (b':', TK_LABEL),
                    };
                    if self.c != i32::from(second) {
                        return Ok((Tok::from(b), TokVal::None));
                    }
                    self.next();
                    return Ok((two, TokVal::None));
                }
                b'"' | b'\'' => {
                    let v = self.string()?;
                    return Ok((TK_STRING, v));
                }
                b'.' => {
                    if self.savenext() == i32::from(b'.') {
                        self.next();
                        if self.c == i32::from(b'.') {
                            self.next();
                            return Ok((TK_DOTS, TokVal::None));
                        }
                        return Ok((TK_CONCAT, TokVal::None));
                    } else if !is_digit(self.c) {
                        return Ok((Tok::from(b'.'), TokVal::None));
                    }
                    let v = self.number()?;
                    return Ok((TK_NUMBER, v));
                }
                _ => {
                    self.next();
                    return Ok((Tok::from(b), TokVal::None));
                }
            }
        }
    }

    /// `lj_lex_next`.
    pub fn advance(&mut self) -> R<()> {
        self.lastline = self.line;
        if self.lookahead == TK_EOF {
            let (t, v) = self.scan_marked()?;
            self.tok = t;
            self.tokval = v;
            self.tok_at = self.scan_at;
        } else {
            self.tok = self.lookahead;
            self.lookahead = TK_EOF;
            self.tokval = std::mem::take(&mut self.lookaheadval);
            self.tok_at = self.look_at;
        }
        Ok(())
    }

    /// `lj_lex_lookahead`.
    pub fn peek(&mut self) -> R<Tok> {
        let (t, v) = self.scan_marked()?;
        self.lookahead = t;
        self.lookaheadval = v;
        self.look_at = self.scan_at;
        Ok(t)
    }

    /// A scan whose errors point at the token being scanned.
    fn scan_marked(&mut self) -> R<(Tok, TokVal)> {
        self.scanning = true;
        let r = self.scan();
        self.scanning = false;
        r
    }
}

/// Decodes one Lua string literal (`"..."`, `'...'` or a long bracket) the way the reader does.
/// `None` when the text is not exactly one string literal.
#[must_use]
pub fn unescape(literal: &str) -> Option<Vec<u8>> {
    let mut lx = Lexer::new(literal.as_bytes());
    lx.next();
    lx.advance().ok()?;
    if lx.tok != TK_STRING {
        return None;
    }
    let TokVal::Str(s) = std::mem::take(&mut lx.tokval) else { return None };
    lx.advance().ok()?;
    (lx.tok == TK_EOF).then_some(s)
}
