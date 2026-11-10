//! The browser's selection: file-manager semantics over entries, folders and files alike. Click
//! replaces, Ctrl toggles, Shift takes the range from the anchor in the complete view order.

use gz_base::SmolStr;
use rustc_hash::FxHashSet;

/// An entry of the view: a folder and a file are the same thing to the selection.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    pub path: SmolStr,
    pub folder: bool,
}

impl Entry {
    #[must_use]
    pub fn new(path: &str, folder: bool) -> Self {
        Self { path: SmolStr::new(path), folder }
    }

    /// How MCP names it: a folder with a trailing `\`.
    #[must_use]
    pub fn wire(&self) -> String {
        if self.folder { format!("{}\\", self.path) } else { self.path.to_string() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Selection {
    order: Vec<Entry>,
    lookup: FxHashSet<Entry>,
    anchor: Option<Entry>,
}

impl Selection {
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.order
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    #[must_use]
    pub fn contains(&self, e: &Entry) -> bool {
        self.lookup.contains(e)
    }

    #[must_use]
    pub fn anchor(&self) -> Option<&Entry> {
        self.anchor.as_ref()
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.lookup.clear();
        self.anchor = None;
    }

    pub fn add(&mut self, e: Entry) {
        if self.lookup.insert(e.clone()) {
            self.order.push(e);
        }
    }

    /// One entry alone, the anchor on it.
    pub fn only(&mut self, e: Entry) {
        self.clear();
        self.add(e.clone());
        self.anchor = Some(e);
    }

    /// Ctrl: in or out; the anchor stays on something that is still selected.
    pub fn toggle(&mut self, e: Entry) {
        if self.lookup.remove(&e) {
            self.order.retain(|x| x != &e);
            if self.anchor.as_ref() == Some(&e) {
                self.anchor = self.order.last().cloned();
            }
            return;
        }
        self.add(e.clone());
        self.anchor = Some(e);
    }

    /// Shift: everything from the anchor to `to` in `view`; the anchor stays put. `false` when
    /// the anchor or the target is not in the view.
    pub fn range(&mut self, view: &[Entry], to: &Entry) -> bool {
        let Some(anchor) = self.anchor.clone() else { return false };
        let from = view.iter().position(|e| e == &anchor);
        let at = view.iter().position(|e| e == to);
        let (Some(a), Some(b)) = (from, at) else { return false };
        let (lo, hi) = (a.min(b), a.max(b));
        self.order.clear();
        self.lookup.clear();
        for e in view.get(lo..=hi).unwrap_or(&[]) {
            self.add(e.clone());
        }
        true
    }

    pub fn all(&mut self, view: &[Entry]) {
        self.clear();
        for e in view {
            self.add(e.clone());
        }
        self.anchor = self.order.last().cloned();
    }

    /// Replaces the whole selection (a marquee); the anchor goes to the last entry.
    pub fn set(&mut self, entries: Vec<Entry>) {
        self.clear();
        for e in entries {
            self.add(e);
        }
        self.anchor = self.order.last().cloned();
    }

    /// Keeps what `exists` still knows.
    pub fn prune(&mut self, exists: impl Fn(&Entry) -> bool) {
        self.order.retain(&exists);
        self.lookup = self.order.iter().cloned().collect();
        if self.anchor.as_ref().is_some_and(|a| !self.lookup.contains(a)) {
            self.anchor = self.order.last().cloned();
        }
    }

    /// Paths below `was` (or it) now live below `now`: a folder renamed.
    pub fn rename(&mut self, was: &str, now: &str) {
        let fix = |e: &mut Entry| {
            if crate::model::is_ancestor(was, &e.path) {
                e.path = SmolStr::new(format!("{now}{}", e.path.get(was.len()..).unwrap_or("")));
            }
        };
        for e in &mut self.order {
            fix(e);
        }
        if let Some(a) = &mut self.anchor {
            fix(a);
        }
        self.lookup = self.order.iter().cloned().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(p: &str) -> Entry {
        Entry::new(p, false)
    }

    #[test]
    fn click_ctrl_shift() {
        let view = vec![e("a"), e("b"), e("c"), e("d")];
        let mut s = Selection::default();
        s.only(e("b"));
        s.toggle(e("d"));
        assert_eq!(s.len(), 2);
        s.toggle(e("d"));
        assert_eq!(s.anchor(), Some(&e("b")));
        assert!(s.range(&view, &e("d")));
        assert_eq!(s.entries(), &[e("b"), e("c"), e("d")]);
        // the anchor stays: back to the start
        assert!(s.range(&view, &e("a")));
        assert_eq!(s.len(), 2);
        assert!(!s.range(&view, &e("zz")));
        s.prune(|x| x.path != "a");
        assert_eq!(s.entries(), &[e("b")]);
        s.rename("b", "q");
        assert!(s.contains(&e("q")));
        assert_eq!(Entry::new("f", true).wire(), "f\\");
    }
}
