//! What the syntax tree means: the values of expressions and which entry of a table gives the
//! value of each key. The rules are the interpreter's, as the reader models them: a constant
//! field goes into the constructor's template, every other field is stored afterwards in the
//! order it is written, so a computed value wins over a constant one for the same key; a
//! negation of a non-zero number and a difference of two numbers are folded into constants.

use std::borrow::Cow;

use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use super::cst::{ExprN, KeyN, Lex, PErr, Tk, Token, Tree};
use crate::num::{Scanned, lua_tonumber, strscan};
use crate::value::{Table, Value, key_rank_bytes};

/// A value while it is evaluated.
#[derive(Clone, Debug)]
pub(crate) enum Lv<'s> {
    Nil,
    Bool(bool),
    Num(f64),
    Str(Cow<'s, [u8]>),
    Table(u32),
    /// An FFI number (`1LL`): it compiles, but it is not data.
    Cdata,
}

fn type_name(v: &Lv<'_>) -> &'static str {
    match v {
        Lv::Nil => "nil",
        Lv::Bool(_) => "boolean",
        Lv::Num(_) => "number",
        Lv::Str(_) => "string",
        Lv::Table(_) => "table",
        Lv::Cdata => "userdata",
    }
}

fn arith(at: u32, v: &Lv<'_>) -> PErr {
    PErr {
        at: at as usize,
        msg: format!("code outside strings: attempt to perform arithmetic on a {} value", type_name(v)),
    }
}

/// A key as the table holds it.
#[derive(Clone, Debug)]
pub(crate) enum KeyV<'s> {
    Str(Cow<'s, [u8]>),
    Num(f64),
    /// A boolean, a table or a cdata key: in the table, but not data.
    Drop,
    Nil,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tpl {
    Set,
    Hole,
    Store,
}

struct Field<'s> {
    key: KeyV<'s>,
    tpl: Tpl,
    /// A constant nil: a template field that removes its key.
    nil: bool,
}

/// Which entry gives the value of each key of a table.
#[derive(Debug, Default)]
pub(crate) struct Eff<'s> {
    /// String keys in the canonical order, with their entry.
    pub strs: Vec<(Cow<'s, [u8]>, usize)>,
    /// Numeric keys in ascending order: the array part.
    pub nums: Vec<(f64, usize)>,
}

impl Eff<'_> {
    pub fn key(&self, k: &[u8]) -> Option<usize> {
        self.strs.iter().find(|(s, _)| s.as_ref() == k).map(|(_, i)| *i)
    }
}

pub(crate) struct Ev<'a> {
    pub s: &'a [u8],
    pub tree: &'a Tree,
    pub max_depth: u32,
    pub max_values: u32,
}

