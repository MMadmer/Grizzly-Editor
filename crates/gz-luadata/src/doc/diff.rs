//! Saving a typed model as the difference of two values: what the model read (`old`) and what
//! it holds now (`new`), applied to the document's own bytes. A rebase is the same walk over
//! another text, watching for places that text changed too.

use rustc_hash::{FxHashMap, FxHashSet};
use smol_str::SmolStr;

use super::{Anchor, Body, LuaDoc, Span, Splice, TableEdits, plain};
use crate::LuaDataError;
use crate::path::{Seg, covers, data_eq};
use crate::value::{Table, Value};

/// How a kind's values map onto a file: which lists are matched by a key, where new keys go,
/// which lists hold nodes. A pattern is a dotted path from the root, `*` standing for any key or
/// list item: `""` is the top table, `"nodes"` the node list, `"nodes.*"` every node.
#[derive(Clone, Debug, Default)]
pub struct DiffRules {
    keyed: Vec<(Vec<SmolStr>, SmolStr)>,
    order: Vec<(Vec<SmolStr>, Vec<SmolStr>)>,
    node: Vec<Vec<SmolStr>>,
}

fn pattern(p: &str) -> Vec<SmolStr> {
    if p.is_empty() { Vec::new() } else { p.split('.').map(SmolStr::new).collect() }
}

fn matches(p: &[SmolStr], at: &[SmolStr]) -> bool {
    p.len() == at.len() && p.iter().zip(at).all(|(a, b)| a == "*" || a == b)
}

impl DiffRules {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The items of `list` are matched by their `field` (`id`, `name`), not by position: their
    /// order in the file is the file's.
    #[must_use]
    pub fn keyed(mut self, list: &str, field: &str) -> Self {
        self.keyed.push((pattern(list), SmolStr::new(field)));
        self
    }

    /// The canonical order of the keys of `table`: a new key goes after the nearest key before
    /// it that the file holds.
    #[must_use]
    pub fn order(mut self, table: &str, keys: &[&str]) -> Self {
        self.order.push((pattern(table), keys.iter().copied().map(SmolStr::new).collect()));
        self
    }

    /// New items of `list` are written in the node layout.
    #[must_use]
    pub fn node_layout(mut self, list: &str) -> Self {
        self.node.push(pattern(list));
        self
    }

    fn keyed_field(&self, at: &[SmolStr]) -> Option<&str> {
        self.keyed.iter().find(|(p, _)| matches(p, at)).map(|(_, f)| f.as_str())
    }

    fn order_for(&self, at: &[SmolStr]) -> Option<&[SmolStr]> {
        self.order.iter().find(|(p, _)| matches(p, at)).map(|(_, o)| o.as_slice())
    }

    fn is_node(&self, at: &[SmolStr]) -> bool {
        self.node.iter().any(|p| matches(p, at))
    }
}

/// A place both sides of a rebase changed: the path, the value each side gives it (`None`:
/// removed) and where it stands in the result. The result holds `mine`.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub path: Vec<Seg>,
    pub mine: Option<Value>,
    pub theirs: Option<Value>,
    pub span: Option<Span>,
}

struct Walk<'d> {
    doc: &'d LuaDoc,
    rules: &'d DiffRules,
    rebase: bool,
    splices: Vec<Splice>,
    tables: FxHashMap<u32, TableEdits>,
    conflicts: Vec<Conflict>,
    path: Vec<Seg>,
    pat: Vec<SmolStr>,
}

type R = Result<(), LuaDataError>;

fn item_key(v: &Value, field: &str) -> Option<String> {
    match v.get(field)? {
        Value::String(s) => Some(s.to_string()),
        x @ Value::Number(_) => Some(x.to_text("")),
        _ => None,
    }
}

fn arr_eq(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| data_eq(x, y))
}

impl<'d> Walk<'d> {
    fn removed(&self, tid: u32, j: usize) -> bool {
        self.tables.get(&tid).is_some_and(|e| e.removed.contains(&j))
    }

    fn remove(&mut self, tid: u32, j: usize) {
        let ed = self.tables.entry(tid).or_default();
        if !ed.removed.contains(&j) {
            ed.removed.push(j);
        }
    }

    fn insert(&mut self, tid: u32, at: Anchor, body: Body) {
        self.tables.entry(tid).or_default().inserts.push((at, body));
    }

    fn conflict(&mut self, mine: Option<Value>, theirs: Option<Value>) {
        if self.rebase {
            self.conflicts.push(Conflict { path: self.path.clone(), mine, theirs, span: None });
        }
    }

