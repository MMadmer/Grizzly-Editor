//! What the code editor's keys do to a text, as pure functions of the text and the selection:
//! byte offsets in, the new text and the new selection out. The widget turns egui's character
//! cursors into bytes and back; everything here is tested without a window.

use super::CodeLang;

/// A selection: `anchor` where it started, `caret` where the caret is (equal: no selection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sel {
    pub anchor: usize,
    pub caret: usize,
}

impl Sel {
    #[must_use]
    pub fn at(i: usize) -> Self {
        Self { anchor: i, caret: i }
    }

    #[must_use]
    pub fn start(self) -> usize {
        self.anchor.min(self.caret)
    }

    #[must_use]
    pub fn end(self) -> usize {
        self.anchor.max(self.caret)
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.anchor == self.caret
    }
}

/// An edit's outcome: the whole new text and the selection in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    pub sel: Sel,
}

/// Where the line holding byte `i` starts.
#[must_use]
pub fn line_start(t: &str, i: usize) -> usize {
    t[..i.min(t.len())].rfind('\n').map_or(0, |n| n + 1)
}

/// Where the line holding byte `i` ends (its `\n`, or the text's end).
#[must_use]
pub fn line_end(t: &str, i: usize) -> usize {
    let i = i.min(t.len());
    t[i..].find('\n').map_or(t.len(), |n| i + n)
}

/// The 1-based line and column (in characters) of byte `i`.
#[must_use]
pub fn line_col(t: &str, i: usize) -> (u32, u32) {
    let i = i.min(t.len());
    let line = t.as_bytes()[..i].iter().filter(|&&b| b == b'\n').count() + 1;
    let col = t[line_start(t, i)..i].chars().count() + 1;
    (u32::try_from(line).unwrap_or(u32::MAX), u32::try_from(col).unwrap_or(u32::MAX))
}

/// The byte of a 1-based line and column (in characters); past the end, the nearest there is.
#[must_use]
pub fn byte_of(t: &str, line: u32, col: u32) -> usize {
    let mut start = 0usize;
    for _ in 1..line.max(1) {
        match t[start..].find('\n') {
            Some(n) => start += n + 1,
            None => return t.len(),
        }
    }
    let end = line_end(t, start);
    let skip = col.max(1) as usize - 1;
    t[start..end].char_indices().nth(skip).map_or(end, |(k, _)| start + k)
}

/// A byte offset as a character index (what egui's cursors count).
#[must_use]
pub fn char_index(t: &str, byte: usize) -> usize {
    t[..byte.min(t.len())].chars().count()
}

/// A character index as a byte offset.
#[must_use]
pub fn byte_index(t: &str, chars: usize) -> usize {
    t.char_indices().nth(chars).map_or(t.len(), |(b, _)| b)
}

/// The indent a text uses: a tab when its lines start with tabs, else the spaces most of its
/// indented lines step by (four when nothing says).
#[must_use]
pub fn indent_unit(t: &str) -> String {
    let (mut tabs, mut spaces) = (0usize, [0usize; 9]);
    for line in t.lines().take(2000) {
        if line.starts_with('\t') {
            tabs += 1;
        } else {
            let n = line.len() - line.trim_start_matches(' ').len();
            if n > 0 && n < line.len() {
                for (unit, count) in spaces.iter_mut().enumerate().skip(2) {
                    if n % unit == 0 {
                        *count += 1;
                    }
                }
            }
        }
    }
    let spaced = spaces[2];
    if tabs > spaced || (tabs == 0 && spaced == 0) && t.contains('\t') {
        return "\t".to_owned();
    }
    if spaced == 0 {
        return "    ".to_owned();
    }
    // the widest step that most indented lines are a multiple of
    let unit = [8, 4, 3, 2].into_iter().find(|&u| spaces[u] * 10 >= spaced * 9).unwrap_or(2);
    " ".repeat(unit)
}

/// The start of every line the selection touches (a selection that ends at a line's start
/// leaves that line out).
fn touched_lines(t: &str, sel: Sel) -> Vec<usize> {
    let (s, mut e) = (sel.start(), sel.end());
    if e > s && t[..e].ends_with('\n') {
        e -= 1;
    }
    let mut starts = vec![line_start(t, s)];
    let mut i = starts[0];
    while let Some(n) = t[i..].find('\n') {
        let next = i + n + 1;
        if next > e {
            break;
        }
        starts.push(next);
        i = next;
    }
    starts
}

