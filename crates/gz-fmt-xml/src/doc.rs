//! The tolerant XML tree and the navigation of `CXml`.

use std::borrow::Cow;

use crate::cnum::{atof, atoi};

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Document,
    Element,
    /// Text, condensed and decoded; CDATA as written.
    Text,
    Comment,
    /// `<?xml ... ?>`.
    Declaration,
    /// `<!DOCTYPE ...>`, `<?pi ...?>` and the like.
    Unknown,
}

/// A node of a [`Document`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

const NONE: u32 = u32::MAX;

#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    /// An element's name, a text's text, a comment's content.
    value: Box<str>,
    attrs: Vec<(Box<str>, Box<str>)>,
    parent: u32,
    first: u32,
    last: u32,
    next: u32,
}

/// A defect the reader read past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlWarning {
    pub line: u32,
    pub message: String,
}

/// A parsed XML file: immutable, shared between threads.
#[derive(Clone, Debug)]
pub struct Document {
    nodes: Vec<Node>,
    utf8: bool,
    warnings: Vec<XmlWarning>,
}

/// The text of a file by its declaration: UTF-8 when it has a BOM or says
/// `encoding="utf-8"`, cp1251 otherwise (the game's files say `windows-1251`).
#[must_use]
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if let Some(body) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(body);
    }
    if declares_utf8(bytes) {
        return String::from_utf8_lossy(bytes);
    }
    gz_base::text::cp1251_to_utf8(bytes)
}

/// Whether the declaration at the start names UTF-8.
fn declares_utf8(b: &[u8]) -> bool {
    let head = b.get(..b.len().min(256)).unwrap_or(&[]);
    let lower: Vec<u8> = head.to_ascii_lowercase();
    let Some(at) = lower.windows(5).position(|w| w == b"<?xml") else { return false };
    let end =
        lower.get(at..).and_then(|r| r.windows(2).position(|w| w == b"?>")).map_or(lower.len(), |e| at + e);
    let decl = lower.get(at..end).unwrap_or(&[]);
    decl.windows(5).any(|w| w == b"utf-8") || decl.windows(4).any(|w| w == b"utf8")
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

fn name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':') || !c.is_ascii()
}

/// The attributes of a tag as written.
type Attrs = Vec<(Box<str>, Box<str>)>;

struct Parser<'a> {
    s: &'a str,
    pos: usize,
    nodes: Vec<Node>,
    utf8: bool,
    warnings: Vec<XmlWarning>,
    line_at: usize,
    line: u32,
}

