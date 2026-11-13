//! The run step of the reader and its walk of the returned table.
//!
//! Only a file the declarative scan let through is run, so what runs is a `return` of table
//! constructors, literals and minus signs. The run reproduces what LuaJIT's interpreter does
//! with the compiled chunk: constant fields go into the constructor's template, the others are
//! stored at run time afterwards - so a computed value wins over a constant one for the same
//! key whatever their order - strings in arithmetic are read as numbers, and the first
//! operation that fails ends the run with the interpreter's message and line.

use std::cmp::Ordering;

use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::num::lua_tonumber;
use crate::parse::{Compiled, DNode, Tpl};
use crate::value::{Table, Value, key_cmp_bytes};

/// The count hook fires at the 4,000,000th instruction (`kHookPeriod * kHookLimit`); the
/// function header is the first one executed.
const HOOK_PC: u32 = 4_000_000 - 1;
const MAX_DEPTH: u32 = 48;
const MAX_STEPS: u32 = 200_000;

#[derive(Debug)]
enum LVal {
    Nil,
    Bool(bool),
    Num(f64),
    Str(u32),
    Table(Box<LTable>),
    /// Anything a declarative file cannot build: a function, a cdata number.
    Other,
}

#[derive(Debug, Default)]
struct LTable {
    num: FxHashMap<u64, (f64, LVal)>,
    str: FxHashMap<u32, LVal>,
}

impl LTable {
    fn set(&mut self, key: &LVal, val: LVal) {
        let present = !matches!(val, LVal::Nil);
        match key {
            LVal::Num(n) => {
                let n = if *n == 0.0 { 0.0 } else { *n };
                if present {
                    self.num.insert(n.to_bits(), (n, val));
                } else {
                    self.num.remove(&n.to_bits());
                }
            }
            LVal::Str(s) => {
                if present {
                    self.str.insert(*s, val);
                } else {
                    self.str.remove(s);
                }
            }
            // booleans and tables are keys of the table, but not data: the walk drops them
            _ => {}
        }
    }
}

/// A run-time error: where it happened and the interpreter's message.
pub(crate) struct RtErr {
    pub pc: u32,
    pub line: u32,
    pub msg: String,
}

pub(crate) enum RunErr {
    /// An operation failed: `name:line: message` follows.
    At { line: u32, msg: String },
    /// The count hook stopped the run; its message carries no location.
    Limit,
}

fn type_name(v: &LVal) -> &'static str {
    match v {
        LVal::Nil => "nil",
        LVal::Bool(_) => "boolean",
        LVal::Num(_) => "number",
        LVal::Str(_) => "string",
        LVal::Table(_) => "table",
        LVal::Other => "userdata",
    }
}

struct Run<'a> {
    c: &'a Compiled,
}