/// Moves a selection over an edit that put `delta` bytes in at `at` (negative: took them out).
fn shift(i: usize, at: usize, delta: isize) -> usize {
    if i < at { i } else { i.saturating_add_signed(delta).max(at) }
}

/// Tab: an indent at the caret (no selection, or one within a line), else one more level for
/// every line the selection touches.
#[must_use]
pub fn indent(t: &str, sel: Sel, unit: &str) -> Edited {
    let within_line = !t[sel.start()..sel.end()].contains('\n');
    if within_line {
        let mut text = String::with_capacity(t.len() + unit.len());
        text.push_str(&t[..sel.start()]);
        text.push_str(unit);
        text.push_str(&t[sel.end()..]);
        return Edited { text, sel: Sel::at(sel.start() + unit.len()) };
    }
    let starts = touched_lines(t, sel);
    let mut text = t.to_owned();
    let mut s = sel;
    for &at in starts.iter().rev() {
        if t[at..line_end(t, at)].trim().is_empty() {
            continue;
        }
        text.insert_str(at, unit);
        let d = unit.len().cast_signed();
        s = Sel { anchor: shift(s.anchor, at, d), caret: shift(s.caret, at, d) };
    }
    // a selection that started at its line's start keeps the indent inside it
    let first = starts[0];
    if s.start() > first && sel.start() == first {
        if s.anchor < s.caret {
            s.anchor = first;
        } else {
            s.caret = first;
        }
    }
    Edited { text, sel: s }
}

/// Shift+Tab: one level less for every line the selection touches (as much of one as a line has).
#[must_use]
pub fn outdent(t: &str, sel: Sel, unit: &str) -> Edited {
    let starts = touched_lines(t, sel);
    let mut text = t.to_owned();
    let mut s = sel;
    for &at in starts.iter().rev() {
        let line = &t[at..line_end(t, at)];
        let n = if line.starts_with('\t') {
            1
        } else {
            let spaces = line.len() - line.trim_start_matches(' ').len();
            spaces.min(unit.len().max(1))
        };
        if n == 0 {
            continue;
        }
        text.replace_range(at..at + n, "");
        let d = -n.cast_signed();
        s = Sel { anchor: shift(s.anchor, at, d), caret: shift(s.caret, at, d) };
    }
    Edited { text, sel: s }
}

/// The comment a language writes a line out with, and the one it closes a block with.
#[must_use]
pub fn comment_marks(lang: CodeLang) -> Option<(&'static str, &'static str)> {
    match lang {
        CodeLang::Lua | CodeLang::LuaData => Some(("--", "")),
        CodeLang::Ltx => Some((";", "")),
        CodeLang::Xml | CodeLang::Markdown => Some(("<!--", "-->")),
        CodeLang::Plain | CodeLang::Json | CodeLang::Diff => None,
    }
}

/// Ctrl+/: comments the lines out, or in again when every one of them is commented.
#[must_use]
pub fn toggle_comment(t: &str, sel: Sel, lang: CodeLang) -> Option<Edited> {
    let (open, close) = comment_marks(lang)?;
    let starts = touched_lines(t, sel);
    let lines: Vec<(usize, usize)> = starts.iter().map(|&s| (s, line_end(t, s))).collect();
    let filled: Vec<&(usize, usize)> = lines.iter().filter(|(a, b)| !t[*a..*b].trim().is_empty()).collect();
    if filled.is_empty() {
        return None;
    }
    let commented = filled.iter().all(|(a, b)| {
        let l = t[*a..*b].trim();
        l.starts_with(open) && l.ends_with(close)
    });
    // the comment goes at the least indent of the lines, so a block stays lined up
    let column =
        filled.iter().map(|(a, b)| t[*a..*b].len() - t[*a..*b].trim_start().len()).min().unwrap_or(0);
    let mut text = t.to_owned();
    let mut s = sel;
    let adjust = |at: usize, d: isize, s: &mut Sel| {
        *s = Sel { anchor: shift(s.anchor, at, d), caret: shift(s.caret, at, d) };
    };
    for &(a, b) in filled.iter().rev() {
        let line = &t[*a..*b];
        if commented {
            let lead = line.len() - line.trim_start().len();
            let body = line.trim();
            let mut inner = &body[open.len()..body.len() - close.len()];
            let (sp_open, sp_close) = (inner.starts_with(' '), !close.is_empty() && inner.ends_with(' '));
            if sp_open {
                inner = &inner[1..];
            }
            if sp_close {
                inner = &inner[..inner.len() - 1];
            }
            let new = format!("{}{}{}", &line[..lead], inner, &line[lead + body.len()..]);
            let removed = line.len() - new.len();
            text.replace_range(*a..*b, &new);
            adjust(*a + lead, -removed.cast_signed(), &mut s);
        } else {
            let head = format!("{open} ");
            let tail = if close.is_empty() { String::new() } else { format!(" {close}") };
            let at = *a + column;
            text.insert_str(*b, &tail);
            text.insert_str(at, &head);
            adjust(at, head.len().cast_signed(), &mut s);
        }
    }
    Some(Edited { text, sel: s })
}