const MAX_WARNINGS: usize = 1000;

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        self.s.get(self.pos..).unwrap_or("")
    }

    fn line_of(&mut self, pos: usize) -> u32 {
        if pos < self.line_at {
            self.line_at = 0;
            self.line = 1;
        }
        let end = pos.min(self.s.len());
        self.line += u32::try_from(
            self.s.as_bytes().get(self.line_at..end).map_or(0, |r| r.iter().filter(|&&c| c == b'\n').count()),
        )
        .unwrap_or(0);
        self.line_at = end;
        self.line
    }

    fn warn(&mut self, pos: usize, message: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            let line = self.line_of(pos);
            self.warnings.push(XmlWarning { line, message: message.into() });
        }
    }

    fn add(&mut self, parent: u32, kind: Kind, value: String, attrs: Vec<(Box<str>, Box<str>)>) -> u32 {
        let id = u32::try_from(self.nodes.len()).unwrap_or(NONE);
        self.nodes.push(Node {
            kind,
            value: value.into(),
            attrs,
            parent,
            first: NONE,
            last: NONE,
            next: NONE,
        });
        let last = self.nodes.get(parent as usize).map_or(NONE, |p| p.last);
        if let Some(l) = self.nodes.get_mut(last as usize) {
            l.next = id;
        }
        if let Some(p) = self.nodes.get_mut(parent as usize) {
            if p.first == NONE {
                p.first = id;
            }
            p.last = id;
        }
        id
    }

    /// `GetEntity`: the character an `&...;` stands for, and how many bytes it took.
    fn entity(&self, at: &str) -> Option<(char, usize)> {
        if let Some(num) = at.strip_prefix("&#") {
            let semi = num.find(';')?;
            let body = num.get(..semi)?;
            let ucs = if let Some(hex) = body.strip_prefix('x') {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                body.parse::<u32>().ok()?
            };
            let c = if self.utf8 {
                char::from_u32(ucs)?
            } else {
                // a legacy document keeps the low byte, as cp1251
                let byte = [(ucs & 0xff) as u8];
                gz_base::text::cp1251_to_utf8(&byte).chars().next()?
            };
            return Some((c, 2 + semi + 1));
        }
        for (name, c) in [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>'), ("&quot;", '"'), ("&apos;", '\'')] {
            if at.starts_with(name) {
                return Some((c, name.len()));
            }
        }
        None
    }

    /// Text up to `end`, entities decoded, condensed when `condense` (`ReadText`).
    fn read_text(&mut self, end: &str, condense: bool) -> String {
        let mut out = String::new();
        let mut white = false;
        if condense {
            while self.rest().starts_with(is_ws) {
                self.pos += 1;
            }
        }
        loop {
            let rest = self.rest();
            if rest.is_empty() || rest.starts_with(end) {
                break;
            }
            let Some(c) = rest.chars().next() else { break };
            if condense && is_ws(c) {
                white = true;
                self.pos += c.len_utf8();
                continue;
            }
            if white {
                out.push(' ');
                white = false;
            }
            if c == '&' {
                match self.entity(rest) {
                    Some((ch, n)) => {
                        out.push(ch);
                        self.pos += n;
                    }
                    None => {
                        if rest.starts_with("&#") {
                            let at = self.pos;
                            self.warn(at, "A character reference is malformed; it is kept as written.");
                        }
                        out.push('&');
                        self.pos += 1;
                    }
                }
                continue;
            }
            out.push(c);
            self.pos += c.len_utf8();
        }
        out
    }

    fn read_name(&mut self) -> Option<String> {
        let rest = self.rest();
        let first = rest.chars().next().filter(|&c| name_start(c))?;
        let len = rest.char_indices().skip(1).find(|&(_, c)| !name_char(c)).map_or(rest.len(), |(i, _)| i);
        let _ = first;
        let name = rest.get(..len)?.to_owned();
        self.pos += len;
        Some(name)
    }

    fn skip_ws(&mut self) {
        while self.rest().starts_with(is_ws) {
            self.pos += 1;
        }
    }

    /// Attributes up to `>`, `/>` or `?>`. Returns them and whether the tag closed itself.
    fn read_attrs(&mut self, tag: &str) -> (Attrs, bool) {
        let mut attrs: Attrs = Vec::new();
        loop {
            self.skip_ws();
            let rest = self.rest();
            if rest.is_empty() {
                let at = self.pos;
                self.warn(at, format!("The tag <{tag}> is cut off by the end of the file."));
                return (attrs, true);
            }
            if rest.starts_with("/>") || rest.starts_with("?>") {
                self.pos += 2;
                return (attrs, true);
            }
            if rest.starts_with('>') {
                self.pos += 1;
                return (attrs, false);
            }
            let at = self.pos;
            let Some(name) = self.read_name() else {
                let c = rest.chars().next().map_or(1, char::len_utf8);
                self.warn(at, format!("A stray character in the tag <{tag}> is skipped."));
                self.pos += c;
                continue;
            };
            self.skip_ws();
            if !self.rest().starts_with('=') {
                self.warn(at, format!("The attribute '{name}' of <{tag}> has no value."));
                continue;
            }
            self.pos += 1;
            self.skip_ws();
            let value = match self.rest().chars().next() {
                Some(q @ ('"' | '\'')) => {
                    self.pos += 1;
                    let v = self.read_text(&q.to_string(), false);
                    if self.rest().starts_with(q) {
                        self.pos += 1;
                    } else {
                        self.warn(at, format!("The value of '{name}' in <{tag}> is not closed."));
                    }
                    v
                }
                _ => {
                    // TinyXML reads an unquoted value up to a blank or the end of the tag
                    let rest = self.rest();
                    let len = rest.find(|c: char| is_ws(c) || c == '>' || c == '/').unwrap_or(rest.len());
                    let v = rest.get(..len).unwrap_or("").to_owned();
                    self.pos += len;
                    self.warn(at, format!("The value of '{name}' in <{tag}> is not in quotes."));
                    v
                }
            };
            if let Some(slot) = attrs.iter_mut().find(|(k, _)| **k == *name) {
                // TinyXML takes the second value and stops reading the file here
                self.warn(
                    at,
                    format!("The attribute '{name}' appears twice in <{tag}>; the last value is kept."),
                );
                slot.1 = value.into();
            } else {
                attrs.push((name.into(), value.into()));
            }
        }
    }

    fn parse(&mut self) {
        let mut stack: Vec<u32> = Vec::new();
        if self.rest().starts_with('\u{feff}') {
            self.pos += '\u{feff}'.len_utf8();
            self.utf8 = true;
        }
        loop {
            let parent = stack.last().copied().unwrap_or(0);
            let rest = self.rest();
            if rest.is_empty() {
                break;
            }
            let at = self.pos;
            if !rest.starts_with('<') {
                let text = self.read_text("<", true);
                if !text.is_empty() {
                    if stack.is_empty() {
                        self.warn(at, "Text outside the root element is ignored.");
                    } else {
                        self.add(parent, Kind::Text, text, Vec::new());
                    }
                }
                continue;
            }
            if let Some(body) = rest.strip_prefix("<!--") {
                let (content, used) = match body.find("-->") {
                    Some(e) => (body.get(..e).unwrap_or(""), 4 + e + 3),
                    None => {
                        self.warn(at, "A comment is not closed.");
                        (body, rest.len())
                    }
                };
                let content = content.to_owned();
                self.pos += used;
                self.add(parent, Kind::Comment, content, Vec::new());
                continue;
            }
            if let Some(body) = rest.strip_prefix("<![CDATA[") {
                let (content, used) = match body.find("]]>") {
                    Some(e) => (body.get(..e).unwrap_or(""), 9 + e + 3),
                    None => {
                        self.warn(at, "A CDATA section is not closed.");
                        (body, rest.len())
                    }
                };
                let content = content.to_owned();
                self.pos += used;
                self.add(parent, Kind::Text, content, Vec::new());
                continue;
            }
            if rest.len() >= 5 && rest.get(..5).is_some_and(|h| h.eq_ignore_ascii_case("<?xml")) {
                self.pos += 5;
                let (attrs, _) = self.read_attrs("?xml");
                if let Some((_, enc)) = attrs.iter().find(|(k, _)| &**k == "encoding") {
                    let e = enc.to_ascii_lowercase();
                    if e == "utf-8" || e == "utf8" {
                        self.utf8 = true;
                    }
                }
                self.add(parent, Kind::Declaration, String::new(), attrs);
                continue;
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                let close = if rest.starts_with("<?") { "?>" } else { ">" };
                let (content, used) = match rest.find(close) {
                    Some(e) => (rest.get(1..e).unwrap_or(""), e + close.len()),
                    None => {
                        self.warn(at, "A declaration is not closed.");
                        (rest.get(1..).unwrap_or(""), rest.len())
                    }
                };
                let content = content.to_owned();
                self.pos += used;
                self.add(parent, Kind::Unknown, content, Vec::new());
                continue;
            }
            if let Some(body) = rest.strip_prefix("</") {
                let end = body.find('>');
                let name = end.and_then(|e| body.get(..e)).unwrap_or(body).trim().to_owned();
                self.pos += 2 + end.map_or(body.len(), |e| e + 1);
                match stack
                    .iter()
                    .rposition(|&e| self.nodes.get(e as usize).is_some_and(|n| *n.value == *name))
                {
                    Some(k) => {
                        if k + 1 != stack.len() {
                            let inner: Vec<String> = stack
                                .iter()
                                .skip(k + 1)
                                .filter_map(|&e| self.nodes.get(e as usize).map(|n| n.value.to_string()))
                                .collect();
                            for n in inner {
                                self.warn(at, format!("The element <{n}> is not closed."));
                            }
                        }
                        stack.truncate(k);
                    }
                    None => {
                        self.warn(at, format!("The closing tag </{name}> closes nothing; it is ignored."))
                    }
                }
                continue;
            }
            self.pos += 1;
            let Some(name) = self.read_name() else {
                self.warn(at, "A '<' that starts no tag is read as text.");
                if !stack.is_empty() {
                    let mut text = String::from("<");
                    text.push_str(&self.read_text("<", true));
                    self.add(parent, Kind::Text, text, Vec::new());
                }
                continue;
            };
            let (attrs, closed) = self.read_attrs(&name);
            if stack.is_empty() && self.nodes.iter().skip(1).any(|n| n.parent == 0 && n.kind == Kind::Element)
            {
                self.warn(at, format!("A second root element <{name}> follows the first."));
            }
            let id = self.add(parent, Kind::Element, name, attrs);
            if !closed {
                stack.push(id);
            }
        }
        for e in stack {
            let n = self.nodes.get(e as usize).map(|n| n.value.to_string()).unwrap_or_default();
            let end = self.s.len();
            self.warn(end, format!("The element <{n}> is not closed at the end of the file."));
        }
    }
}

