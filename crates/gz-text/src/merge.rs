//! Lines: what two texts have in common, and the three-way merge of two changes of one text.
//!
//! The common lines are a longest common subsequence (Myers' O(ND) difference, in linear space,
//! after the common head and tail are trimmed and the lines only one side has are set aside):
//! a small edit of a large file costs a pass over it. A difference that would take longer than
//! a budget of steps is cut short there - what is left is read as changed, never as the same.

use rustc_hash::FxHashMap;

/// How many steps of the search one difference may take before what is left of it is read as
/// changed: some hundred milliseconds of work, for two large files that share hardly a line.
const BUDGET: u64 = 50_000_000;

/// The lines `old` and `new` share, as pairs of their indices in order: a longest common
/// subsequence of the two.
#[must_use]
pub fn same_lines(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[head..].iter().rev().zip(new[head..].iter().rev()).take_while(|(a, b)| a == b).count();
    let mut out: Vec<(usize, usize)> = (0..head).map(|i| (i, i)).collect();
    let (a, b) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
    if !a.is_empty() && !b.is_empty() {
        // lines as numbers: a line only one side has can be in no common subsequence
        let mut ids: FxHashMap<&str, (u32, bool, bool)> = FxHashMap::default();
        let mut next = 0u32;
        for (side, lines) in [(0, a), (1, b)] {
            for line in lines {
                let e = ids.entry(line).or_insert_with(|| {
                    next += 1;
                    (next, false, false)
                });
                if side == 0 {
                    e.1 = true;
                } else {
                    e.2 = true;
                }
            }
        }
        let shared = |l: &str| ids.get(l).filter(|e| e.1 && e.2).map(|e| e.0);
        let (ai, an): (Vec<usize>, Vec<u32>) =
            a.iter().enumerate().filter_map(|(i, l)| shared(l).map(|id| (i, id))).unzip();
        let (bi, bn): (Vec<usize>, Vec<u32>) =
            b.iter().enumerate().filter_map(|(i, l)| shared(l).map(|id| (i, id))).unzip();
        let mut mid = Vec::new();
        let mut budget = BUDGET;
        lcs(&an, &bn, 0, 0, &mut mid, &mut budget);
        out.extend(mid.into_iter().map(|(x, y)| (head + ai[x], head + bi[y])));
    }
    out.extend((0..tail).map(|i| (old.len() - tail + i, new.len() - tail + i)));
    out
}

/// The common subsequence of `a` and `b` (offset by `ao`, `bo`), appended in order.
fn lcs(a: &[u32], b: &[u32], ao: usize, bo: usize, out: &mut Vec<(usize, usize)>, budget: &mut u64) {
    let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    out.extend((0..p).map(|i| (ao + i, bo + i)));
    let (a, b) = (&a[p..], &b[p..]);
    let s = a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[..a.len() - s], &b[..b.len() - s]);
    if !am.is_empty()
        && !bm.is_empty()
        && let Some((x, y)) = bisect(am, bm, budget)
    {
        lcs(&am[..x], &bm[..y], ao + p, bo + p, out, budget);
        lcs(&am[x..], &bm[y..], ao + p + x, bo + p + y, out, budget);
    }
    out.extend((0..s).map(|i| (ao + p + am.len() + i, bo + p + bm.len() + i)));
}