/// Ctrl+D: the lines the selection touches written once more under them; the selection moves
/// with the copy.
#[must_use]
pub fn duplicate(t: &str, sel: Sel) -> Edited {
    let starts = touched_lines(t, sel);
    let first = starts[0];
    let last_end = line_end(t, *starts.last().unwrap_or(&first));
    let block = &t[first..last_end];
    let mut text = String::with_capacity(t.len() + block.len() + 1);
    text.push_str(&t[..last_end]);
    text.push('\n');
    text.push_str(block);
    text.push_str(&t[last_end..]);
    let d = block.len() + 1;
    Edited { text, sel: Sel { anchor: sel.anchor + d, caret: sel.caret + d } }
}

/// Alt+Up / Alt+Down: the lines the selection touches change places with the line above or
/// below; `None` at the text's edge.
#[must_use]
pub fn move_lines(t: &str, sel: Sel, up: bool) -> Option<Edited> {
    let starts = touched_lines(t, sel);
    let first = starts[0];
    let last_end = line_end(t, *starts.last().unwrap_or(&first));
    let block = &t[first..last_end];
    if up {
        if first == 0 {
            return None;
        }
        let above = line_start(t, first - 1);
        let other = &t[above..first - 1];
        let text = format!("{}{}\n{}{}", &t[..above], block, other, &t[last_end..]);
        let d = (first - above).cast_signed();
        Some(Edited {
            text,
            sel: Sel {
                anchor: sel.anchor.saturating_add_signed(-d),
                caret: sel.caret.saturating_add_signed(-d),
            },
        })
    } else {
        if last_end >= t.len() {
            return None;
        }
        let below_end = line_end(t, last_end + 1);
        let other = &t[last_end + 1..below_end];
        let text = format!("{}{}\n{}{}", &t[..first], other, block, &t[below_end..]);
        let d = other.len() + 1;
        Some(Edited { text, sel: Sel { anchor: sel.anchor + d, caret: sel.caret + d } })
    }
}

/// Whether a line opens a block the next line is indented in.
fn opens_block(lang: CodeLang, line: &str) -> bool {
    // what a comment says is not code
    let code = match lang {
        CodeLang::Lua | CodeLang::LuaData => line.find("--").map_or(line, |c| &line[..c]),
        _ => line,
    };
    let code = code.trim_end();
    if code.ends_with(['{', '(', '[']) {
        return true;
    }
    match lang {
        CodeLang::Lua | CodeLang::LuaData => {
            let last = code.rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or("");
            if matches!(last, "then" | "do" | "else" | "repeat") {
                return true;
            }
            // `function f(a)` and `x = function()`: a function opened and not closed on the line
            let opened = code.contains("function") && code.ends_with(')');
            opened && !code.split_whitespace().any(|w| w == "end")
        }
        CodeLang::Xml => {
            code.ends_with('>')
                && !code.ends_with("/>")
                && !code.contains("</")
                && code.trim_start().starts_with('<')
                && !code.trim_start().starts_with("<?")
                && !code.trim_start().starts_with("<!")
        }
        _ => false,
    }
}

/// Enter: a new line with the indent of the caret's line, one level more after a line that
/// opens a block.
#[must_use]
pub fn newline(t: &str, sel: Sel, lang: CodeLang, unit: &str) -> Edited {
    let (s, e) = (sel.start(), sel.end());
    let ls = line_start(t, s);
    let before = &t[ls..s];
    let lead: String = before.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let mut add = String::from("\n");
    add.push_str(&lead);
    if opens_block(lang, before) {
        add.push_str(unit);
    }
    let mut text = String::with_capacity(t.len() + add.len());
    text.push_str(&t[..s]);
    text.push_str(&add);
    text.push_str(&t[e..]);
    Edited { text, sel: Sel::at(s + add.len()) }
}