impl Document {
    /// Reads a document. It never fails; see [`Document::warnings`].
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let utf8 = declares_utf8(text.as_bytes());
        let mut p = Parser {
            s: text,
            pos: 0,
            nodes: vec![Node {
                kind: Kind::Document,
                value: "".into(),
                attrs: Vec::new(),
                parent: NONE,
                first: NONE,
                last: NONE,
                next: NONE,
            }],
            utf8,
            warnings: Vec::new(),
            line_at: 0,
            line: 1,
        };
        p.parse();
        Document { nodes: p.nodes, utf8: p.utf8, warnings: p.warnings }
    }

    /// What the reader read past.
    #[must_use]
    pub fn warnings(&self) -> &[XmlWarning] {
        &self.warnings
    }

    /// Whether the document declared UTF-8 (numeric references decode as Unicode then).
    #[must_use]
    pub fn is_utf8(&self) -> bool {
        self.utf8
    }

    fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)
    }

    /// The document node.
    #[must_use]
    pub fn document(&self) -> NodeId {
        NodeId(0)
    }

    /// The first element of the document (`m_root`).
    #[must_use]
    pub fn root(&self) -> Option<NodeId> {
        self.children(self.document()).find(|&c| self.kind(c) == Some(Kind::Element))
    }

    #[must_use]
    pub fn kind(&self, id: NodeId) -> Option<Kind> {
        self.node(id).map(|n| n.kind)
    }

    /// An element's name, a text's text, a comment's content (`Value()`).
    #[must_use]
    pub fn value(&self, id: NodeId) -> &str {
        self.node(id).map_or("", |n| &n.value)
    }

    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).map(|n| n.parent).filter(|&p| p != NONE).map(NodeId)
    }

    /// The children in order.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let first = self.node(id).map_or(NONE, |n| n.first);
        std::iter::successors((first != NONE).then_some(NodeId(first)), move |c| {
            self.node(*c).map(|n| n.next).filter(|&x| x != NONE).map(NodeId)
        })
    }

    /// An element's attributes in order.
    #[must_use]
    pub fn attributes(&self, id: NodeId) -> &[(Box<str>, Box<str>)] {
        self.node(id).map_or(&[], |n| &n.attrs)
    }

    /// `FirstChild(value)`: the first child whose value is `value`; any child for `None`.
    #[must_use]
    pub fn first_child(&self, id: NodeId, value: Option<&str>) -> Option<NodeId> {
        self.children(id).find(|&c| value.is_none_or(|v| self.value(c) == v))
    }

    /// `NextSibling(value)`.
    #[must_use]
    pub fn next_sibling(&self, id: NodeId, value: Option<&str>) -> Option<NodeId> {
        let mut c = self.node(id).map(|n| n.next).filter(|&x| x != NONE).map(NodeId);
        while let Some(x) = c {
            if value.is_none_or(|v| self.value(x) == v) {
                return Some(x);
            }
            c = self.node(x).map(|n| n.next).filter(|&y| y != NONE).map(NodeId);
        }
        None
    }

    /// `NavigateToNode`: `path` is `a:b:c`; the first step takes the `index`-th child named
    /// `a`, each further step the first child of that name.
    #[must_use]
    pub fn navigate(&self, start: NodeId, path: &str, index: usize) -> Option<NodeId> {
        let mut tokens = path.split(':').filter(|t| !t.is_empty());
        let first = tokens.next()?;
        let mut node = self.first_child(start, Some(first));
        for _ in 0..index {
            node = self.next_sibling(node?, Some(first));
        }
        for t in tokens {
            node = self.first_child(node?, Some(t));
        }
        node
    }

    /// `NavigateToNodeWithAttribute`: the first child `tag` of `start` whose `attr` is `value`.
    #[must_use]
    pub fn navigate_with_attr(&self, start: NodeId, tag: &str, attr: &str, value: &str) -> Option<NodeId> {
        let n = self.count(Some(start), Some(tag));
        (0..n).find_map(|i| {
            let node = self.navigate(start, tag, i);
            (self.attr(node, attr) == Some(value)).then_some(node).flatten()
        })
    }

    /// `SearchForAttribute`: depth first over `start` and its siblings named `tag`.
    #[must_use]
    pub fn search_for_attribute(
        &self,
        start: Option<NodeId>,
        tag: &str,
        attr: &str,
        value: &str,
    ) -> Option<NodeId> {
        // an explicit stack: hostile nesting must not exhaust the thread's
        let mut stack: Vec<NodeId> = start.into_iter().collect();
        while let Some(node) = stack.pop() {
            if self.kind(node) == Some(Kind::Element)
                && self.value(node) == tag
                && self.attr(Some(node), attr) == Some(value)
            {
                return Some(node);
            }
            if let Some(next) = self.next_sibling(node, Some(tag)) {
                stack.push(next);
            }
            if let Some(child) = self.first_child(node, Some(tag)) {
                stack.push(child);
            }
        }
        None
    }

    /// `GetNodesNum`: the children named `tag` (all children for `None`).
    #[must_use]
    pub fn count(&self, node: Option<NodeId>, tag: Option<&str>) -> usize {
        node.map_or(0, |n| self.children(n).filter(|&c| tag.is_none_or(|t| self.value(c) == t)).count())
    }

    /// `Read`: the text of a node's first child, when that child is text.
    #[must_use]
    pub fn read(&self, node: Option<NodeId>) -> Option<&str> {
        let c = self.first_child(node?, None)?;
        (self.kind(c) == Some(Kind::Text)).then(|| self.value(c))
    }

    #[must_use]
    pub fn read_or<'a>(&'a self, node: Option<NodeId>, default: &'a str) -> &'a str {
        self.read(node).unwrap_or(default)
    }

    #[must_use]
    pub fn read_int(&self, node: Option<NodeId>, default: i32) -> i32 {
        self.read(node).map_or(default, atoi)
    }

    #[must_use]
    pub fn read_float(&self, node: Option<NodeId>, default: f32) -> f32 {
        self.read(node).map_or(default, |s| atof(s) as f32)
    }

    /// `ReadAttrib`: an element's attribute.
    #[must_use]
    pub fn attr(&self, node: Option<NodeId>, name: &str) -> Option<&str> {
        let n = self.node(node?)?;
        if n.kind != Kind::Element {
            return None;
        }
        n.attrs.iter().find(|(k, _)| &**k == name).map(|(_, v)| &**v)
    }

    #[must_use]
    pub fn attr_or<'a>(&'a self, node: Option<NodeId>, name: &str, default: &'a str) -> &'a str {
        self.attr(node, name).unwrap_or(default)
    }

    #[must_use]
    pub fn attr_int(&self, node: Option<NodeId>, name: &str, default: i32) -> i32 {
        self.attr(node, name).map_or(default, atoi)
    }

    #[must_use]
    pub fn attr_float(&self, node: Option<NodeId>, name: &str, default: f32) -> f32 {
        self.attr(node, name).map_or(default, |s| atof(s) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_navigates() {
        let d = Document::parse(
            "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\r\n<w>\r\n<!-- c -->\r\n<a id=\"1\">  one \r\n two  </a>\r\n<a id=\"2\"><b>x &amp; y &#65;</b></a>\r\n<n v=\"3.5\">7</n></w>",
        );
        assert!(d.warnings().is_empty(), "{:?}", d.warnings());
        let root = d.root().unwrap();
        assert_eq!(d.value(root), "w");
        assert_eq!(d.count(Some(root), Some("a")), 2);
        assert_eq!(d.read(d.navigate(root, "a", 0)), Some("one two"));
        assert_eq!(d.read(d.navigate(root, "a:b", 1)), Some("x & y A"));
        assert_eq!(d.navigate(root, "a", 2), None);
        assert_eq!(d.attr(d.navigate(root, "a", 1), "id"), Some("2"));
        assert_eq!(d.read_int(d.navigate(root, "n", 0), 0), 7);
        assert_eq!(d.attr_float(d.navigate(root, "n", 0), "v", 0.0), 3.5);
        assert_eq!(d.attr_or(d.navigate(root, "n", 0), "zz", "d"), "d");
        let a2 = d.navigate_with_attr(root, "a", "id", "2");
        assert_eq!(a2, d.navigate(root, "a", 1));
        assert_eq!(d.search_for_attribute(d.first_child(root, Some("a")), "a", "id", "2"), a2);
        assert_eq!(d.count(Some(root), Some(" c ")), 1);
    }

    #[test]
    fn reads_past_defects() {
        let d = Document::parse("<w a=\"1\" a=\"2\" b=3><x><y>t</x>&#xZZ; <z/></w></q>stray");
        let root = d.root().unwrap();
        assert_eq!(d.attr(Some(root), "a"), Some("2"));
        assert_eq!(d.attr(Some(root), "b"), Some("3"));
        assert_eq!(d.read(d.navigate(root, "x:y", 0)), Some("t"));
        assert!(d.navigate(root, "z", 0).is_some());
        let msgs: Vec<&str> = d.warnings().iter().map(|w| w.message.as_str()).collect();
        assert_eq!(msgs.len(), 6, "{msgs:?}");
    }

    #[test]
    fn decodes_by_declaration() {
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, b'a']), "a");
        assert_eq!(
            decode(b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\xd0\x96"),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>Ж"
        );
        assert_eq!(decode(b"<a>\xc6</a>"), "<a>Ж</a>");
        let d = Document::parse("<?xml encoding=\"utf-8\"?><a>&#1046;</a>");
        assert_eq!(d.read(d.root()), Some("Ж"));
        let d = Document::parse("<a>&#198;</a>");
        assert_eq!(d.read(d.root()), Some("Ж"));
    }
}
