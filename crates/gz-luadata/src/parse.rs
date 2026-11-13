//! The compile step of the reader: LuaJIT's parser (`lj_parse.c`), ported for what a
//! compile can say - every syntax message, the scope and goto checks, the nesting, local,
//! upvalue and register limits - without the bytecode itself.
//!
//! The register allocator is kept because `function or expression too complex` depends on it;
//! the instruction stream is kept as one tag per instruction, which is all the allocator, the
//! goto lines and the run step need. On the way it records the data a declarative file builds
//! (`DNode`), with the line and instruction index of every operation that can fail at run time,
//! so `eval` can run it the way the interpreter would.

use rustc_hash::FxHashMap;

use crate::lex::*;

const MAX_XLEVEL: u32 = 200;
const MAX_SLOTS: u32 = 250;
const MAX_LOCVAR: u32 = 200;
const MAX_UPVAL: u32 = 60;
const MAX_VSTACK: usize = 65536 - 60;
const NO_REG: u32 = 255;
const BCMAX_C: u32 = 255;
const BCMAX_D: u32 = 0xffff;

const PROTO_CHILD: u8 = 0x01;
const PROTO_VARARG: u8 = 0x02;
const PROTO_HAS_RETURN: u8 = 0x20;
const PROTO_FIXUP_RETURN: u8 = 0x40;

const FSCOPE_LOOP: u8 = 0x01;
const FSCOPE_BREAK: u8 = 0x02;
const FSCOPE_GOLA: u8 = 0x04;
const FSCOPE_UPVAL: u8 = 0x08;
const FSCOPE_NOCLOSE: u8 = 0x10;

const VSTACK_GOTO: u8 = 0x02;
const VSTACK_LABEL: u8 = 0x04;

// LuaJIT's expression kinds (VKNIL, VKFALSE, ... VLOCAL): the constant kinds keep their K, the
// order of the variants is LuaJIT's and the comparisons depend on it
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum K {
    KNil,
    KFalse,
    KTrue,
    KStr,
    KNum,
    KCdata,
    Local,
    Upval,
    Global,
    Indexed,
    Jmp,
    Relocable,
    NonReloc,
    Call,
    Void,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Idx {
    Reg(u32),
    Byte,
    Str,
}

/// A jump list: `None` is `NO_JMP`; `Some(novalue)` says whether one of its jumps produces no
/// value (`jmp_novalue`).
type Jl = Option<bool>;

fn jl_append(a: &mut Jl, b: Jl) {
    if let Some(nb) = b {
        *a = Some(a.unwrap_or(false) || nb);
    }
}

#[derive(Clone, Copy, Debug)]
struct Exp {
    k: K,
    info: u32,
    aux: u32,
    idx: Idx,
    t: Jl,
    f: Jl,
    num: f64,
    sym: u32,
    /// The data node this expression builds.
    d: u32,
}

impl Exp {
    fn new(k: K, info: u32) -> Self {
        Self { k, info, aux: 0, idx: Idx::Byte, t: None, f: None, num: 0.0, sym: 0, d: 0 }
    }
    fn hasjump(&self) -> bool {
        self.t.is_some() || self.f.is_some()
    }
    fn isk(&self) -> bool {
        self.k <= K::KNum
    }
    fn isk_nojump(&self) -> bool {
        self.isk() && !self.hasjump()
    }
    fn isnumk(&self) -> bool {
        self.k == K::KNum
    }
    fn isnumk_nojump(&self) -> bool {
        self.isnumk() && !self.hasjump()
    }
}

/// One instruction, as far as the parser looks back at it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Uclo,
    Jmp,
    Ret,
    Call,
    Varg,
    Not,
    Cat,
    KPriNil(u32),
    KNilRange(u32, u32),
    Tnew,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VName {
    Null,
    Break,
    Fixed,
    Sym(u32),
}

#[derive(Clone, Copy, Debug)]
struct VarInfo {
    name: VName,
    startpc: u32,
    slot: u32,
    info: u8,
}

#[derive(Clone, Copy, Debug)]
struct Scope {
    vstart: usize,
    nactvar: u32,
    flags: u8,
}

#[derive(Default)]
struct FuncState {
    lasttarget: u32,
    freereg: u32,
    nactvar: u32,
    framesize: u32,
    linedefined: u32,
    vbase: usize,
    flags: u8,
    numparams: u32,
    varmap: Vec<u32>,
    uvmap: Vec<u32>,
    bl: Vec<Scope>,
    ops: Vec<Op>,
    lines: Vec<u32>,
    kstr: FxHashMap<u32, u32>,
    nkgc: u32,
    knum: FxHashMap<u64, u32>,
    nkn: u32,
}

impl FuncState {
    fn pc(&self) -> u32 {
        u32::try_from(self.ops.len()).unwrap_or(u32::MAX)
    }
    fn op(&self, pc: u32) -> Op {
        self.ops.get(pc as usize).copied().unwrap_or(Op::Other)
    }
    fn set_op(&mut self, pc: u32, op: Op) {
        if let Some(o) = self.ops.get_mut(pc as usize) {
            *o = op;
        }
    }
    fn varmap_at(&self, reg: u32) -> usize {
        self.varmap.get(reg as usize).copied().unwrap_or(0) as usize
    }
    fn set_varmap(&mut self, reg: u32, v: usize) {
        let reg = reg as usize;
        if self.varmap.len() <= reg {
            self.varmap.resize(reg + 1, 0);
        }
        if let Some(slot) = self.varmap.get_mut(reg) {
            *slot = u32::try_from(v).unwrap_or(0);
        }
    }
    fn bl_flags_or(&mut self, flags: u8) {
        if let Some(b) = self.bl.last_mut() {
            b.flags |= flags;
        }
    }
}

/// What a declarative file builds, recorded while it compiles.
#[derive(Clone, Debug)]
pub(crate) enum DNode {
    Other,
    Nil,
    Bool(bool),
    Num(f64),
    Str(u32),
    Table(Vec<DField>),
    Unm { a: u32, line: u32, pc: u32 },
    Sub { a: u32, b: u32, line: u32, pc: u32 },
}

/// How a constructor field reaches the table: in the template the compiler builds
/// (`Set`), or by a store at run time - with a dummy slot in the template first (`Hole`) or
/// without (`Store`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tpl {
    Set,
    Hole,
    Store,
}

#[derive(Clone, Debug)]
pub(crate) struct DField {
    pub key: u32,
    pub val: u32,
    pub tpl: Tpl,
    pub line: u32,
    pub pc: u32,
}

/// The compiled chunk: its data and the instruction count of the main function.
pub(crate) struct Compiled {
    pub nodes: Vec<DNode>,
    pub syms: Vec<Vec<u8>>,
    /// The values `return` hands back, `None` when the chunk does not end in one.
    pub ret: Option<Vec<u32>>,
    pub total_pc: u32,
}

pub(crate) struct Parser<'a> {
    ls: Lexer<'a>,
    cur: FuncState,
    outer: Vec<FuncState>,
    vstack: Vec<VarInfo>,
    level: u32,
    syms: Vec<Vec<u8>>,
    symmap: FxHashMap<Vec<u8>, u32>,
    nodes: Vec<DNode>,
    ret: Option<Vec<u32>>,
}

/// Compiles `src` as LuaJIT's `luaL_loadbuffer` does. `Ok(None)`: the text is a bytecode dump.
pub(crate) fn compile(src: &[u8]) -> R<Option<Compiled>> {
    let mut p = Parser {
        ls: Lexer::new(src),
        cur: FuncState::default(),
        outer: Vec::new(),
        vstack: Vec::new(),
        level: 0,
        syms: Vec::new(),
        symmap: FxHashMap::default(),
        nodes: vec![DNode::Other],
        ret: None,
    };
    if p.ls.setup()? {
        return Ok(None);
    }
    let total_pc = p.run()?;
    Ok(Some(Compiled { nodes: p.nodes, syms: p.syms, ret: p.ret, total_pc }))
}

fn fmt_limit_main(limit: u32, what: &str) -> String {
    format!("main function has more than {limit} {what}")
}