    fn effective(&self, tid: u32) -> Result<super::run::Eff<'d>, LuaDataError> {
        let doc: &'d LuaDoc = self.doc;
        doc.ev().effective(tid).map_err(|e| LuaDoc::err_in(&doc.text, e))
    }

    fn table(&mut self, tid: u32, old: &Table, new: &Table) -> R {
        let eff = self.effective(tid)?;
        let strs: Vec<(Vec<u8>, usize)> = eff.strs.iter().map(|(k, j)| (k.to_vec(), *j)).collect();
        let nums: Vec<usize> = eff.nums.iter().map(|x| x.1).collect();
        let key = |k: &str| strs.iter().find(|(s, _)| s.as_slice() == k.as_bytes()).map(|(_, j)| *j);
        for (k, ov) in &old.fields {
            if new.get(k).is_some() {
                continue;
            }
            if let Some(j) = key(k) {
                self.path.push(Seg::Key(k.clone()));
                if self.rebase {
                    let dv = self.doc.entry_value(tid, j);
                    if !covers(&dv, ov) {
                        self.conflict(None, Some(dv));
                    }
                }
                self.remove(tid, j);
                self.path.pop();
            }
        }
        for (k, nv) in &new.fields {
            let ov = old.get(k);
            if ov.is_some_and(|ov| data_eq(ov, nv)) {
                continue;
            }
            match key(k) {
                Some(j) => {
                    self.path.push(Seg::Key(k.clone()));
                    self.pat.push(k.clone());
                    self.changed(tid, j, ov, nv)?;
                    self.pat.pop();
                    self.path.pop();
                }
                None => {
                    let anchor = self.key_anchor(tid, &key, k, new);
                    self.path.push(Seg::Key(k.clone()));
                    if ov.is_some() {
                        self.conflict(Some(nv.clone()), None);
                    }
                    self.path.pop();
                    self.insert(
                        tid,
                        anchor,
                        Body::Entry { key: Some(k.clone()), value: nv.clone(), node: false },
                    );
                }
            }
        }
        if !arr_eq(&old.arr, &new.arr) {
            match self.rules.keyed_field(&self.pat) {
                Some(field) => {
                    let field = SmolStr::new(field);
                    self.keyed(tid, &nums, &field, &old.arr, &new.arr)?;
                }
                None => self.positional(tid, &nums, &old.arr, &new.arr)?,
            }
        }
        Ok(())
    }

    /// Where a new key of a table goes: after the nearest key before it (in the rules' order,
    /// else in the new value's) the file holds, else before the nearest after it, else last.
    fn key_anchor(&self, tid: u32, key: &dyn Fn(&str) -> Option<usize>, k: &str, new: &Table) -> Anchor {
        let kept = |name: &str| key(name).filter(|&j| !self.removed(tid, j));
        if let Some(order) = self.rules.order_for(&self.pat)
            && let Some(pos) = order.iter().position(|x| x == k)
        {
            if let Some(j) = order[..pos].iter().rev().find_map(|x| kept(x)) {
                return Anchor::After(j);
            }
            if let Some(j) = order[pos + 1..].iter().find_map(|x| kept(x)) {
                return Anchor::Before(j);
            }
        }
        let pos = new.fields.iter().position(|(x, _)| x == k).unwrap_or(new.fields.len());
        if let Some(j) = new.fields[..pos].iter().rev().find_map(|(x, _)| kept(x)) {
            return Anchor::After(j);
        }
        if let Some(j) = new.fields.get(pos + 1..).unwrap_or(&[]).iter().find_map(|(x, _)| kept(x)) {
            return Anchor::Before(j);
        }
        Anchor::End
    }

    /// One value the model changed, at entry `j` of table `tid`.
    fn changed(&mut self, tid: u32, j: usize, ov: Option<&Value>, nv: &Value) -> R {
        let ct = self.doc.table(tid).and_then(|t| t.entries.get(j)).and_then(|e| e.val.table);
        if let (Value::Table(nt), Some(ct)) = (nv, ct) {
            if let Some(Value::Table(ot)) = ov {
                // a list of the file that does not line up with what the model read (an item
                // the model skipped, a table where it read a list) cannot be followed by
                // position: the value is written whole
                if !self.rebase
                    && !ot.arr.is_empty()
                    && self.rules.keyed_field(&self.pat).is_none()
                    && self.effective(ct)?.nums.len() != ot.arr.len()
                {
                    self.splices.push(self.doc.replace_splice(tid, j, nv));
                    return Ok(());
                }
                return self.table(ct, ot, nt);
            }
            // the file holds a table the model did not read here: write the difference to it
            let dv = self.doc.entry_value(tid, j);
            if data_eq(&dv, nv) {
                return Ok(());
            }
            self.conflict(Some(nv.clone()), Some(dv.clone()));
            let Value::Table(dt) = dv else { return Ok(()) };
            return self.table(ct, &dt, nt);
        }
        let dv = self.doc.entry_value(tid, j);
        if data_eq(&dv, nv) {
            return Ok(());
        }
        if !ov.is_some_and(|o| covers(&dv, o)) {
            self.conflict(Some(nv.clone()), Some(dv));
        }
        self.splices.push(self.doc.replace_splice(tid, j, nv));
        Ok(())
    }

    fn keyed(&mut self, tid: u32, nums: &[usize], field: &SmolStr, old: &[Value], new: &[Value]) -> R {
        let mut doc_items: FxHashMap<String, usize> = FxHashMap::default();
        let mut unkeyed: Vec<usize> = Vec::new();
        for &j in nums {
            match self.doc.item_key(tid, j, field) {
                Some(k) => {
                    doc_items.entry(k).or_insert(j);
                }
                None => unkeyed.push(j),
            }
        }
        let old_items: FxHashMap<String, &Value> =
            old.iter().filter_map(|v| item_key(v, field).map(|k| (k, v))).collect();
        let new_keys: FxHashSet<String> = new.iter().filter_map(|v| item_key(v, field)).collect();
        let node = self.rules.is_node(&self.pat);
        for ov in old {
            match item_key(ov, field) {
                Some(k) if !new_keys.contains(&k) => {
                    if let Some(&j) = doc_items.get(&k) {
                        self.path.push(Seg::Item { field: field.clone(), value: SmolStr::new(&k) });
                        if self.rebase {
                            let dv = self.doc.entry_value(tid, j);
                            if !covers(&dv, ov) {
                                self.conflict(None, Some(dv));
                            }
                        }
                        self.remove(tid, j);
                        self.path.pop();
                    }
                }
                None if !new.iter().any(|nv| item_key(nv, field).is_none() && data_eq(nv, ov)) => {
                    if let Some(&j) = unkeyed
                        .iter()
                        .find(|&&j| !self.removed(tid, j) && data_eq(&self.doc.entry_value(tid, j), ov))
                    {
                        self.remove(tid, j);
                    }
                }
                _ => {}
            }
        }
        for (pos, nv) in new.iter().enumerate() {
            let Some(k) = item_key(nv, field) else {
                if !old.iter().any(|ov| item_key(ov, field).is_none() && data_eq(ov, nv)) {
                    let at = self.item_anchor(tid, &doc_items, field, new, pos);
                    self.insert(tid, at, Body::Entry { key: None, value: nv.clone(), node });
                }
                continue;
            };
            self.path.push(Seg::Item { field: field.clone(), value: SmolStr::new(&k) });
            self.pat.push(SmolStr::new_static("*"));
            match (old_items.get(&k), doc_items.get(&k)) {
                (Some(ov), Some(&j)) => {
                    if !data_eq(ov, nv) {
                        self.changed(tid, j, Some(ov), nv)?;
                    }
                }
                (Some(ov), None) => {
                    if !data_eq(ov, nv) {
                        self.conflict(Some(nv.clone()), None);
                        let at = self.item_anchor(tid, &doc_items, field, new, pos);
                        self.insert(tid, at, Body::Entry { key: None, value: nv.clone(), node });
                    }
                }
                (None, Some(&j)) => self.changed(tid, j, None, nv)?,
                (None, None) => {
                    let at = self.item_anchor(tid, &doc_items, field, new, pos);
                    self.insert(tid, at, Body::Entry { key: None, value: nv.clone(), node });
                }
            }
            self.pat.pop();
            self.path.pop();
        }
        Ok(())
    }

    /// Where a new item of a keyed list goes: after the nearest item before it in the new list
    /// that the file keeps, else before the nearest after it, else last.
    fn item_anchor(
        &self,
        tid: u32,
        doc_items: &FxHashMap<String, usize>,
        field: &str,
        new: &[Value],
        pos: usize,
    ) -> Anchor {
        let kept = |v: &Value| {
            item_key(v, field).and_then(|k| doc_items.get(&k).copied()).filter(|&j| !self.removed(tid, j))
        };
        if let Some(j) = new[..pos].iter().rev().find_map(kept) {
            return Anchor::After(j);
        }
        if let Some(j) = new.get(pos + 1..).unwrap_or(&[]).iter().find_map(kept) {
            return Anchor::Before(j);
        }
        Anchor::End
    }

    fn positional(&mut self, tid: u32, nums: &[usize], old: &[Value], new: &[Value]) -> R {
        let node = self.rules.is_node(&self.pat);
        if self.rebase {
            let doc: Vec<Value> = nums.iter().map(|&j| self.doc.entry_value(tid, j)).collect();
            let same = doc.len() == old.len() && doc.iter().zip(old).all(|(d, o)| covers(d, o));
            if !same {
                // the other side changed the list too: this side's list replaces it
                if !arr_eq(&doc, new) {
                    self.conflict(Some(Value::from(new.to_vec())), Some(Value::from(doc)));
                    for &j in nums {
                        self.remove(tid, j);
                    }
                    for nv in new {
                        self.insert(tid, Anchor::End, Body::Entry { key: None, value: nv.clone(), node });
                    }
                }
                return Ok(());
            }
        }
        let (lo, ln) = (old.len(), new.len());
        let mut p = 0;
        while p < lo && p < ln && data_eq(&old[p], &new[p]) {
            p += 1;
        }
        let mut s = 0;
        while s < lo - p && s < ln - p && data_eq(&old[lo - 1 - s], &new[ln - 1 - s]) {
            s += 1;
        }
        let (mo, mn) = (lo - p - s, ln - p - s);
        for k in 0..mo.min(mn) {
            let i = p + k;
            self.path.push(Seg::Index(i));
            self.pat.push(SmolStr::new_static("*"));
            match nums.get(i) {
                Some(&j) => self.changed(tid, j, Some(&old[i]), &new[i])?,
                None => self.insert(tid, Anchor::End, Body::Entry { key: None, value: new[i].clone(), node }),
            }
            self.pat.pop();
            self.path.pop();
        }
        if mn > mo {
            let after = p + mo;
            let anchor = match after.checked_sub(1).and_then(|i| nums.get(i)) {
                Some(&j) => Anchor::After(j),
                None => nums.get(after).map_or(Anchor::End, |&j| Anchor::Before(j)),
            };
            for nv in &new[p + mo..p + mn] {
                self.insert(tid, anchor, Body::Entry { key: None, value: nv.clone(), node });
            }
        } else if mo > mn {
            for i in p + mn..p + mo {
                if let Some(&j) = nums.get(i) {
                    self.remove(tid, j);
                }
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(Vec<Splice>, Vec<Conflict>), LuaDataError> {
        let mut ids: Vec<u32> = self.tables.keys().copied().collect();
        ids.sort_unstable();
        for tid in ids {
            if let Some(ed) = self.tables.get(&tid) {
                self.doc.table_splices(tid, ed, &mut self.splices)?;
            }
        }
        Ok((self.splices, self.conflicts))
    }
}

impl LuaDoc {
    fn walk(
        &self,
        old: &Value,
        new: &Value,
        rules: &DiffRules,
        rebase: bool,
    ) -> Result<(Vec<Splice>, Vec<Conflict>), LuaDataError> {
        let (Value::Table(ot), Value::Table(nt)) = (old, new) else {
            return Err(plain("a file's value is a table"));
        };
        if !super::frag::writable(new) {
            return Err(plain("a NaN cannot be written as data"));
        }
        let mut w = Walk {
            doc: self,
            rules,
            rebase,
            splices: Vec::new(),
            tables: FxHashMap::default(),
            conflicts: Vec::new(),
            path: Vec::new(),
            pat: Vec::new(),
        };
        w.table(self.tree.root, ot, nt)?;
        w.finish()
    }

    /// Writes the difference between `old` (the model as it was read) and `new` (the model as
    /// it is now) into the document: new keys and items inserted, removed ones taken out with
    /// their comments, changed values rewritten where they stand, tables recursed. What neither
    /// value names stays as it is. True when the text changed.
    pub fn apply_diff(&mut self, old: &Value, new: &Value, rules: &DiffRules) -> Result<bool, LuaDataError> {
        let (splices, _) = self.walk(old, new, rules, false)?;
        self.commit(splices)
    }

    /// Applies `base -> mine` to `theirs`, a text that changed since `base` was read. Where
    /// `theirs` changed a value `mine` changed too, `mine` is kept and the place is reported.
    pub fn rebase(
        base: &Value,
        mine: &Value,
        theirs: &str,
        rules: &DiffRules,
    ) -> Result<(LuaDoc, Vec<Conflict>), LuaDataError> {
        Self::rebase_with(base, mine, theirs, rules, "", super::Limits::asset())
    }

    /// [`LuaDoc::rebase`] with a chunk name and reader limits.
    pub fn rebase_with(
        base: &Value,
        mine: &Value,
        theirs: &str,
        rules: &DiffRules,
        name: &str,
        limits: super::Limits,
    ) -> Result<(LuaDoc, Vec<Conflict>), LuaDataError> {
        let mut doc = LuaDoc::parse_with(theirs, name, limits)?;
        if data_eq(base, mine) {
            return Ok((doc, Vec::new()));
        }
        let (splices, mut conflicts) = doc.walk(base, mine, rules, true)?;
        doc.commit(splices)?;
        for c in &mut conflicts {
            c.span = Some(doc.nearest_span(&c.path));
        }
        Ok((doc, conflicts))
    }
}