impl Run<'_> {
    fn node(&self, id: u32) -> &DNode {
        self.c.nodes.get(id as usize).unwrap_or(&DNode::Other)
    }

    fn tonumber(&self, v: &LVal) -> Option<f64> {
        match v {
            LVal::Num(n) => Some(*n),
            LVal::Str(s) => lua_tonumber(self.c.syms.get(*s as usize).map_or(&[][..], Vec::as_slice)),
            _ => None,
        }
    }

    fn arith_err(pc: u32, line: u32, v: &LVal) -> RtErr {
        RtErr { pc, line, msg: format!("attempt to perform arithmetic on a {} value", type_name(v)) }
    }

    /// The value a constant node stands for; `None` for a node that runs code.
    fn constant(&self, id: u32) -> Option<LVal> {
        Some(match self.node(id) {
            DNode::Nil => LVal::Nil,
            DNode::Bool(b) => LVal::Bool(*b),
            DNode::Num(n) => LVal::Num(*n),
            DNode::Str(s) => LVal::Str(*s),
            _ => return None,
        })
    }

    fn eval(&self, id: u32) -> Result<LVal, RtErr> {
        if let Some(v) = self.constant(id) {
            return Ok(v);
        }
        match self.node(id) {
            DNode::Unm { a, line, pc } => {
                let x = self.eval(*a)?;
                match self.tonumber(&x) {
                    Some(n) => Ok(LVal::Num(-n)),
                    None => Err(Self::arith_err(*pc, *line, &x)),
                }
            }
            DNode::Sub { a, b, line, pc } => {
                let x = self.eval(*a)?;
                let y = self.eval(*b)?;
                match (self.tonumber(&x), self.tonumber(&y)) {
                    (Some(p), Some(q)) => Ok(LVal::Num(p - q)),
                    (None, _) => Err(Self::arith_err(*pc, *line, &x)),
                    (Some(_), None) => Err(Self::arith_err(*pc, *line, &y)),
                }
            }
            DNode::Table(fields) => {
                let mut t = LTable::default();
                // the template the compiler built, dummy slots included
                let mut holes: Vec<LVal> = Vec::new();
                for f in fields {
                    match f.tpl {
                        Tpl::Set => {
                            if let (Some(k), Some(v)) = (self.constant(f.key), self.constant(f.val)) {
                                t.set(&k, v);
                            }
                        }
                        Tpl::Hole => {
                            if let Some(k) = self.constant(f.key) {
                                t.set(&k, LVal::Other);
                                holes.push(k);
                            }
                        }
                        Tpl::Store => {}
                    }
                }
                for k in &holes {
                    if let LVal::Str(s) = k
                        && matches!(t.str.get(s), Some(LVal::Other))
                    {
                        t.str.remove(s);
                    }
                }
                // then the stores, in the order they were compiled
                for f in fields.iter().filter(|f| f.tpl != Tpl::Set) {
                    let k = self.eval(f.key)?;
                    let v = self.eval(f.val)?;
                    match &k {
                        LVal::Nil => {
                            return Err(RtErr { pc: f.pc, line: f.line, msg: "table index is nil".into() });
                        }
                        LVal::Num(n) if n.is_nan() => {
                            return Err(RtErr { pc: f.pc, line: f.line, msg: "table index is NaN".into() });
                        }
                        _ => {}
                    }
                    t.set(&k, v);
                }
                Ok(LVal::Table(Box::new(t)))
            }
            _ => Ok(LVal::Other),
        }
    }

    fn walk(&self, v: &LVal, depth: u32, steps: &mut u32) -> Result<Value, &'static str> {
        *steps += 1;
        if *steps > MAX_STEPS {
            return Err("table too large");
        }
        Ok(match v {
            LVal::Nil => Value::Nil,
            LVal::Bool(b) => Value::Bool(*b),
            LVal::Num(n) => Value::Number(*n),
            LVal::Str(s) => Value::String(self.text(*s)),
            LVal::Table(t) => {
                if depth > MAX_DEPTH {
                    return Err("table nesting too deep (self reference?)");
                }
                let mut nums: Vec<&(f64, LVal)> = t.num.values().collect();
                nums.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
                let empty: &[u8] = &[];
                let bytes = |s: u32| self.c.syms.get(s as usize).map_or(empty, Vec::as_slice);
                let mut strs: Vec<(&u32, &LVal)> = t.str.iter().collect();
                strs.sort_by(|a, b| key_cmp_bytes(bytes(*a.0), bytes(*b.0)));
                let mut out =
                    Table { arr: Vec::with_capacity(nums.len()), fields: Vec::with_capacity(strs.len()) };
                for (_, x) in nums {
                    out.arr.push(self.walk(x, depth + 1, steps)?);
                }
                for (k, x) in strs {
                    let val = self.walk(x, depth + 1, steps)?;
                    out.fields.push((self.text(*k), val));
                }
                Value::Table(out)
            }
            LVal::Other => return Err("value is not plain data (function/userdata)"),
        })
    }

    fn text(&self, s: u32) -> SmolStr {
        // a Lua string is bytes; one that is not UTF-8 (a "\xff" escape) is kept lossily
        let b = self.c.syms.get(s as usize).map_or(&[][..], Vec::as_slice);
        SmolStr::new(String::from_utf8_lossy(b))
    }
}

/// What the run made of the chunk.
pub(crate) enum Outcome {
    Value(Value),
    /// The chunk did not return exactly one table.
    NotOneTable,
    /// The walk refused the table.
    Walk(&'static str),
}

pub(crate) fn run(c: &Compiled) -> Result<Outcome, RunErr> {
    let r = Run { c };
    let rets = c.ret.as_deref().unwrap_or(&[]);
    let mut values = Vec::with_capacity(rets.len());
    for &id in rets {
        match r.eval(id) {
            Ok(v) => values.push(v),
            Err(e) if e.pc >= HOOK_PC => return Err(RunErr::Limit),
            Err(e) => return Err(RunErr::At { line: e.line, msg: e.msg }),
        }
    }
    if c.total_pc > HOOK_PC {
        return Err(RunErr::Limit);
    }
    let [only] = values.as_slice() else { return Ok(Outcome::NotOneTable) };
    if !matches!(only, LVal::Table(_)) {
        return Ok(Outcome::NotOneTable);
    }
    let mut steps = 0;
    Ok(match r.walk(only, 0, &mut steps) {
        Ok(v) => Outcome::Value(v),
        Err(m) => Outcome::Walk(m),
    })
}