/// Where the forward and the backward shortest paths of the difference of `a` and `b` meet: a
/// point on an optimal path that splits the problem in two. `None` when the two share nothing
/// (or the budget is spent).
fn bisect(a: &[u32], b: &[u32], budget: &mut u64) -> Option<(usize, usize)> {
    let (n, m) = (isize::try_from(a.len()).ok()?, isize::try_from(b.len()).ok()?);
    let max_d = (n + m + 1) / 2;
    let offset = max_d;
    let len = usize::try_from(2 * max_d + 2).ok()?;
    let mut v1 = vec![-1isize; len];
    let mut v2 = vec![-1isize; len];
    let at = |k: isize| usize::try_from(k).ok();
    *v1.get_mut(at(offset + 1)?)? = 0;
    *v2.get_mut(at(offset + 1)?)? = 0;
    let delta = n - m;
    let front = delta % 2 != 0;
    let (mut k1start, mut k1end, mut k2start, mut k2end) = (0isize, 0isize, 0isize, 0isize);
    let same = |x: isize, y: isize| a.get(at(x)?).zip(b.get(at(y)?)).map(|(p, q)| p == q);
    for d in 0..max_d {
        let mut k1 = -d + k1start;
        while k1 <= d - k1end {
            let k = offset + k1;
            let below = v1.get(at(k - 1)?).copied().unwrap_or(-1);
            let above = v1.get(at(k + 1)?).copied().unwrap_or(-1);
            let mut x1 = if k1 == -d || (k1 != d && below < above) { above } else { below + 1 };
            let mut y1 = x1 - k1;
            let mut steps = 1u64;
            while x1 < n && y1 < m && same(x1, y1) == Some(true) {
                x1 += 1;
                y1 += 1;
                steps += 1;
            }
            *budget = budget.checked_sub(steps)?;
            *v1.get_mut(at(k)?)? = x1;
            if x1 > n {
                k1end += 2;
            } else if y1 > m {
                k1start += 2;
            } else if front {
                let k2 = offset + delta - k1;
                if let Some(&x2) = at(k2).and_then(|k2| v2.get(k2))
                    && x2 != -1
                    && x1 >= n - x2
                {
                    return Some((at(x1)?, at(y1)?));
                }
            }
            k1 += 2;
        }
        let mut k2 = -d + k2start;
        while k2 <= d - k2end {
            let k = offset + k2;
            let below = v2.get(at(k - 1)?).copied().unwrap_or(-1);
            let above = v2.get(at(k + 1)?).copied().unwrap_or(-1);
            let mut x2 = if k2 == -d || (k2 != d && below < above) { above } else { below + 1 };
            let mut y2 = x2 - k2;
            let mut steps = 1u64;
            while x2 < n && y2 < m && same(n - x2 - 1, m - y2 - 1) == Some(true) {
                x2 += 1;
                y2 += 1;
                steps += 1;
            }
            *budget = budget.checked_sub(steps)?;
            *v2.get_mut(at(k)?)? = x2;
            if x2 > n {
                k2end += 2;
            } else if y2 > m {
                k2start += 2;
            } else if !front {
                let k1 = offset + delta - k2;
                if let Some(&x1) = at(k1).and_then(|k1| v1.get(k1))
                    && x1 != -1
                {
                    let y1 = offset + x1 - k1;
                    if x1 >= n - x2 {
                        return Some((at(x1)?, at(y1)?));
                    }
                }
            }
            k2 += 2;
        }
    }
    None
}

/// A change of one side: base lines `[s, e)` became that side's lines `[from, to)`.
#[derive(Clone, Copy, Debug)]
struct Hunk {
    s: usize,
    e: usize,
    from: usize,
    to: usize,
    mine: bool,
}

/// The changes that turn `base` into `side`, in order.
fn hunks(base: &[&str], side: &[&str], mine: bool) -> Vec<Hunk> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let ends = same_lines(base, side).into_iter().chain(std::iter::once((base.len(), side.len())));
    for (x, y) in ends {
        if x > i || y > j {
            out.push(Hunk { s: i, e: x, from: j, to: y, mine });
        }
        (i, j) = (x + 1, y + 1);
    }
    out
}

/// Whether two changes of `base` lines touch the same lines: ranges that overlap, two
/// insertions at one place, an insertion inside the other's lines.
fn overlap((s1, e1): (usize, usize), (s2, e2): (usize, usize)) -> bool {
    match (s1 == e1, s2 == e2) {
        (true, true) => s1 == s2,
        (true, false) => s2 < s1 && s1 < e2,
        (false, true) => s1 < s2 && s2 < e1,
        (false, false) => s1 < e2 && s2 < e1,
    }
}

/// What one side made of the base lines `[s, e)`: its changes there, the base lines between them.
fn side_text(base: &[&str], lines: &[&str], hunks: &[&Hunk], (s, e): (usize, usize), out: &mut String) {
    let mut at = s;
    for h in hunks {
        for line in base.get(at..h.s).unwrap_or(&[]) {
            out.push_str(line);
        }
        for line in lines.get(h.from..h.to).unwrap_or(&[]) {
            out.push_str(line);
        }
        at = h.e;
    }
    for line in base.get(at..e).unwrap_or(&[]) {
        out.push_str(line);
    }
}