/// The bracket at the caret (before or after it) and its pair, as byte offsets, when both are
/// there. Brackets inside strings and comments are not counted.
#[must_use]
pub fn bracket_pair(t: &str, caret: usize, quiet: &[(usize, usize)]) -> Option<(usize, usize)> {
    let b = t.as_bytes();
    let in_quiet = |i: usize| quiet.iter().any(|&(s, e)| i >= s && i < e);
    let at = [caret, caret.wrapping_sub(1)]
        .into_iter()
        .find(|&i| i < b.len() && matches!(b[i], b'(' | b')' | b'[' | b']' | b'{' | b'}') && !in_quiet(i))?;
    let (open, close, forward) = match b[at] {
        b'(' => (b'(', b')', true),
        b'[' => (b'[', b']', true),
        b'{' => (b'{', b'}', true),
        b')' => (b'(', b')', false),
        b']' => (b'[', b']', false),
        _ => (b'{', b'}', false),
    };
    let mut depth = 0i32;
    let mut i = at;
    loop {
        let c = b[i];
        if !in_quiet(i) {
            if c == open {
                depth += if forward { 1 } else { -1 };
            } else if c == close {
                depth += if forward { -1 } else { 1 };
            }
            if depth == 0 {
                return Some((at, i));
            }
        }
        if forward {
            i += 1;
            if i >= b.len() {
                return None;
            }
        } else {
            if i == 0 {
                return None;
            }
            i -= 1;
        }
    }
}

/// The name at a byte with what qualifies it: `xr_logic.pick` of `xr_logic.pi|ck` (dotted and
/// `:`-joined parts taken), as its byte range and text. `None` off a name.
#[must_use]
pub fn dotted_at(t: &str, i: usize) -> Option<(usize, usize, String)> {
    let b = t.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut s = i.min(b.len());
    while s > 0 && (word(b[s - 1]) || (matches!(b[s - 1], b'.' | b':') && s >= 2 && word(b[s - 2]))) {
        s -= 1;
    }
    let mut e = i.min(b.len());
    while e < b.len() && word(b[e]) {
        e += 1;
    }
    if s == e || !b[s].is_ascii_alphabetic() && b[s] != b'_' {
        return None;
    }
    Some((s, e, t[s..e].to_owned()))
}

