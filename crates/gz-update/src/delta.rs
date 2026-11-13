//! Which bytes of the archive a client asks for: the records of the files it does not hold,
//! merged into one request wherever two lie close together.

/// Two records closer than this are one request: the bytes between them cost less than a request.
pub const GAP: u64 = 256 * 1024;

/// Ranges `[start, end)` sorted and merged wherever the gap between two is under `gap`.
#[must_use]
pub fn merge(mut spans: Vec<(u64, u64)>, gap: u64) -> Vec<(u64, u64)> {
    spans.retain(|(a, b)| b > a);
    spans.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a < last.1.saturating_add(gap) => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The bytes the ranges hold.
#[must_use]
pub fn total(spans: &[(u64, u64)]) -> u64 {
    spans.iter().map(|(a, b)| b - a).sum()
}

/// Whether reading the delta is not worth it: 90 % of the archive or more.
#[must_use]
pub fn whole_is_better(delta: u64, archive: u64) -> bool {
    u128::from(delta) * 10 >= u128::from(archive) * 9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_records_are_one_request() {
        let k = 1024;
        let spans =
            vec![(900 * k, 1000 * k), (0, 10 * k), (10 * k, 20 * k), (300 * k, 310 * k), (575 * k, 600 * k)];
        // 20 K..300 K is 280 K apart: two requests; 310 K..575 K is 265 K: apart too
        assert_eq!(
            merge(spans.clone(), GAP),
            vec![(0, 20 * k), (300 * k, 310 * k), (575 * k, 600 * k), (900 * k, 1000 * k)]
        );
        // with a wider gap everything within it joins, the bytes between included
        assert_eq!(merge(spans, 300 * k), vec![(0, 600 * k), (900 * k, 1000 * k)]);
        assert_eq!(merge(vec![(5, 5), (1, 3), (2, 4)], 0), vec![(1, 4)]);
        assert_eq!(total(&[(0, 10), (20, 25)]), 15);
        // exactly GAP apart is two requests
        assert_eq!(merge(vec![(0, 1), (1 + GAP, 2 + GAP)], GAP).len(), 2);
        assert_eq!(merge(vec![(0, 1), (GAP, 2 + GAP)], GAP).len(), 1);
    }

    #[test]
    fn ninety_percent_takes_the_whole_archive() {
        assert!(whole_is_better(90, 100));
        assert!(!whole_is_better(89, 100));
        assert!(whole_is_better(u64::MAX, u64::MAX));
    }
}