/// `mine`'s and `theirs`' changes of `base` together, line by line: a region only one of them
/// changed takes that change, one both changed alike takes it once, one they changed apart keeps
/// `mine` and is counted. Answers the text and the count of such regions.
#[must_use]
pub fn merge3(base: &str, mine: &str, theirs: &str) -> (String, usize) {
    if mine == theirs || base == theirs {
        return (mine.to_owned(), 0);
    }
    if base == mine {
        return (theirs.to_owned(), 0);
    }
    // a line holds its own line break: a last line without one differs from the same text with it
    let b: Vec<&str> = base.split_inclusive('\n').collect();
    let m: Vec<&str> = mine.split_inclusive('\n').collect();
    let t: Vec<&str> = theirs.split_inclusive('\n').collect();
    let mut all = hunks(&b, &m, true);
    all.extend(hunks(&b, &t, false));
    all.sort_by_key(|h| (h.s, h.e, !h.mine));
    let mut out = String::with_capacity(mine.len().max(theirs.len()));
    let mut conflicts = 0;
    let mut at = 0;
    let mut i = 0;
    while let Some(first) = all.get(i) {
        let (mut rs, mut re) = (first.s, first.e);
        let mut j = i + 1;
        while let Some(h) = all.get(j) {
            if !overlap((rs, re), (h.s, h.e)) {
                break;
            }
            (rs, re) = (rs.min(h.s), re.max(h.e));
            j += 1;
        }
        let region = all.get(i..j).unwrap_or(&[]);
        for line in b.get(at..rs).unwrap_or(&[]) {
            out.push_str(line);
        }
        let ours: Vec<&Hunk> = region.iter().filter(|h| h.mine).collect();
        let others: Vec<&Hunk> = region.iter().filter(|h| !h.mine).collect();
        if others.is_empty() {
            side_text(&b, &m, &ours, (rs, re), &mut out);
        } else if ours.is_empty() {
            side_text(&b, &t, &others, (rs, re), &mut out);
        } else {
            let mut a = String::new();
            side_text(&b, &m, &ours, (rs, re), &mut a);
            let mut z = String::new();
            side_text(&b, &t, &others, (rs, re), &mut z);
            if a != z {
                conflicts += 1;
            }
            out.push_str(&a);
        }
        at = re;
        i = j;
    }
    for line in b.get(at..).unwrap_or(&[]) {
        out.push_str(line);
    }
    (out, conflicts)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<&str> {
        s.split('\n').collect()
    }

    /// The length of a longest common subsequence, by the table (small inputs only).
    fn lcs_len(a: &[&str], b: &[&str]) -> usize {
        let mut t = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                t[i][j] = if a[i] == b[j] { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
            }
        }
        t[0][0]
    }

    #[test]
    fn the_same_lines_are_a_longest_common_subsequence() {
        let cases = [
            ("a\nb\nc", "a\nb\nc"),
            ("a\nb\nc", "a\nx\nc"),
            ("", "a"),
            ("a\nb", ""),
            ("a\nb\nc\nd\ne\nf", "b\nx\nd\ne\ny\nf\nz"),
            ("x\ny\nx\ny\nx", "y\nx\ny\nx\ny"),
            ("1\n2\n3\n4\n5\n6\n7\n8", "8\n7\n6\n5\n4\n3\n2\n1"),
            ("end\nend\na\nend\nb\nend", "end\na\nend\nend\nb\nb\nend"),
        ];
        for (x, y) in cases {
            let (a, b) = (lines(x), lines(y));
            let pairs = same_lines(&a, &b);
            assert_eq!(pairs.len(), lcs_len(&a, &b), "{x:?} / {y:?}: {pairs:?}");
            for w in pairs.windows(2) {
                assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1, "{pairs:?}");
            }
            for &(i, j) in &pairs {
                assert_eq!(a[i], b[j]);
            }
        }
    }

    #[test]
    fn a_small_edit_of_a_large_file_is_quick() {
        let old: Vec<String> = (0..200_000).map(|i| format!("line {i}")).collect();
        let mut new = old.clone();
        new[100_000] = "changed".to_owned();
        new.insert(150_000, "added".to_owned());
        let a: Vec<&str> = old.iter().map(String::as_str).collect();
        let b: Vec<&str> = new.iter().map(String::as_str).collect();
        let t = std::time::Instant::now();
        assert_eq!(same_lines(&a, &b).len(), 199_999);
        assert!(t.elapsed().as_secs() < 2);
        // two files that share nothing are no work either
        let other: Vec<String> = (0..200_000).map(|i| format!("other {i}")).collect();
        let c: Vec<&str> = other.iter().map(String::as_str).collect();
        assert!(same_lines(&a, &c).is_empty());
    }

    #[test]
    fn disjoint_edits_merge() {
        let base = "a\nb\nc\nd\ne\n";
        assert_eq!(merge3(base, "A\nb\nc\nd\ne\n", "a\nb\nc\nd\nE\n"), ("A\nb\nc\nd\nE\n".to_owned(), 0));
        // edits on neighbouring lines are no conflict
        assert_eq!(merge3(base, "a\nB\nc\nd\ne\n", "a\nb\nC\nd\ne\n"), ("a\nB\nC\nd\ne\n".to_owned(), 0));
    }

    #[test]
    fn the_same_edit_is_taken_once() {
        let base = "a\nb\nc\n";
        assert_eq!(merge3(base, "a\nX\nc\nmine\n", "a\nX\nc\n"), ("a\nX\nc\nmine\n".to_owned(), 0));
        assert_eq!(merge3(base, "a\nc\n", "a\nc\n"), ("a\nc\n".to_owned(), 0));
    }

    #[test]
    fn a_conflict_keeps_mine_and_is_counted() {
        let base = "a\nb\nc\nd\n";
        let (text, n) = merge3(base, "a\nMINE\nc\nd\n", "a\nTHEIRS\nc\nD\n");
        assert_eq!((text.as_str(), n), ("a\nMINE\nc\nD\n", 1));
        let (text, n) = merge3("1\n2\n3\n4\n5\n", "1\nm\n3\n4\nm\n", "1\nt\n3\n4\nt\n");
        assert_eq!((text.as_str(), n), ("1\nm\n3\n4\nm\n", 2));
    }

    #[test]
    fn insertions_at_one_place() {
        let base = "a\nb\n";
        // the same insertion once, two different ones keep mine
        assert_eq!(merge3(base, "a\nx\nb\n", "a\nx\nb\n").0, "a\nx\nb\n");
        assert_eq!(merge3(base, "a\nx\nb\n", "a\ny\nb\n"), ("a\nx\nb\n".to_owned(), 1));
        // insertions at two places both stay
        assert_eq!(merge3(base, "x\na\nb\n", "a\nb\ny\n"), ("x\na\nb\ny\n".to_owned(), 0));
        // an insertion beside a change of the other side
        assert_eq!(merge3(base, "a\nx\nb\n", "a\nB\n"), ("a\nx\nB\n".to_owned(), 0));
    }

    #[test]
    fn deletions() {
        let base = "a\nb\nc\nd\n";
        assert_eq!(merge3(base, "a\nc\nd\n", "a\nb\nc\n"), ("a\nc\n".to_owned(), 0));
        // a line one side deleted and the other changed: mine
        assert_eq!(merge3(base, "a\nc\nd\n", "a\nB\nc\nd\n"), ("a\nc\nd\n".to_owned(), 1));
    }

    #[test]
    fn empty_texts_and_the_last_line_break() {
        assert_eq!(merge3("", "", ""), (String::new(), 0));
        assert_eq!(merge3("", "a\n", ""), ("a\n".to_owned(), 0));
        assert_eq!(merge3("", "", "b\n"), ("b\n".to_owned(), 0));
        assert_eq!(merge3("", "a\n", "b\n"), ("a\n".to_owned(), 1));
        assert_eq!(merge3("a\nb\n", "", "a\nb\n"), (String::new(), 0));
        // a line break added at the end by one side, a line changed by the other
        assert_eq!(merge3("a\nb", "A\nb", "a\nb\n"), ("A\nb\n".to_owned(), 0));
    }
}