impl Parser<'_> {
    // -- small helpers ------------------------------------------------------------------------

    fn sym(&mut self, s: &[u8]) -> u32 {
        if let Some(&id) = self.symmap.get(s) {
            return id;
        }
        let id = u32::try_from(self.syms.len()).unwrap_or(u32::MAX);
        self.syms.push(s.to_vec());
        self.symmap.insert(s.to_vec(), id);
        id
    }

    fn sym_bytes(&self, id: u32) -> Vec<u8> {
        self.syms.get(id as usize).cloned().unwrap_or_default()
    }

    fn node(&mut self, n: DNode) -> u32 {
        let id = u32::try_from(self.nodes.len()).unwrap_or(0);
        self.nodes.push(n);
        id
    }

    fn vinfo(&self, i: usize) -> VarInfo {
        self.vstack.get(i).copied().unwrap_or(VarInfo { name: VName::Null, startpc: 0, slot: 0, info: 0 })
    }

    fn vinfo_mut(&mut self, i: usize) -> Option<&mut VarInfo> {
        self.vstack.get_mut(i)
    }

    fn err_syntax(&self, msg: &str) -> SynErr {
        self.ls.error(self.ls.tok, msg)
    }

    fn err_token(&self, tok: Tok) -> SynErr {
        let mut m = b"'".to_vec();
        m.extend(Lexer::token2str(tok));
        m.extend(b"' expected");
        self.ls.error(self.ls.tok, m)
    }

    fn err_limit(&self, linedefined: u32, limit: u32, what: &str) -> SynErr {
        if linedefined == 0 {
            self.ls.error(0, fmt_limit_main(limit, what))
        } else {
            self.ls.error(0, format!("function at line {linedefined} has more than {limit} {what}"))
        }
    }

    // -- constants ----------------------------------------------------------------------------

    fn const_num(&mut self, n: f64) -> u32 {
        let n = if n == 0.0 { 0.0 } else { n };
        let fs = &mut self.cur;
        let next = fs.nkn;
        let id = *fs.knum.entry(n.to_bits()).or_insert(next);
        if id == next {
            fs.nkn += 1;
        }
        id
    }

    fn const_str(&mut self, sym: u32) -> u32 {
        let fs = &mut self.cur;
        let next = fs.nkgc;
        let id = *fs.kstr.entry(sym).or_insert(next);
        if id == next {
            fs.nkgc += 1;
        }
        id
    }

    fn const_gc_new(&mut self) -> u32 {
        let id = self.cur.nkgc;
        self.cur.nkgc += 1;
        id
    }

    // -- registers and instructions -----------------------------------------------------------

    fn bcreg_bump(&mut self, n: u32) -> R<()> {
        let sz = self.cur.freereg + n;
        if sz > self.cur.framesize {
            if sz >= MAX_SLOTS {
                return Err(self.err_syntax("function or expression too complex"));
            }
            self.cur.framesize = sz;
        }
        Ok(())
    }

    fn bcreg_reserve(&mut self, n: u32) -> R<()> {
        self.bcreg_bump(n)?;
        self.cur.freereg += n;
        Ok(())
    }

    fn bcreg_free(&mut self, reg: u32) {
        if reg >= self.cur.nactvar {
            self.cur.freereg = self.cur.freereg.wrapping_sub(1);
        }
    }

    fn expr_free(&mut self, e: &Exp) {
        if e.k == K::NonReloc {
            self.bcreg_free(e.info);
        }
    }

    fn emit(&mut self, op: Op) -> u32 {
        let pc = self.cur.pc();
        let line = self.ls.lastline;
        self.cur.ops.push(op);
        self.cur.lines.push(line);
        pc
    }

    fn bcemit_nil(&mut self, from: u32, n: u32) {
        let fs = &mut self.cur;
        let pc = fs.pc();
        if pc > fs.lasttarget {
            let last = pc - 1;
            match fs.op(last) {
                Op::KPriNil(pfrom) => {
                    let (mut from, mut n) = (from, n);
                    let merge = if from == pfrom {
                        if n == 1 {
                            return;
                        }
                        true
                    } else if from == pfrom + 1 {
                        from = pfrom;
                        n += 1;
                        true
                    } else {
                        false
                    };
                    if merge {
                        fs.set_op(last, Op::KNilRange(from, from + n - 1));
                        return;
                    }
                }
                Op::KNilRange(pfrom, pto) if pfrom <= from && from <= pto + 1 => {
                    if from + n - 1 > pto {
                        fs.set_op(last, Op::KNilRange(pfrom, from + n - 1));
                    }
                    return;
                }
                _ => {}
            }
        }
        self.emit(if n == 1 { Op::KPriNil(from) } else { Op::KNilRange(from, from + n - 1) });
    }

    fn jmp_tohere(&mut self) {
        self.cur.lasttarget = self.cur.pc();
    }

    fn bcemit_jmp(&mut self) -> u32 {
        let pc = self.cur.pc();
        let j = i64::from(pc) - 1;
        if j >= i64::from(self.cur.lasttarget) && j >= 0 && self.cur.op(j as u32) == Op::Uclo {
            self.cur.lasttarget = pc;
            return j as u32;
        }
        self.emit(Op::Jmp)
    }

    // -- expressions to registers -------------------------------------------------------------

    fn expr_discharge(&mut self, e: &mut Exp) {
        match e.k {
            K::Upval => {}
            K::Global => {
                self.const_str(e.sym);
            }
            K::Indexed => {
                if let Idx::Reg(rc) = e.idx {
                    self.bcreg_free(rc);
                }
                self.bcreg_free(e.info);
            }
            K::Call => {
                e.info = e.aux;
                e.k = K::NonReloc;
                return;
            }
            K::Local => {
                e.k = K::NonReloc;
                return;
            }
            _ => return,
        }
        e.info = self.emit(Op::Other);
        e.k = K::Relocable;
    }

    fn expr_toreg_nobranch(&mut self, e: &mut Exp, reg: u32) {
        self.expr_discharge(e);
        match e.k {
            K::KStr => {
                self.const_str(e.sym);
                self.emit(Op::Other);
            }
            K::KNum => {
                let n = e.num;
                let k = num2int(n);
                if !(i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&k) || n != f64::from(k) {
                    self.const_num(n);
                }
                self.emit(Op::Other);
            }
            K::KCdata => {
                self.const_gc_new();
                self.emit(Op::Other);
            }
            K::Relocable => {}
            K::NonReloc => {
                if reg != e.info {
                    self.emit(Op::Other);
                }
            }
            K::KNil => self.bcemit_nil(reg, 1),
            K::KFalse | K::KTrue => {
                self.emit(Op::Other);
            }
            _ => return,
        }
        e.info = reg;
        e.k = K::NonReloc;
    }

    fn expr_toreg(&mut self, e: &mut Exp, reg: u32) {
        self.expr_toreg_nobranch(e, reg);
        if e.k == K::Jmp {
            jl_append(&mut e.t, Some(true));
        }
        if e.hasjump() {
            if e.t == Some(true) || e.f == Some(true) {
                if e.k != K::Jmp {
                    self.bcemit_jmp();
                }
                self.emit(Op::Other);
                self.emit(Op::Jmp);
                self.emit(Op::Other);
            }
            self.cur.lasttarget = self.cur.pc();
        }
        e.t = None;
        e.f = None;
        e.info = reg;
        e.k = K::NonReloc;
    }

    fn expr_tonextreg(&mut self, e: &mut Exp) -> R<()> {
        self.expr_discharge(e);
        self.expr_free(e);
        self.bcreg_reserve(1)?;
        let reg = self.cur.freereg - 1;
        self.expr_toreg(e, reg);
        Ok(())
    }

    fn expr_toanyreg(&mut self, e: &mut Exp) -> R<u32> {
        self.expr_discharge(e);
        if e.k == K::NonReloc {
            if !e.hasjump() {
                return Ok(e.info);
            }
            if e.info >= self.cur.nactvar {
                let reg = e.info;
                self.expr_toreg(e, reg);
                return Ok(e.info);
            }
        }
        self.expr_tonextreg(e)?;
        Ok(e.info)
    }

    fn expr_toval(&mut self, e: &mut Exp) -> R<()> {
        if e.hasjump() {
            self.expr_toanyreg(e)?;
        } else {
            self.expr_discharge(e);
        }
        Ok(())
    }

    fn bcemit_store(&mut self, var: &Exp, e: &mut Exp) -> R<()> {
        match var.k {
            K::Local => {
                self.expr_free(e);
                self.expr_toreg(e, var.info);
                return Ok(());
            }
            K::Upval => {
                self.expr_toval(e)?;
                match e.k {
                    K::KNil | K::KFalse | K::KTrue => {}
                    K::KStr => {
                        self.const_str(e.sym);
                    }
                    K::KNum => {
                        self.const_num(e.num);
                    }
                    _ => {
                        self.expr_toanyreg(e)?;
                    }
                }
            }
            K::Global => {
                self.expr_toanyreg(e)?;
                self.const_str(var.sym);
            }
            _ => {
                self.expr_toanyreg(e)?;
            }
        }
        self.emit(Op::Other);
        self.expr_free(e);
        Ok(())
    }

    fn bcemit_method(&mut self, e: &mut Exp, key: &Exp) -> R<()> {
        let _obj = self.expr_toanyreg(e)?;
        self.expr_free(e);
        let func = self.cur.freereg;
        self.emit(Op::Other);
        let idx = self.const_str(key.sym);
        if idx <= BCMAX_C {
            self.bcreg_reserve(2)?;
            self.emit(Op::Other);
        } else {
            self.bcreg_reserve(3)?;
            self.emit(Op::Other);
            self.emit(Op::Other);
            self.cur.freereg -= 1;
        }
        e.info = func;
        e.k = K::NonReloc;
        Ok(())
    }

    // -- branches -----------------------------------------------------------------------------

    /// Returns the jump and whether it carries no value.
    fn bcemit_branch(&mut self, e: &mut Exp, _cond: bool) -> R<(u32, bool)> {
        if e.k == K::Relocable && self.cur.op(e.info) == Op::Not {
            self.cur.set_op(e.info, Op::Other);
            return Ok((self.bcemit_jmp(), true));
        }
        if e.k != K::NonReloc {
            self.bcreg_reserve(1)?;
            let reg = self.cur.freereg - 1;
            self.expr_toreg_nobranch(e, reg);
        }
        self.emit(Op::Other);
        let pc = self.bcemit_jmp();
        self.expr_free(e);
        Ok((pc, false))
    }

    fn bcemit_branch_t(&mut self, e: &mut Exp) -> R<()> {
        self.expr_discharge(e);
        let pc: Jl = match e.k {
            K::KStr | K::KNum | K::KTrue => None,
            K::Jmp => Some(true),
            K::KFalse | K::KNil => {
                self.expr_toreg_nobranch(e, NO_REG);
                self.bcemit_jmp();
                Some(false)
            }
            _ => Some(self.bcemit_branch(e, false)?.1),
        };
        jl_append(&mut e.f, pc);
        self.jmp_tohere();
        e.t = None;
        Ok(())
    }

    fn bcemit_branch_f(&mut self, e: &mut Exp) -> R<()> {
        self.expr_discharge(e);
        let pc: Jl = match e.k {
            K::KNil | K::KFalse => None,
            K::Jmp => Some(true),
            K::KStr | K::KNum | K::KTrue => {
                self.expr_toreg_nobranch(e, NO_REG);
                self.bcemit_jmp();
                Some(false)
            }
            _ => Some(self.bcemit_branch(e, true)?.1),
        };
        jl_append(&mut e.t, pc);
        self.jmp_tohere();
        e.f = None;
        Ok(())
    }

    // -- operators ----------------------------------------------------------------------------

    fn foldarith(opr: BinOpr, e1: &mut Exp, e2: &Exp) -> bool {
        if !e1.isnumk_nojump() || !e2.isnumk_nojump() {
            return false;
        }
        let (a, b) = (e1.num, e2.num);
        let n = match opr {
            BinOpr::Add => a + b,
            BinOpr::Sub => a - b,
            BinOpr::Mul => a * b,
            BinOpr::Div => a / b,
            BinOpr::Mod => a - (a / b).floor() * b,
            _ => a.powf(b),
        };
        if n.is_nan() || (n == 0.0 && n.is_sign_negative()) {
            return false;
        }
        e1.num = n;
        true
    }

    fn bcemit_arith(&mut self, opr: BinOpr, e1: &mut Exp, e2: &mut Exp) -> R<()> {
        let (d1, d2) = (e1.d, e2.d);
        if Self::foldarith(opr, e1, e2) {
            e1.d = if opr == BinOpr::Sub { self.node(DNode::Num(e1.num)) } else { 0 };
            return Ok(());
        }
        if opr == BinOpr::Pow {
            self.expr_toanyreg(e2)?;
            self.expr_toanyreg(e1)?;
        } else {
            self.expr_toval(e2)?;
            let e2k = e2.isnumk() && self.const_num(e2.num) <= BCMAX_C;
            if !e2k {
                self.expr_toanyreg(e2)?;
            }
            self.expr_toval(e1)?;
            let e1k = e1.isnumk() && !e2.isnumk() && self.const_num(e1.num) <= 0xff;
            if !e1k {
                self.expr_toanyreg(e1)?;
            }
        }
        if e1.k == K::NonReloc && e1.info >= self.cur.nactvar {
            self.cur.freereg -= 1;
        }
        if e2.k == K::NonReloc && e2.info >= self.cur.nactvar {
            self.cur.freereg -= 1;
        }
        let line = self.ls.lastline;
        let pc = self.emit(Op::Other);
        e1.info = pc;
        e1.k = K::Relocable;
        e1.d = if opr == BinOpr::Sub { self.node(DNode::Sub { a: d1, b: d2, line, pc }) } else { 0 };
        Ok(())
    }

    fn bcemit_comp(&mut self, opr: BinOpr, e1: &mut Exp, e2: &mut Exp) -> R<()> {
        self.expr_toval(e1)?;
        // which of the two ends up where; `ret` is the one that carries the result
        let mut a = *e1;
        let mut b = *e2;
        if matches!(opr, BinOpr::Eq | BinOpr::Ne) {
            if a.isk() {
                std::mem::swap(&mut a, &mut b);
            }
            self.expr_toanyreg(&mut a)?;
            self.expr_toval(&mut b)?;
            match b.k {
                K::KNil | K::KFalse | K::KTrue => {}
                K::KStr => {
                    self.const_str(b.sym);
                }
                K::KNum => {
                    self.const_num(b.num);
                }
                _ => {
                    self.expr_toanyreg(&mut b)?;
                }
            }
        } else if matches!(opr, BinOpr::Ge | BinOpr::Gt) {
            std::mem::swap(&mut a, &mut b);
            self.expr_toval(&mut a)?;
            self.expr_toanyreg(&mut a)?;
            self.expr_toanyreg(&mut b)?;
        } else {
            self.expr_toanyreg(&mut b)?;
            self.expr_toanyreg(&mut a)?;
        }
        if a.k == K::NonReloc && a.info >= self.cur.nactvar {
            self.cur.freereg -= 1;
        }
        if b.k == K::NonReloc && b.info >= self.cur.nactvar {
            self.cur.freereg -= 1;
        }
        self.emit(Op::Other);
        let j = self.bcemit_jmp();
        *e1 = Exp { info: j, k: K::Jmp, d: 0, ..*e1 };
        Ok(())
    }

    fn bcemit_binop_left(&mut self, op: BinOpr, e: &mut Exp) -> R<()> {
        match op {
            BinOpr::And => self.bcemit_branch_t(e)?,
            BinOpr::Or => self.bcemit_branch_f(e)?,
            BinOpr::Concat => self.expr_tonextreg(e)?,
            BinOpr::Eq | BinOpr::Ne => {
                if !e.isk_nojump() {
                    self.expr_toanyreg(e)?;
                }
            }
            _ => {
                if !e.isnumk_nojump() {
                    self.expr_toanyreg(e)?;
                }
            }
        }
        Ok(())
    }

    fn bcemit_binop(&mut self, op: BinOpr, e1: &mut Exp, e2: &mut Exp) -> R<()> {
        match op {
            BinOpr::Add | BinOpr::Sub | BinOpr::Mul | BinOpr::Div | BinOpr::Mod | BinOpr::Pow => {
                self.bcemit_arith(op, e1, e2)?;
            }
            BinOpr::And => {
                self.expr_discharge(e2);
                jl_append(&mut e2.f, e1.f);
                *e1 = Exp { d: 0, ..*e2 };
            }
            BinOpr::Or => {
                self.expr_discharge(e2);
                jl_append(&mut e2.t, e1.t);
                *e1 = Exp { d: 0, ..*e2 };
            }
            BinOpr::Concat => {
                self.expr_toval(e2)?;
                if e2.k == K::Relocable && self.cur.op(e2.info) == Op::Cat {
                    self.expr_free(e1);
                    e1.info = e2.info;
                } else {
                    self.expr_tonextreg(e2)?;
                    self.expr_free(e2);
                    self.expr_free(e1);
                    e1.info = self.emit(Op::Cat);
                }
                e1.k = K::Relocable;
                e1.d = 0;
            }
            _ => self.bcemit_comp(op, e1, e2)?,
        }
        Ok(())
    }

    fn bcemit_unop(&mut self, op: UnOp, e: &mut Exp) -> R<()> {
        match op {
            UnOp::Not => {
                std::mem::swap(&mut e.t, &mut e.f);
                // jmp_dropval: no jump of the lists carries its value any more
                e.t = e.t.map(|_| true);
                e.f = e.f.map(|_| true);
                self.expr_discharge(e);
                e.d = 0;
                if e.k == K::KNil || e.k == K::KFalse {
                    e.k = K::KTrue;
                    return Ok(());
                } else if e.isk() || e.k == K::KCdata {
                    e.k = K::KFalse;
                    return Ok(());
                } else if e.k == K::Jmp {
                    return Ok(());
                } else if e.k == K::Relocable {
                    self.bcreg_reserve(1)?;
                    e.info = self.cur.freereg - 1;
                    e.k = K::NonReloc;
                }
                self.expr_free(e);
                e.info = self.emit(Op::Not);
                e.k = K::Relocable;
            }
            UnOp::Neg | UnOp::Len => {
                if op == UnOp::Neg && !e.hasjump() {
                    if e.k == K::KCdata {
                        return Ok(());
                    }
                    if e.isnumk() && e.num != 0.0 {
                        e.num = -e.num;
                        e.d = self.node(DNode::Num(e.num));
                        return Ok(());
                    }
                }
                let a = e.d;
                self.expr_toanyreg(e)?;
                self.expr_free(e);
                let line = self.ls.lastline;
                e.info = self.emit(Op::Other);
                e.k = K::Relocable;
                e.d = if op == UnOp::Neg { self.node(DNode::Unm { a, line, pc: e.info }) } else { 0 };
            }
        }
        Ok(())
    }

    // -- lexer support ------------------------------------------------------------------------

    fn next(&mut self) -> R<()> {
        self.ls.advance()
    }

    fn lex_opt(&mut self, tok: Tok) -> R<bool> {
        if self.ls.tok == tok {
            self.next()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn lex_check(&mut self, tok: Tok) -> R<()> {
        if self.ls.tok != tok {
            return Err(self.err_token(tok));
        }
        self.next()
    }

    fn lex_match(&mut self, what: Tok, who: Tok, line: u32) -> R<()> {
        if !self.lex_opt(what)? {
            if line == self.ls.line {
                return Err(self.err_token(what));
            }
            let mut m = b"'".to_vec();
            m.extend(Lexer::token2str(what));
            m.extend(b"' expected (to close '");
            m.extend(Lexer::token2str(who));
            m.extend(format!("' at line {line})").into_bytes());
            return Err(self.ls.error(self.ls.tok, m));
        }
        Ok(())
    }

    fn lex_str(&mut self) -> R<u32> {
        if self.ls.tok != TK_NAME && self.ls.tok != TK_GOTO {
            return Err(self.err_token(TK_NAME));
        }
        let s = match &self.ls.tokval {
            TokVal::Str(s) => s.clone(),
            _ => Vec::new(),
        };
        let id = self.sym(&s);
        self.next()?;
        Ok(id)
    }

    // -- variables ----------------------------------------------------------------------------

    fn vtop(&self) -> usize {
        self.vstack.len()
    }

    fn vpush(&mut self, v: VarInfo) -> R<usize> {
        let vtop = self.vtop();
        if vtop >= MAX_VSTACK {
            return Err(self.ls.error(0, format!("chunk has more than {MAX_VSTACK} local variables")));
        }
        self.vstack.push(v);
        Ok(vtop)
    }

    fn var_new(&mut self, n: u32, name: VName) -> R<()> {
        if self.cur.nactvar + n >= MAX_LOCVAR {
            return Err(self.err_limit(self.cur.linedefined, MAX_LOCVAR, "local variables"));
        }
        let vtop = self.vpush(VarInfo { name, startpc: 0, slot: 0, info: 0 })?;
        let reg = self.cur.nactvar + n;
        self.cur.set_varmap(reg, vtop);
        Ok(())
    }

    fn var_add(&mut self, nvars: u32) {
        let pc = self.cur.pc();
        for _ in 0..nvars {
            let slot = self.cur.nactvar;
            let vi = self.cur.varmap_at(slot);
            if let Some(v) = self.vinfo_mut(vi) {
                v.startpc = pc;
                v.slot = slot;
                v.info = 0;
            }
            self.cur.nactvar += 1;
        }
    }

    fn fs_at(&mut self, i: usize) -> &mut FuncState {
        match self.outer.get_mut(i) {
            Some(f) => f,
            None => &mut self.cur,
        }
    }

    fn var_lookup_local(&self, fs: &FuncState, name: u32) -> Option<u32> {
        (0..fs.nactvar).rev().find(|&i| self.vinfo(fs.varmap_at(i)).name == VName::Sym(name))
    }

    /// `var_lookup_`: the variable in function `level` (or an outer one). `Some(vidx)` for a
    /// local or upvalue, `None` for a global.
    fn var_lookup_at(&mut self, level: usize, name: u32, e: &mut Exp, first: bool) -> R<Option<usize>> {
        let reg = {
            let fs: &FuncState = if level < self.outer.len() {
                match self.outer.get(level) {
                    Some(f) => f,
                    None => &self.cur,
                }
            } else {
                &self.cur
            };
            self.var_lookup_local(fs, name)
        };
        if let Some(reg) = reg {
            *e = Exp::new(K::Local, reg);
            if !first {
                let fs = self.fs_at(level);
                if let Some(bl) = fs.bl.iter_mut().rev().find(|b| b.nactvar <= reg) {
                    bl.flags |= FSCOPE_UPVAL;
                }
            }
            let vidx = self.fs_at(level).varmap_at(reg);
            e.aux = u32::try_from(vidx).unwrap_or(0);
            return Ok(Some(vidx));
        }
        if level == 0 {
            *e = Exp::new(K::Global, 0);
            e.sym = name;
            return Ok(None);
        }
        let Some(vidx) = self.var_lookup_at(level - 1, name, e, false)? else { return Ok(None) };
        // var_lookup_uv
        let vidx32 = u32::try_from(vidx).unwrap_or(0);
        let fs = self.fs_at(level);
        let found = fs.uvmap.iter().position(|&v| v == vidx32);
        let uv = match found {
            Some(i) => i,
            None => {
                let (n, linedefined) = (fs.uvmap.len(), fs.linedefined);
                if n >= MAX_UPVAL as usize {
                    return Err(self.err_limit(linedefined, MAX_UPVAL, "upvalues"));
                }
                self.fs_at(level).uvmap.push(vidx32);
                n
            }
        };
        e.info = u32::try_from(uv).unwrap_or(0);
        e.k = K::Upval;
        Ok(Some(vidx))
    }

    fn var_lookup(&mut self, e: &mut Exp) -> R<()> {
        let name = self.lex_str()?;
        let level = self.outer.len();
        self.var_lookup_at(level, name, e, true)?;
        Ok(())
    }

    // -- gotos and labels ---------------------------------------------------------------------

    fn gola_new(&mut self, name: VName, info: u8, pc: u32) -> R<usize> {
        let slot = self.cur.nactvar;
        self.vpush(VarInfo { name, startpc: pc, slot, info })
    }

    fn gola_patch(&mut self, vg: usize, vl: usize) {
        let target = self.vinfo(vl).startpc;
        if let Some(g) = self.vinfo_mut(vg) {
            g.name = VName::Null;
        }
        if target == self.cur.pc() {
            self.jmp_tohere();
        }
    }

    fn gola_close(&mut self, vg: usize) {
        let pc = self.vinfo(vg).startpc;
        if self.cur.op(pc) == Op::Jmp {
            self.cur.set_op(pc, Op::Uclo);
        }
    }

    fn name_text(&self, name: VName) -> Vec<u8> {
        match name {
            VName::Sym(s) => self.sym_bytes(s),
            _ => Vec::new(),
        }
    }

    fn gola_resolve(&mut self, vstart: usize, idx: usize, vl: VarInfo) -> R<()> {
        for vg in vstart..idx {
            let g = self.vinfo(vg);
            if g.name == vl.name && g.info & VSTACK_GOTO != 0 {
                if g.slot < vl.slot {
                    let local = self.vinfo(self.cur.varmap_at(g.slot)).name;
                    self.ls.line = self.cur.lines.get(g.startpc as usize).copied().unwrap_or(0);
                    let mut m = b"<goto ".to_vec();
                    m.extend(self.name_text(g.name));
                    m.extend(b"> jumps into the scope of local '");
                    m.extend(self.name_text(local));
                    m.push(b'\'');
                    return Err(self.ls.error(0, m));
                }
                if let Some(g) = self.vinfo_mut(vg) {
                    g.name = VName::Null;
                }
                if vl.startpc == self.cur.pc() {
                    self.jmp_tohere();
                }
            }
        }
        Ok(())
    }

    /// `gola_fixup` for the scope `bl`, which has already been taken off the scope stack.
    fn gola_fixup(&mut self, bl: Scope) -> R<()> {
        let ve = self.vtop();
        for v in bl.vstart..ve {
            let vi = self.vinfo(v);
            let name = vi.name;
            if name == VName::Null {
                continue;
            }
            if vi.info & VSTACK_LABEL != 0 {
                if let Some(x) = self.vinfo_mut(v) {
                    x.name = VName::Null;
                }
                for vg in v + 1..ve {
                    let g = self.vinfo(vg);
                    if g.name == name && g.info & VSTACK_GOTO != 0 {
                        if bl.flags & FSCOPE_UPVAL != 0 && g.slot > vi.slot {
                            self.gola_close(vg);
                        }
                        self.gola_patch(vg, v);
                    }
                }
            } else if vi.info & VSTACK_GOTO != 0 {
                if let Some(prev) = self.cur.bl.last_mut() {
                    prev.flags |= if name == VName::Break { FSCOPE_BREAK } else { FSCOPE_GOLA };
                    if let Some(x) = self.vinfo_mut(v) {
                        x.slot = bl.nactvar;
                    }
                    if bl.flags & FSCOPE_UPVAL != 0 {
                        self.gola_close(v);
                    }
                } else {
                    self.ls.line = self.cur.lines.get(vi.startpc as usize).copied().unwrap_or(0);
                    if name == VName::Break {
                        return Err(self.ls.error(0, "no loop to break"));
                    }
                    let mut m = b"undefined label '".to_vec();
                    m.extend(self.name_text(name));
                    m.push(b'\'');
                    return Err(self.ls.error(0, m));
                }
            }
        }
        Ok(())
    }

    fn gola_findlabel(&self, name: u32) -> bool {
        let vstart = self.cur.bl.last().map_or(0, |b| b.vstart);
        (vstart..self.vtop()).any(|v| {
            let x = self.vinfo(v);
            x.name == VName::Sym(name) && x.info & VSTACK_LABEL != 0
        })
    }

    // -- scopes -------------------------------------------------------------------------------

    fn fscope_begin(&mut self, flags: u8) {
        let vstart = self.vtop();
        let nactvar = self.cur.nactvar;
        self.cur.bl.push(Scope { vstart, nactvar, flags });
    }

    fn fscope_end(&mut self) -> R<()> {
        let Some(bl) = self.cur.bl.pop() else { return Ok(()) };
        if self.cur.nactvar > bl.nactvar {
            self.cur.nactvar = bl.nactvar;
        }
        self.cur.freereg = self.cur.nactvar;
        if bl.flags & (FSCOPE_UPVAL | FSCOPE_NOCLOSE) == FSCOPE_UPVAL {
            self.emit(Op::Uclo);
        }
        if bl.flags & FSCOPE_BREAK != 0 {
            if bl.flags & FSCOPE_LOOP != 0 {
                // the break label lives one moment on the variable stack (gola_new + vtop = idx)
                let idx = self.vtop();
                if idx >= MAX_VSTACK {
                    return Err(self
                        .ls
                        .error(0, format!("chunk has more than {MAX_VSTACK} local variables")));
                }
                let pc = self.cur.pc();
                let vl =
                    VarInfo { name: VName::Break, startpc: pc, slot: self.cur.nactvar, info: VSTACK_LABEL };
                self.gola_resolve(bl.vstart, idx, vl)?;
            } else {
                return self.gola_fixup(bl);
            }
        }
        if bl.flags & FSCOPE_GOLA != 0 {
            self.gola_fixup(bl)?;
        }
        Ok(())
    }

    // -- functions ----------------------------------------------------------------------------

    fn fs_init(&mut self) {
        let vbase = self.vtop();
        let parent = std::mem::take(&mut self.cur);
        self.outer.push(parent);
        self.cur = FuncState { vbase, framesize: 1, ..FuncState::default() };
    }

    /// `fs_finish`: the closing checks of a function; leaves the parent current again and
    /// returns the closed function's instruction count.
    fn fs_finish(&mut self) -> R<u32> {
        let fs = &self.cur;
        let lastpc = fs.pc();
        let isret = lastpc > 0 && fs.op(lastpc - 1) == Op::Ret;
        if lastpc <= fs.lasttarget || !isret {
            if self.cur.bl.last().is_some_and(|b| b.flags & FSCOPE_UPVAL != 0) {
                self.emit(Op::Uclo);
            }
            self.emit(Op::Ret);
        }
        self.cur.bl_flags_or(FSCOPE_NOCLOSE);
        self.fscope_end()?;
        if self.cur.flags & PROTO_FIXUP_RETURN != 0 {
            let rets = self
                .cur
                .ops
                .iter()
                .take(lastpc as usize)
                .skip(1)
                .take_while(|o| **o != Op::Uclo)
                .filter(|o| **o == Op::Ret)
                .count();
            for _ in 0..rets {
                self.emit(Op::Ret);
            }
        }
        if self.cur.nkn > BCMAX_D + 1 || self.cur.nkgc > BCMAX_D + 1 {
            return Err(self.err_limit(self.cur.linedefined, BCMAX_D + 1, "constants"));
        }
        let total = self.cur.pc();
        let vbase = self.cur.vbase;
        self.vstack.truncate(vbase);
        if let Some(parent) = self.outer.pop() {
            self.cur = parent;
        }
        Ok(total)
    }

    fn synlevel_begin(&mut self) -> R<()> {
        self.level += 1;
        if self.level >= MAX_XLEVEL {
            return Err(self.ls.error(0, "chunk has too many syntax levels"));
        }
        Ok(())
    }

    fn synlevel_end(&mut self) {
        self.level -= 1;
    }

    // -- expression parsing -------------------------------------------------------------------

    fn expr_str(&mut self) -> R<Exp> {
        let sym = self.lex_str()?;
        let mut e = Exp::new(K::KStr, 0);
        e.sym = sym;
        e.d = self.node(DNode::Str(sym));
        Ok(e)
    }

    fn expr_index(&mut self, t: &mut Exp, e: &mut Exp) -> R<()> {
        t.k = K::Indexed;
        if e.isnumk() {
            let n = e.num;
            let k = num2int(n);
            if (0..=255).contains(&k) && n == f64::from(k) {
                t.idx = Idx::Byte;
                return Ok(());
            }
        } else if e.k == K::KStr {
            let idx = self.const_str(e.sym);
            if idx <= BCMAX_C {
                t.idx = Idx::Str;
                return Ok(());
            }
        }
        let r = self.expr_toanyreg(e)?;
        t.idx = Idx::Reg(r);
        Ok(())
    }

    fn expr_field(&mut self, v: &mut Exp) -> R<()> {
        self.expr_toanyreg(v)?;
        self.next()?;
        let mut key = self.expr_str()?;
        self.expr_index(v, &mut key)
    }

    fn expr_bracket(&mut self) -> R<Exp> {
        self.next()?;
        let mut v = self.expr()?;
        self.expr_toval(&mut v)?;
        self.lex_check(Tok::from(b']'))?;
        Ok(v)
    }

    fn expr_table(&mut self) -> R<Exp> {
        let line = self.ls.line;
        let mut has_template = false;
        let mut vcall = false;
        let mut narr: u32 = 1;
        let freg = self.cur.freereg;
        let pc = self.emit(Op::Tnew);
        let mut e = Exp::new(K::NonReloc, freg);
        self.bcreg_reserve(1)?;
        let freg = freg + 1;
        let mut fields: Vec<DField> = Vec::new();
        self.lex_check(Tok::from(b'{'))?;
        while self.ls.tok != Tok::from(b'}') {
            vcall = false;
            let mut key;
            if self.ls.tok == Tok::from(b'[') {
                key = self.expr_bracket()?;
                if !key.isk() {
                    self.expr_index(&mut e, &mut key)?;
                }
                self.lex_check(Tok::from(b'='))?;
            } else if (self.ls.tok == TK_NAME || self.ls.tok == TK_GOTO) && self.ls.peek()? == Tok::from(b'=')
            {
                key = self.expr_str()?;
                self.lex_check(Tok::from(b'='))?;
            } else {
                key = Exp::new(K::KNum, 0);
                key.num = f64::from(narr);
                key.d = self.node(DNode::Num(key.num));
                narr += 1;
                vcall = true;
            }
            let mut val = self.expr()?;
            let mut tpl = Tpl::Store;
            let mut nonconst = true;
            if key.isk() && key.k != K::KNil && (key.k == K::KStr || val.isk_nojump()) {
                if !has_template {
                    has_template = true;
                    self.const_gc_new();
                }
                vcall = false;
                if val.isk_nojump() {
                    tpl = Tpl::Set;
                    nonconst = false;
                } else {
                    tpl = Tpl::Hole;
                }
            }
            let (mut fline, mut fpc) = (0, 0);
            if nonconst {
                if val.k != K::Call {
                    self.expr_toanyreg(&mut val)?;
                    vcall = false;
                }
                if key.isk() {
                    self.expr_index(&mut e, &mut key)?;
                }
                fline = self.ls.lastline;
                self.bcemit_store(&e, &mut val)?;
                fpc = self.cur.pc().saturating_sub(1);
            }
            fields.push(DField { key: key.d, val: val.d, tpl, line: fline, pc: fpc });
            self.cur.freereg = freg;
            if !self.lex_opt(Tok::from(b','))? && !self.lex_opt(Tok::from(b';'))? {
                break;
            }
        }
        self.lex_match(Tok::from(b'}'), Tok::from(b'{'), line)?;
        if vcall {
            let biased = 4_503_599_627_370_496.0 + f64::from(narr - 1);
            self.const_num(biased);
            if narr > 256 {
                self.cur.ops.pop();
                self.cur.lines.pop();
            }
            let last = self.cur.pc().saturating_sub(1);
            self.cur.set_op(last, Op::Other);
        }
        if pc == self.cur.pc().wrapping_sub(1) {
            e.info = pc;
            self.cur.freereg -= 1;
            e.k = K::Relocable;
        } else {
            e.k = K::NonReloc;
        }
        e.idx = Idx::Byte;
        e.d = self.node(DNode::Table(fields));
        Ok(e)
    }

    fn parse_params(&mut self, needself: bool) -> R<u32> {
        let mut nparams = 0;
        self.lex_check(Tok::from(b'('))?;
        if needself {
            let s = self.sym(b"self");
            self.var_new(nparams, VName::Sym(s))?;
            nparams += 1;
        }
        if self.ls.tok != Tok::from(b')') {
            loop {
                if self.ls.tok == TK_NAME || self.ls.tok == TK_GOTO {
                    let s = self.lex_str()?;
                    self.var_new(nparams, VName::Sym(s))?;
                    nparams += 1;
                } else if self.ls.tok == TK_DOTS {
                    self.next()?;
                    self.cur.flags |= PROTO_VARARG;
                    break;
                } else {
                    return Err(self.err_syntax("<name> or '...' expected"));
                }
                if !self.lex_opt(Tok::from(b','))? {
                    break;
                }
            }
        }
        self.var_add(nparams);
        self.bcreg_reserve(nparams)?;
        self.lex_check(Tok::from(b')'))?;
        Ok(nparams)
    }

    fn parse_body(&mut self, needself: bool, line: u32) -> R<Exp> {
        self.fs_init();
        self.fscope_begin(0);
        self.cur.linedefined = line;
        self.cur.numparams = self.parse_params(needself)?;
        self.emit(Op::Other);
        self.parse_chunk()?;
        if self.ls.tok != TK_END {
            self.lex_match(TK_END, TK_FUNCTION, line)?;
        }
        self.ls.lastline = self.ls.line;
        self.fs_finish()?;
        self.const_gc_new();
        let pc = self.emit(Op::Other);
        let e = Exp::new(K::Relocable, pc);
        if self.cur.flags & PROTO_CHILD == 0 {
            if self.cur.flags & PROTO_HAS_RETURN != 0 {
                self.cur.flags |= PROTO_FIXUP_RETURN;
            }
            self.cur.flags |= PROTO_CHILD;
        }
        self.next()?;
        Ok(e)
    }

    fn expr_list(&mut self, v: &mut Exp, values: &mut Vec<u32>) -> R<u32> {
        let mut n = 1;
        *v = self.expr()?;
        values.push(v.d);
        while self.lex_opt(Tok::from(b','))? {
            self.expr_tonextreg(v)?;
            *v = self.expr()?;
            values.push(v.d);
            n += 1;
        }
        Ok(n)
    }

    fn parse_args(&mut self, e: &mut Exp) -> R<()> {
        let line = self.ls.line;
        let mut args;
        if self.ls.tok == Tok::from(b'(') {
            if line != self.ls.lastline {
                return Err(self.err_syntax("ambiguous syntax (function call x new statement)"));
            }
            self.next()?;
            if self.ls.tok == Tok::from(b')') {
                args = Exp::new(K::Void, 0);
            } else {
                args = Exp::new(K::Void, 0);
                self.expr_list(&mut args, &mut Vec::new())?;
            }
            self.lex_match(Tok::from(b')'), Tok::from(b'('), line)?;
        } else if self.ls.tok == Tok::from(b'{') {
            args = self.expr_table()?;
        } else if self.ls.tok == TK_STRING {
            args = Exp::new(K::KStr, 0);
            if let TokVal::Str(s) = &self.ls.tokval {
                let s = s.clone();
                args.sym = self.sym(&s);
            }
            self.next()?;
        } else {
            return Err(self.err_syntax("function arguments expected"));
        }
        let base = e.info;
        if args.k != K::Call && args.k != K::Void {
            self.expr_tonextreg(&mut args)?;
        }
        let pc = self.emit(Op::Call);
        *e = Exp::new(K::Call, pc);
        e.aux = base;
        if let Some(l) = self.cur.lines.last_mut() {
            *l = line;
        }
        self.cur.freereg = base + 1;
        Ok(())
    }

    fn expr_primary(&mut self) -> R<Exp> {
        let mut v;
        if self.ls.tok == Tok::from(b'(') {
            let line = self.ls.line;
            self.next()?;
            v = self.expr()?;
            self.lex_match(Tok::from(b')'), Tok::from(b'('), line)?;
            self.expr_discharge(&mut v);
            v.d = 0;
        } else if self.ls.tok == TK_NAME || self.ls.tok == TK_GOTO {
            v = Exp::new(K::Void, 0);
            self.var_lookup(&mut v)?;
        } else {
            return Err(self.err_syntax("unexpected symbol"));
        }
        loop {
            let t = self.ls.tok;
            if t == Tok::from(b'.') {
                self.expr_field(&mut v)?;
            } else if t == Tok::from(b'[') {
                self.expr_toanyreg(&mut v)?;
                let mut key = self.expr_bracket()?;
                self.expr_index(&mut v, &mut key)?;
            } else if t == Tok::from(b':') {
                self.next()?;
                let key = self.expr_str()?;
                self.bcemit_method(&mut v, &key)?;
                self.parse_args(&mut v)?;
            } else if t == Tok::from(b'(') || t == TK_STRING || t == Tok::from(b'{') {
                self.expr_tonextreg(&mut v)?;
                self.parse_args(&mut v)?;
            } else {
                break;
            }
            v.d = 0;
        }
        Ok(v)
    }

    fn expr_simple(&mut self) -> R<Exp> {
        let mut v;
        match self.ls.tok {
            TK_NUMBER => match self.ls.tokval {
                TokVal::Num(n) => {
                    v = Exp::new(K::KNum, 0);
                    v.num = n;
                    v.d = self.node(DNode::Num(n));
                }
                _ => v = Exp::new(K::KCdata, 0),
            },
            TK_STRING => {
                v = Exp::new(K::KStr, 0);
                if let TokVal::Str(s) = &self.ls.tokval {
                    let s = s.clone();
                    v.sym = self.sym(&s);
                }
                v.d = self.node(DNode::Str(v.sym));
            }
            TK_NIL => {
                v = Exp::new(K::KNil, 0);
                v.d = self.node(DNode::Nil);
            }
            TK_TRUE => {
                v = Exp::new(K::KTrue, 0);
                v.d = self.node(DNode::Bool(true));
            }
            TK_FALSE => {
                v = Exp::new(K::KFalse, 0);
                v.d = self.node(DNode::Bool(false));
            }
            TK_DOTS => {
                if self.cur.flags & PROTO_VARARG == 0 {
                    return Err(self.err_syntax("cannot use '...' outside a vararg function"));
                }
                self.bcreg_reserve(1)?;
                let base = self.cur.freereg - 1;
                let pc = self.emit(Op::Varg);
                v = Exp::new(K::Call, pc);
                v.aux = base;
            }
            t if t == Tok::from(b'{') => return self.expr_table(),
            TK_FUNCTION => {
                self.next()?;
                let line = self.ls.line;
                return self.parse_body(false, line);
            }
            _ => return self.expr_primary(),
        }
        self.next()?;
        Ok(v)
    }

    fn expr_unop(&mut self) -> R<Exp> {
        let op = match self.ls.tok {
            TK_NOT => UnOp::Not,
            t if t == Tok::from(b'-') => UnOp::Neg,
            t if t == Tok::from(b'#') => UnOp::Len,
            _ => return self.expr_simple(),
        };
        self.next()?;
        let (mut v, _) = self.expr_binop(UNARY_PRIORITY)?;
        self.bcemit_unop(op, &mut v)?;
        Ok(v)
    }

    fn expr_binop(&mut self, limit: u8) -> R<(Exp, Option<BinOpr>)> {
        self.synlevel_begin()?;
        let mut v = self.expr_unop()?;
        let mut op = token2binop(self.ls.tok);
        while let Some(o) = op {
            let (left, right) = priority(o);
            if left <= limit {
                break;
            }
            self.next()?;
            self.bcemit_binop_left(o, &mut v)?;
            let (mut v2, nextop) = self.expr_binop(right)?;
            self.bcemit_binop(o, &mut v, &mut v2)?;
            op = nextop;
        }
        self.synlevel_end();
        Ok((v, op))
    }

    fn expr(&mut self) -> R<Exp> {
        Ok(self.expr_binop(0)?.0)
    }

    fn expr_next(&mut self) -> R<()> {
        let mut e = self.expr()?;
        self.expr_tonextreg(&mut e)
    }

    fn expr_cond(&mut self) -> R<()> {
        let mut v = self.expr()?;
        if v.k == K::KNil {
            v.k = K::KFalse;
        }
        self.bcemit_branch_t(&mut v)
    }

    // -- assignments --------------------------------------------------------------------------

    fn assign_hazard(&mut self, lh: &mut [Exp], v: &Exp) -> R<()> {
        let reg = v.info;
        let tmp = self.cur.freereg;
        let mut hazard = false;
        for x in lh.iter_mut() {
            if x.k == K::Indexed {
                if x.info == reg {
                    hazard = true;
                    x.info = tmp;
                }
                if x.idx == Idx::Reg(reg) {
                    hazard = true;
                    x.idx = Idx::Reg(tmp);
                }
            }
        }
        if hazard {
            self.emit(Op::Other);
            self.bcreg_reserve(1)?;
        }
        Ok(())
    }

    fn assign_adjust(&mut self, nvars: u32, nexps: u32, e: &mut Exp) -> R<()> {
        let mut extra = i64::from(nvars) - i64::from(nexps);
        if e.k == K::Call {
            extra += 1;
            if extra < 0 {
                extra = 0;
            }
            if extra > 1 {
                self.bcreg_reserve(u32::try_from(extra - 1).unwrap_or(0))?;
            }
        } else {
            if e.k != K::Void {
                self.expr_tonextreg(e)?;
            }
            if extra > 0 {
                let reg = self.cur.freereg;
                let n = u32::try_from(extra).unwrap_or(0);
                self.bcreg_reserve(n)?;
                self.bcemit_nil(reg, n);
            }
        }
        if nexps > nvars {
            self.cur.freereg -= nexps - nvars;
        }
        Ok(())
    }

    fn parse_assignment(&mut self, lh: &mut Vec<Exp>, nvars: u32) -> R<()> {
        let Some(last) = lh.last().copied() else { return Ok(()) };
        if !(K::Local..=K::Indexed).contains(&last.k) {
            return Err(self.err_syntax("syntax error"));
        }
        let mut e;
        if self.lex_opt(Tok::from(b','))? {
            let v = self.expr_primary()?;
            if v.k == K::Local {
                self.assign_hazard(lh, &v)?;
            }
            if self.level + nvars >= MAX_XLEVEL {
                return Err(self.err_limit(self.cur.linedefined, MAX_XLEVEL, "variable names"));
            }
            lh.push(v);
            self.parse_assignment(lh, nvars + 1)?;
            lh.pop();
        } else {
            self.lex_check(Tok::from(b'='))?;
            e = Exp::new(K::Void, 0);
            let nexps = self.expr_list(&mut e, &mut Vec::new())?;
            if nexps == nvars {
                if e.k == K::Call {
                    if self.cur.op(e.info) == Op::Varg {
                        self.cur.freereg -= 1;
                        e.k = K::Relocable;
                    } else {
                        e.info = e.aux;
                        e.k = K::NonReloc;
                    }
                }
                return self.bcemit_store(&last, &mut e);
            }
            self.assign_adjust(nvars, nexps, &mut e)?;
        }
        let Some(target) = lh.last().copied() else { return Ok(()) };
        e = Exp::new(K::NonReloc, self.cur.freereg.wrapping_sub(1));
        self.bcemit_store(&target, &mut e)
    }

    fn parse_call_assign(&mut self) -> R<()> {
        let v = self.expr_primary()?;
        if v.k != K::Call {
            let mut lh = vec![v];
            self.parse_assignment(&mut lh, 1)?;
        }
        Ok(())
    }

    fn parse_local(&mut self) -> R<()> {
        if self.lex_opt(TK_FUNCTION)? {
            let name = self.lex_str()?;
            self.var_new(0, VName::Sym(name))?;
            let reg = self.cur.freereg;
            self.bcreg_reserve(1)?;
            self.var_add(1);
            let line = self.ls.line;
            let mut b = self.parse_body(false, line)?;
            self.expr_free(&b);
            self.expr_toreg(&mut b, reg);
        } else {
            let mut nvars = 0;
            loop {
                let name = self.lex_str()?;
                self.var_new(nvars, VName::Sym(name))?;
                nvars += 1;
                if !self.lex_opt(Tok::from(b','))? {
                    break;
                }
            }
            let mut e = Exp::new(K::Void, 0);
            let nexps =
                if self.lex_opt(Tok::from(b'='))? { self.expr_list(&mut e, &mut Vec::new())? } else { 0 };
            self.assign_adjust(nvars, nexps, &mut e)?;
            self.var_add(nvars);
        }
        Ok(())
    }

    fn parse_func(&mut self, line: u32) -> R<()> {
        self.next()?;
        let mut v = Exp::new(K::Void, 0);
        self.var_lookup(&mut v)?;
        while self.ls.tok == Tok::from(b'.') {
            self.expr_field(&mut v)?;
        }
        let mut needself = false;
        if self.ls.tok == Tok::from(b':') {
            needself = true;
            self.expr_field(&mut v)?;
        }
        let mut b = self.parse_body(needself, line)?;
        self.bcemit_store(&v, &mut b)
    }

    // -- statements ---------------------------------------------------------------------------

    fn parse_return(&mut self) -> R<()> {
        self.next()?;
        self.cur.flags |= PROTO_HAS_RETURN;
        let mut values = Vec::new();
        if !parse_isend(self.ls.tok) && self.ls.tok != Tok::from(b';') {
            let mut e = Exp::new(K::Void, 0);
            let nret = self.expr_list(&mut e, &mut values)?;
            if nret == 1 && e.k == K::Call && self.cur.op(e.info) != Op::Varg {
                self.cur.ops.pop();
                self.cur.lines.pop();
            } else if nret == 1 && e.k != K::Call {
                self.expr_toanyreg(&mut e)?;
            } else if e.k != K::Call {
                self.expr_tonextreg(&mut e)?;
            }
        }
        if self.cur.flags & PROTO_CHILD != 0 {
            self.emit(Op::Uclo);
        }
        self.emit(Op::Ret);
        if self.outer.is_empty() {
            self.ret = Some(values);
        }
        Ok(())
    }

    fn parse_break(&mut self) -> R<()> {
        self.cur.bl_flags_or(FSCOPE_BREAK);
        let pc = self.bcemit_jmp();
        self.gola_new(VName::Break, VSTACK_GOTO, pc)?;
        Ok(())
    }

    fn parse_goto(&mut self) -> R<()> {
        let name = self.lex_str()?;
        if self.gola_findlabel(name) {
            self.emit(Op::Other);
        }
        self.cur.bl_flags_or(FSCOPE_GOLA);
        let pc = self.bcemit_jmp();
        self.gola_new(VName::Sym(name), VSTACK_GOTO, pc)?;
        Ok(())
    }

    fn parse_label(&mut self) -> R<()> {
        self.cur.lasttarget = self.cur.pc();
        self.cur.bl_flags_or(FSCOPE_GOLA);
        self.next()?;
        let name = self.lex_str()?;
        if self.gola_findlabel(name) {
            let mut m = b"duplicate label '".to_vec();
            m.extend(self.sym_bytes(name));
            m.push(b'\'');
            return Err(self.ls.error(0, m));
        }
        let pc = self.cur.pc();
        let idx = self.gola_new(VName::Sym(name), VSTACK_LABEL, pc)?;
        self.lex_check(TK_LABEL)?;
        while self.ls.tok == TK_LABEL {
            self.synlevel_begin()?;
            self.parse_label()?;
            self.synlevel_end();
        }
        if parse_isend(self.ls.tok) && self.ls.tok != TK_UNTIL {
            let n = self.cur.bl.last().map_or(0, |b| b.nactvar);
            if let Some(v) = self.vinfo_mut(idx) {
                v.slot = n;
            }
        }
        let vstart = self.cur.bl.last().map_or(0, |b| b.vstart);
        let vl = self.vinfo(idx);
        self.gola_resolve(vstart, idx, vl)
    }

    fn parse_block(&mut self) -> R<()> {
        self.fscope_begin(0);
        self.parse_chunk()?;
        self.fscope_end()
    }

    fn parse_while(&mut self, line: u32) -> R<()> {
        self.next()?;
        self.cur.lasttarget = self.cur.pc();
        self.expr_cond()?;
        self.fscope_begin(FSCOPE_LOOP);
        self.lex_check(TK_DO)?;
        self.emit(Op::Other);
        self.parse_block()?;
        self.bcemit_jmp();
        self.lex_match(TK_END, TK_WHILE, line)?;
        self.fscope_end()?;
        self.jmp_tohere();
        Ok(())
    }

    fn parse_repeat(&mut self, line: u32) -> R<()> {
        let looppc = self.cur.pc();
        self.cur.lasttarget = looppc;
        self.fscope_begin(FSCOPE_LOOP);
        self.fscope_begin(0);
        self.next()?;
        self.emit(Op::Other);
        self.parse_chunk()?;
        self.lex_match(TK_UNTIL, TK_REPEAT, line)?;
        self.expr_cond()?;
        let upval = self.cur.bl.last().is_some_and(|b| b.flags & FSCOPE_UPVAL != 0);
        if !upval {
            self.fscope_end()?;
        } else {
            self.parse_break()?;
            self.jmp_tohere();
            self.fscope_end()?;
            self.bcemit_jmp();
        }
        if looppc == self.cur.pc() {
            self.jmp_tohere();
        }
        self.fscope_end()
    }

    fn parse_for_num(&mut self, varname: u32) -> R<()> {
        self.var_new(0, VName::Fixed)?;
        self.var_new(1, VName::Fixed)?;
        self.var_new(2, VName::Fixed)?;
        self.var_new(3, VName::Sym(varname))?;
        self.lex_check(Tok::from(b'='))?;
        self.expr_next()?;
        self.lex_check(Tok::from(b','))?;
        self.expr_next()?;
        if self.lex_opt(Tok::from(b','))? {
            self.expr_next()?;
        } else {
            self.emit(Op::Other);
            self.bcreg_reserve(1)?;
        }
        self.var_add(3);
        self.lex_check(TK_DO)?;
        self.emit(Op::Other);
        self.fscope_begin(0);
        self.var_add(1);
        self.bcreg_reserve(1)?;
        self.parse_block()?;
        self.fscope_end()?;
        self.emit(Op::Other);
        Ok(())
    }

    fn parse_for_iter(&mut self, indexname: u32) -> R<()> {
        let mut nvars = 0;
        for _ in 0..3 {
            self.var_new(nvars, VName::Fixed)?;
            nvars += 1;
        }
        self.var_new(nvars, VName::Sym(indexname))?;
        nvars += 1;
        while self.lex_opt(Tok::from(b','))? {
            let s = self.lex_str()?;
            self.var_new(nvars, VName::Sym(s))?;
            nvars += 1;
        }
        self.lex_check(TK_IN)?;
        let mut e = Exp::new(K::Void, 0);
        let n = self.expr_list(&mut e, &mut Vec::new())?;
        self.assign_adjust(3, n, &mut e)?;
        self.bcreg_bump(3)?;
        self.var_add(3);
        self.lex_check(TK_DO)?;
        self.emit(Op::Other);
        self.fscope_begin(0);
        self.var_add(nvars - 3);
        self.bcreg_reserve(nvars - 3)?;
        self.parse_block()?;
        self.fscope_end()?;
        self.emit(Op::Other);
        self.emit(Op::Other);
        Ok(())
    }

    fn parse_for(&mut self, line: u32) -> R<()> {
        self.fscope_begin(FSCOPE_LOOP);
        self.next()?;
        let varname = self.lex_str()?;
        if self.ls.tok == Tok::from(b'=') {
            self.parse_for_num(varname)?;
        } else if self.ls.tok == Tok::from(b',') || self.ls.tok == TK_IN {
            self.parse_for_iter(varname)?;
        } else {
            return Err(self.err_syntax("'=' or 'in' expected"));
        }
        self.lex_match(TK_END, TK_FOR, line)?;
        self.fscope_end()
    }

    fn parse_then(&mut self) -> R<()> {
        self.next()?;
        self.expr_cond()?;
        self.lex_check(TK_THEN)?;
        self.parse_block()
    }

    fn parse_if(&mut self, line: u32) -> R<()> {
        self.parse_then()?;
        while self.ls.tok == TK_ELSEIF {
            self.bcemit_jmp();
            self.jmp_tohere();
            self.parse_then()?;
        }
        if self.ls.tok == TK_ELSE {
            self.bcemit_jmp();
            self.jmp_tohere();
            self.next()?;
            self.parse_block()?;
        }
        self.jmp_tohere();
        self.lex_match(TK_END, TK_IF, line)
    }

    fn parse_stmt(&mut self) -> R<bool> {
        let line = self.ls.line;
        match self.ls.tok {
            TK_IF => self.parse_if(line)?,
            TK_WHILE => self.parse_while(line)?,
            TK_DO => {
                self.next()?;
                self.parse_block()?;
                self.lex_match(TK_END, TK_DO, line)?;
            }
            TK_FOR => self.parse_for(line)?,
            TK_REPEAT => self.parse_repeat(line)?,
            TK_FUNCTION => self.parse_func(line)?,
            TK_LOCAL => {
                self.next()?;
                self.parse_local()?;
            }
            TK_RETURN => {
                self.parse_return()?;
                return Ok(true);
            }
            TK_BREAK => {
                self.next()?;
                self.parse_break()?;
                return Ok(true);
            }
            TK_LABEL => self.parse_label()?,
            TK_GOTO => {
                if self.ls.peek()? == TK_NAME {
                    self.next()?;
                    self.parse_goto()?;
                } else {
                    self.parse_call_assign()?;
                }
            }
            _ => self.parse_call_assign()?,
        }
        Ok(false)
    }

    fn parse_chunk(&mut self) -> R<()> {
        let mut islast = false;
        self.synlevel_begin()?;
        while !islast && !parse_isend(self.ls.tok) {
            islast = self.parse_stmt()?;
            self.lex_opt(Tok::from(b';'))?;
            self.cur.freereg = self.cur.nactvar;
        }
        self.synlevel_end();
        Ok(())
    }

    /// `lj_parse`: the main chunk. Returns its instruction count.
    fn run(&mut self) -> R<u32> {
        self.level = 0;
        self.cur = FuncState { framesize: 1, flags: PROTO_VARARG, ..FuncState::default() };
        self.fscope_begin(0);
        self.emit(Op::Other);
        self.next()?;
        self.parse_chunk()?;
        if self.ls.tok != TK_EOF {
            return Err(self.err_token(TK_EOF));
        }
        self.fs_finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UnOp {
    Not,
    Neg,
    Len,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BinOpr {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Ne,
    Eq,
    Lt,
    Ge,
    Le,
    Gt,
    And,
    Or,
}

const UNARY_PRIORITY: u8 = 8;

fn priority(op: BinOpr) -> (u8, u8) {
    match op {
        BinOpr::Add | BinOpr::Sub => (6, 6),
        BinOpr::Mul | BinOpr::Div | BinOpr::Mod => (7, 7),
        BinOpr::Pow => (10, 9),
        BinOpr::Concat => (5, 4),
        BinOpr::Ne | BinOpr::Eq | BinOpr::Lt | BinOpr::Ge | BinOpr::Le | BinOpr::Gt => (3, 3),
        BinOpr::And => (2, 2),
        BinOpr::Or => (1, 1),
    }
}

fn token2binop(tok: Tok) -> Option<BinOpr> {
    Some(match tok {
        t if t == Tok::from(b'+') => BinOpr::Add,
        t if t == Tok::from(b'-') => BinOpr::Sub,
        t if t == Tok::from(b'*') => BinOpr::Mul,
        t if t == Tok::from(b'/') => BinOpr::Div,
        t if t == Tok::from(b'%') => BinOpr::Mod,
        t if t == Tok::from(b'^') => BinOpr::Pow,
        TK_CONCAT => BinOpr::Concat,
        TK_NE => BinOpr::Ne,
        TK_EQ => BinOpr::Eq,
        t if t == Tok::from(b'<') => BinOpr::Lt,
        TK_LE => BinOpr::Le,
        t if t == Tok::from(b'>') => BinOpr::Gt,
        TK_GE => BinOpr::Ge,
        TK_AND => BinOpr::And,
        TK_OR => BinOpr::Or,
        _ => return None,
    })
}

fn parse_isend(tok: Tok) -> bool {
    matches!(tok, TK_ELSE | TK_ELSEIF | TK_END | TK_UNTIL | TK_EOF)
}

/// `lj_num2int`: the truncating conversion of x64, with `i32::MIN` for anything out of range.
fn num2int(n: f64) -> i32 {
    if n.is_nan() || n >= 2_147_483_648.0 || n < -2_147_483_648.0 { i32::MIN } else { n as i32 }
}