/// What a completion is asked for at the caret: the word typed so far and the name before its
/// dot or colon (`xr_logic` of `xr_logic.pi`), with where the word starts.
#[must_use]
pub fn completion_at(t: &str, caret: usize) -> Option<(usize, Option<String>, String)> {
    let b = t.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let caret = caret.min(b.len());
    let mut s = caret;
    while s > 0 && word(b[s - 1]) {
        s -= 1;
    }
    let prefix = t[s..caret].to_owned();
    if prefix.bytes().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let qualifier = if s > 0 && matches!(b[s - 1], b'.' | b':') {
        let mut q = s - 1;
        while q > 0 && word(b[q - 1]) {
            q -= 1;
        }
        (q < s - 1).then(|| t[q..s - 1].to_owned())
    } else {
        None
    };
    Some((s, qualifier, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret_of(marked: &str) -> (String, Sel) {
        // `|` is the caret, `[` .. `]` a selection with the caret at its end
        if let (Some(a), Some(c)) = (marked.find('['), marked.find(']')) {
            let t = marked.replacen('[', "", 1).replacen(']', "", 1);
            return (t, Sel { anchor: a, caret: c - 1 });
        }
        let at = marked.find('|').expect("a caret");
        (marked.replacen('|', "", 1), Sel::at(at))
    }

    #[test]
    fn lines_and_columns_go_both_ways() {
        let t = "ab\nцд e\n";
        assert_eq!(line_col(t, 0), (1, 1));
        let e = t.find('e').unwrap();
        assert_eq!(line_col(t, e), (2, 4));
        assert_eq!(byte_of(t, 2, 4), e);
        assert_eq!(byte_of(t, 9, 1), t.len());
        assert_eq!(byte_index(t, char_index(t, e)), e);
    }

    #[test]
    fn the_indent_is_the_files() {
        assert_eq!(indent_unit("a\n\tb\n\t\tc\n"), "\t");
        assert_eq!(indent_unit("a\n  b\n    c\n  d\n"), "  ");
        assert_eq!(indent_unit("a\n    b\n        c\n"), "    ");
        assert_eq!(indent_unit("no indent"), "    ");
    }

    #[test]
    fn tab_and_shift_tab_move_lines() {
        let (t, s) = caret_of("[a\nb\n]c");
        let e = indent(&t, s, "  ");
        assert_eq!(e.text, "  a\n  b\nc");
        let back = outdent(&e.text, e.sel, "  ");
        assert_eq!(back.text, t);
        let (t, s) = caret_of("x|y");
        assert_eq!(indent(&t, s, "\t").text, "x\ty");
        assert_eq!(outdent("\t\tq", Sel::at(2), "\t").text, "\tq");
    }

    #[test]
    fn comments_go_in_and_out() {
        let (t, s) = caret_of("  [a = 1\n    b = 2]\n");
        let on = toggle_comment(&t, s, CodeLang::Lua).unwrap();
        assert_eq!(on.text, "  -- a = 1\n  --   b = 2\n");
        let off =
            toggle_comment(&on.text, Sel { anchor: 0, caret: on.text.len() - 1 }, CodeLang::Lua).unwrap();
        assert_eq!(off.text, t);
        let x = toggle_comment("<a/>", Sel::at(0), CodeLang::Xml).unwrap();
        assert_eq!(x.text, "<!-- <a/> -->");
        assert_eq!(toggle_comment(&x.text, Sel::at(0), CodeLang::Xml).unwrap().text, "<a/>");
        assert!(toggle_comment("x", Sel::at(0), CodeLang::Json).is_none());
    }

    #[test]
    fn duplicate_and_move() {
        let (t, s) = caret_of("a\nb|\nc");
        assert_eq!(duplicate(&t, s).text, "a\nb\nb\nc");
        assert_eq!(move_lines(&t, s, true).unwrap().text, "b\na\nc");
        assert_eq!(move_lines(&t, s, false).unwrap().text, "a\nc\nb");
        assert!(move_lines("a\nb", Sel::at(0), true).is_none());
        assert!(move_lines("a\nb", Sel::at(3), false).is_none());
    }

    #[test]
    fn enter_keeps_and_opens_the_indent() {
        let (t, s) = caret_of("\tif x then|\nend");
        let e = newline(&t, s, CodeLang::Lua, "\t");
        assert_eq!(e.text, "\tif x then\n\t\t\nend");
        let (t, s) = caret_of("  local t = {|");
        assert_eq!(newline(&t, s, CodeLang::Lua, "  ").text, "  local t = {\n    ");
        let (t, s) = caret_of("  x = 1 -- then|");
        assert_eq!(newline(&t, s, CodeLang::Lua, "  ").text, "  x = 1 -- then\n  ");
        let (t, s) = caret_of("function f(a)|");
        assert_eq!(newline(&t, s, CodeLang::Lua, "\t").text, "function f(a)\n\t");
        let (t, s) = caret_of("local f = function() return 1 end|");
        assert_eq!(newline(&t, s, CodeLang::Lua, "\t").text, "local f = function() return 1 end\n");
    }

    #[test]
    fn brackets_pair_up() {
        let t = "f(a[1], \")\", {b})";
        let q = t.find('"').unwrap();
        let quiet = [(q, q + 3)];
        assert_eq!(bracket_pair(t, 1, &quiet), Some((1, t.len() - 1)));
        assert_eq!(bracket_pair(t, t.len(), &quiet), Some((t.len() - 1, 1)));
        // a bracket in a string is no bracket: unquieted it pairs with the wrong one
        assert_ne!(bracket_pair(t, 1, &[]), Some((1, t.len() - 1)));
        assert_eq!(bracket_pair("(", 0, &[]), None);
    }

    #[test]
    fn names_and_completions_at_the_caret() {
        let t = "x = xr_logic.pick_section(npc)";
        let at = t.find("ck_").unwrap();
        assert_eq!(dotted_at(t, at).map(|d| d.2), Some("xr_logic.pick_section".to_owned()));
        assert_eq!(dotted_at("  ", 1), None);
        let (s, q, p) = completion_at("a = db.act", 10).unwrap();
        assert_eq!((s, q.as_deref(), p.as_str()), (7, Some("db"), "act"));
        let (_, q, p) = completion_at("obj:", 4).unwrap();
        assert_eq!((q.as_deref(), p.as_str()), (Some("obj"), ""));
        assert!(completion_at("x = 12", 6).is_none());
    }
}