fn decode_long(body: &[u8]) -> Cow<'_, [u8]> {
    let body = match body {
        [b'\r', b'\n', rest @ ..] | [b'\n', b'\r', rest @ ..] => rest,
        [b'\r' | b'\n', rest @ ..] => rest,
        _ => body,
    };
    if !body.contains(&b'\r') {
        return Cow::Borrowed(body);
    }
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while let Some(&c) = body.get(i) {
        if c == b'\r' || c == b'\n' {
            out.push(b'\n');
            i += 1;
            if let Some(&d @ (b'\r' | b'\n')) = body.get(i)
                && d != c
            {
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    Cow::Owned(out)
}

impl<'a> Ev<'a> {
    fn bytes(&self, start: u32, end: u32) -> &'a [u8] {
        self.s.get(start as usize..end as usize).unwrap_or(&[])
    }

    /// The table whose `{` is at `open`.
    pub fn table_at(&self, open: u32) -> Option<u32> {
        self.tree.tables.binary_search_by_key(&open, |t| t.open).ok().map(|i| i as u32)
    }

    /// A string literal's bytes.
    pub fn string(&self, t: Token) -> Result<Cow<'a, [u8]>, PErr> {
        let b = self.bytes(t.start, t.end);
        match b.first() {
            Some(b'[') => {
                let level = b.iter().skip(1).take_while(|&&c| c == b'=').count();
                let body = b.get(level + 2..b.len().saturating_sub(level + 2)).unwrap_or(&[]);
                Ok(decode_long(body))
            }
            _ => {
                let inner = b.get(1..b.len().saturating_sub(1)).unwrap_or(&[]);
                if !inner.contains(&b'\\') {
                    return Ok(Cow::Borrowed(inner));
                }
                std::str::from_utf8(b).ok().and_then(crate::lex::unescape).map(Cow::Owned).ok_or_else(|| {
                    PErr {
                        at: t.start as usize,
                        msg: format!("invalid escape sequence near '{}'", String::from_utf8_lossy(b)),
                    }
                })
            }
        }
    }

    fn literal(&self, t: Token) -> Result<(Lv<'a>, bool), PErr> {
        Ok(match t.kind {
            Tk::Num => match strscan(self.bytes(t.start, t.end), true) {
                Some(Scanned::Num(n)) => (Lv::Num(n), true),
                Some(_) => (Lv::Cdata, false),
                None => {
                    return Err(PErr {
                        at: t.start as usize,
                        msg: format!(
                            "malformed number near '{}'",
                            String::from_utf8_lossy(self.bytes(t.start, t.end))
                        ),
                    });
                }
            },
            Tk::Str => (Lv::Str(self.string(t)?), true),
            Tk::Name => match self.bytes(t.start, t.end) {
                b"true" => (Lv::Bool(true), true),
                b"false" => (Lv::Bool(false), true),
                _ => (Lv::Nil, true),
            },
            _ => (Lv::Nil, true),
        })
    }

    /// The value of an expression and whether the compiler makes a constant of it.
    pub fn scalar(&self, e: ExprN) -> Result<(Lv<'a>, bool), PErr> {
        if let Some(t) = e.table {
            return Ok((Lv::Table(t), false));
        }
        if let Some(t) = e.token() {
            return self.literal(t);
        }
        let mut lx = Lex::new(self.s, e.start as usize);
        let first = lx.next()?;
        let mut m = Mini { ev: self, lx, tok: first, end: e.end };
        m.expr()
    }

    fn key_of(&self, e: ExprN) -> Result<(KeyV<'a>, bool), PErr> {
        let (v, k) = self.scalar(e)?;
        let key = match v {
            Lv::Num(n) if n.is_nan() => {
                return Ok((KeyV::Num(f64::NAN), k));
            }
            Lv::Num(n) => KeyV::Num(if n == 0.0 { 0.0 } else { n }),
            Lv::Str(s) => KeyV::Str(s),
            Lv::Nil => KeyV::Nil,
            Lv::Bool(_) | Lv::Table(_) | Lv::Cdata => KeyV::Drop,
        };
        Ok((key, k))
    }

    /// Whether an expression is a constant, and whether it is the constant nil, without
    /// decoding what a literal holds.
    fn constness(&self, e: ExprN) -> Result<(bool, bool), PErr> {
        if e.table.is_some() {
            return Ok((false, false));
        }
        if let Some(t) = e.token() {
            return Ok(match t.kind {
                Tk::Num => {
                    let b = self.bytes(t.start, t.end);
                    // only an FFI suffix makes a number literal no constant
                    let cdata = matches!(b.last(), Some(b'l' | b'L' | b'i' | b'I'))
                        && !matches!(strscan(b, true), Some(Scanned::Num(_)));
                    (!cdata, false)
                }
                Tk::Name => (true, self.bytes(t.start, t.end) == b"nil"),
                _ => (true, false),
            });
        }
        let (v, c) = self.scalar(e)?;
        Ok((c, c && matches!(v, Lv::Nil)))
    }

    fn fields(&self, id: u32) -> Result<Vec<Field<'a>>, PErr> {
        let Some(t) = self.tree.tables.get(id as usize) else { return Ok(Vec::new()) };
        let mut narr = 1.0;
        let mut out = Vec::with_capacity(t.entries.len());
        for e in &t.entries {
            let (key, kconst) = match e.key {
                KeyN::Pos => {
                    let k = KeyV::Num(narr);
                    narr += 1.0;
                    (k, true)
                }
                KeyN::Name { start, end } => (KeyV::Str(Cow::Borrowed(self.bytes(start, end))), true),
                KeyN::Bracket { expr, .. } => self.key_of(expr)?,
            };
            let (vconst, nil) = self.constness(e.val)?;
            let tpl = if kconst && !matches!(key, KeyV::Nil) && (matches!(key, KeyV::Str(_)) || vconst) {
                if vconst { Tpl::Set } else { Tpl::Hole }
            } else {
                Tpl::Store
            };
            out.push(Field { key, tpl, nil });
        }
        Ok(out)
    }

    /// Which entry gives each key of table `id`. Fails on a store the interpreter refuses (a
    /// nil or NaN key).
    pub fn effective(&self, id: u32) -> Result<Eff<'a>, PErr> {
        let fields = self.fields(id)?;
        self.effective_of(id, &fields)
    }

    fn effective_of(&self, id: u32, fields: &[Field<'a>]) -> Result<Eff<'a>, PErr> {
        let entries = self.tree.tables.get(id as usize).map_or(&[][..], |t| t.entries.as_slice());
        if entries.iter().all(|e| !matches!(e.key, KeyN::Bracket { .. })) {
            return Ok(Self::effective_plain(fields));
        }
        let mut strs: FxHashMap<Cow<'a, [u8]>, (usize, bool)> = FxHashMap::default();
        let mut nums: FxHashMap<u64, (f64, usize)> = FxHashMap::default();
        for (i, f) in fields.iter().enumerate() {
            match (f.tpl, &f.key) {
                (Tpl::Set, KeyV::Str(k)) => {
                    if f.nil {
                        strs.remove(k);
                    } else {
                        strs.insert(k.clone(), (i, false));
                    }
                }
                (Tpl::Set, KeyV::Num(n)) => {
                    if f.nil {
                        nums.remove(&n.to_bits());
                    } else {
                        nums.insert(n.to_bits(), (*n, i));
                    }
                }
                (Tpl::Hole, KeyV::Str(k)) => {
                    strs.insert(k.clone(), (i, true));
                }
                _ => {}
            }
        }
        strs.retain(|_, (_, hole)| !*hole);
        for (i, f) in fields.iter().enumerate().filter(|(_, f)| f.tpl != Tpl::Set) {
            let at = entries.get(i).map_or(0, |e| e.start as usize);
            match &f.key {
                KeyV::Nil => return Err(PErr { at, msg: "code outside strings: table index is nil".into() }),
                KeyV::Num(n) if n.is_nan() => {
                    return Err(PErr { at, msg: "code outside strings: table index is NaN".into() });
                }
                KeyV::Num(n) => {
                    if f.nil {
                        nums.remove(&n.to_bits());
                    } else {
                        nums.insert(n.to_bits(), (*n, i));
                    }
                }
                KeyV::Str(k) => {
                    if f.nil {
                        strs.remove(k);
                    } else {
                        strs.insert(k.clone(), (i, false));
                    }
                }
                KeyV::Drop => {}
            }
        }
        let s: Vec<(Cow<'a, [u8]>, usize)> = strs.into_iter().map(|(k, (i, _))| (k, i)).collect();
        let mut n: Vec<(f64, usize)> = nums.into_values().collect();
        n.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(Eff { strs: Self::sorted(s), nums: n })
    }

    /// The string keys in the canonical order, each key ranked once.
    fn sorted(s: Vec<(Cow<'a, [u8]>, usize)>) -> Vec<(Cow<'a, [u8]>, usize)> {
        let mut r: Vec<(u32, Cow<'a, [u8]>, usize)> =
            s.into_iter().map(|(k, i)| (key_rank_bytes(&k), k, i)).collect();
        r.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        r.into_iter().map(|(_, k, i)| (k, i)).collect()
    }

    /// A table without `[key]` entries: its positional entries are its array part in order,
    /// and a name takes the value of its last computed entry, else of its last constant one.
    fn effective_plain(fields: &[Field<'a>]) -> Eff<'a> {
        let mut nums = Vec::new();
        // per name: the last constant entry (none after a constant nil) and the last computed one
        type Name<'s> = (Cow<'s, [u8]>, Option<usize>, Option<usize>);
        let mut names: Vec<Name<'a>> = Vec::new();
        let mut index: FxHashMap<Cow<'a, [u8]>, usize> = FxHashMap::default();
        let big = fields.len() > 24;
        for (i, f) in fields.iter().enumerate() {
            match &f.key {
                KeyV::Num(n) => {
                    if !(f.tpl == Tpl::Set && f.nil) {
                        nums.push((*n, i));
                    }
                }
                KeyV::Str(k) => {
                    let found =
                        if big { index.get(k).copied() } else { names.iter().position(|(x, _, _)| x == k) };
                    let at = found.unwrap_or_else(|| {
                        names.push((k.clone(), None, None));
                        if big {
                            index.insert(k.clone(), names.len() - 1);
                        }
                        names.len() - 1
                    });
                    if let Some(slot) = names.get_mut(at) {
                        match f.tpl {
                            Tpl::Set => slot.1 = (!f.nil).then_some(i),
                            _ => slot.2 = Some(i),
                        }
                    }
                }
                _ => {}
            }
        }
        let strs =
            names.into_iter().filter_map(|(k, set, computed)| computed.or(set).map(|i| (k, i))).collect();
        Eff { strs: Self::sorted(strs), nums }
    }

    /// The value of table `id` as data, with the reader's limits.
    pub fn value(&self, id: u32) -> Result<Value, PErr> {
        let (v, steps) = self.run(id, 0)?;
        if steps > self.max_values {
            let at = self.tree.tables.get(id as usize).map_or(0, |t| t.open as usize);
            return Err(PErr { at, msg: "table too large".into() });
        }
        Ok(v)
    }

    fn data(&self, v: Lv<'a>, depth: u32) -> Result<(Value, u32), PErr> {
        Ok(match v {
            Lv::Nil => (Value::Nil, 1),
            Lv::Bool(b) => (Value::Bool(b), 1),
            Lv::Num(n) => (Value::Number(n), 1),
            Lv::Str(s) => (Value::String(SmolStr::new(String::from_utf8_lossy(&s))), 1),
            Lv::Table(t) => self.run(t, depth + 1)?,
            Lv::Cdata => {
                return Err(PErr { at: 0, msg: "value is not plain data (function/userdata)".into() });
            }
        })
    }

    fn run(&self, id: u32, depth: u32) -> Result<(Value, u32), PErr> {
        let Some(t) = self.tree.tables.get(id as usize) else { return Ok((Value::table(), 1)) };
        if depth > self.max_depth {
            return Err(PErr { at: t.open as usize, msg: "table nesting too deep (self reference?)".into() });
        }
        let fields = self.fields(id)?;
        // the stores run in the order they are written, nested constructors first
        let mut stored: Vec<Option<(Value, u32)>> = Vec::new();
        stored.resize_with(t.entries.len(), || None);
        for (i, f) in fields.iter().enumerate().filter(|(_, f)| f.tpl != Tpl::Set) {
            let Some(e) = t.entries.get(i) else { continue };
            if let KeyN::Bracket { expr, .. } = e.key
                && let Some(kt) = expr.table
            {
                self.run(kt, depth + 1)?;
            }
            let v = if let Some(vt) = e.val.table {
                self.run(vt, depth + 1)?
            } else {
                let (v, _) = self.scalar(e.val)?;
                self.data(v, depth).map_err(|mut err| {
                    if err.at == 0 {
                        err.at = e.val.start as usize;
                    }
                    err
                })?
            };
            if !matches!(f.key, KeyV::Drop) {
                stored[i] = Some(v);
            }
        }
        let eff = self.effective_of(id, &fields)?;
        let mut steps = 1u32;
        let mut take = |i: usize, stored: &mut Vec<Option<(Value, u32)>>| -> Result<Value, PErr> {
            let (v, n) = match stored.get_mut(i).and_then(Option::take) {
                Some(x) => x,
                None => {
                    let Some(e) = t.entries.get(i) else { return Ok(Value::Nil) };
                    let (v, _) = self.scalar(e.val)?;
                    self.data(v, depth).map_err(|mut err| {
                        if err.at == 0 {
                            err.at = e.val.start as usize;
                        }
                        err
                    })?
                }
            };
            steps = steps.saturating_add(n);
            Ok(v)
        };
        let mut out =
            Table { arr: Vec::with_capacity(eff.nums.len()), fields: Vec::with_capacity(eff.strs.len()) };
        for &(_, i) in &eff.nums {
            out.arr.push(take(i, &mut stored)?);
        }
        for (k, i) in &eff.strs {
            let v = take(*i, &mut stored)?;
            out.fields.push((SmolStr::new(String::from_utf8_lossy(k)), v));
        }
        Ok((Value::Table(out), steps))
    }
}

/// The evaluator of an expression of more than one token: unary and binary minus over
/// literals, as the compiler folds them and the interpreter runs them.
struct Mini<'e, 'a> {
    ev: &'e Ev<'a>,
    lx: Lex<'a>,
    tok: Token,
    end: u32,
}

impl<'a> Mini<'_, 'a> {
    fn advance(&mut self) -> Result<(), PErr> {
        self.tok = self.lx.next()?;
        if self.tok.start >= self.end {
            self.tok = Token { kind: Tk::Eof, start: self.end, end: self.end };
        }
        Ok(())
    }

    fn primary(&mut self) -> Result<(Lv<'a>, bool), PErr> {
        if self.tok.kind == Tk::LBrace {
            let id = self.ev.table_at(self.tok.start).unwrap_or(0);
            let close = self.ev.tree.tables.get(id as usize).map_or(self.tok.start, |t| t.close);
            self.lx.p = close as usize + 1;
            self.advance()?;
            return Ok((Lv::Table(id), false));
        }
        let v = self.ev.literal(self.tok)?;
        self.advance()?;
        Ok(v)
    }

    fn tonumber(v: &Lv<'_>) -> Option<f64> {
        match v {
            Lv::Num(n) => Some(*n),
            Lv::Str(s) => lua_tonumber(s),
            _ => None,
        }
    }

    fn unary(&mut self) -> Result<(Lv<'a>, bool), PErr> {
        if self.tok.kind != Tk::Minus {
            return self.primary();
        }
        let at = self.tok.start;
        self.advance()?;
        let (x, k) = self.unary()?;
        Ok(match x {
            Lv::Num(n) if k && n != 0.0 => (Lv::Num(-n), true),
            Lv::Cdata => (Lv::Cdata, false),
            v => match Self::tonumber(&v) {
                Some(n) => (Lv::Num(-n), false),
                None => return Err(arith(at, &v)),
            },
        })
    }

    fn expr(&mut self) -> Result<(Lv<'a>, bool), PErr> {
        let mut a = self.unary()?;
        while self.tok.kind == Tk::Minus {
            let at = self.tok.start;
            self.advance()?;
            let b = self.unary()?;
            if let ((Lv::Num(x), true), (Lv::Num(y), true)) = (&a, &b) {
                let r = x - y;
                if !(r.is_nan() || (r == 0.0 && r.is_sign_negative())) {
                    a = (Lv::Num(r), true);
                    continue;
                }
            }
            let x = Self::tonumber(&a.0).ok_or_else(|| arith(at, &a.0))?;
            let y = Self::tonumber(&b.0).ok_or_else(|| arith(at, &b.0))?;
            a = (Lv::Num(x - y), false);
        }
        Ok(a)
    }
}
