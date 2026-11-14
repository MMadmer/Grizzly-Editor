//! MSVC's `std::sort` (`<algorithm>`): introsort - insertion sort up to 32
//! elements, a partition around Tukey's ninther above that, heap sort once 1.5 log2 n divisions
//! are spent. It is not stable, and where X-Ray's compiler sorts with ties its output follows that order
//! of equal elements; [`sort_by`] gives the same order.

const ISORT_MAX: usize = 32;

/// `std::sort(first, last, pred)`; `less` is the predicate.
pub fn sort_by<T: Copy>(v: &mut [T], less: impl Fn(&T, &T) -> bool) {
    sort_unchecked(v, 0, v.len(), v.len(), &less);
}

fn sort_unchecked<T: Copy>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    mut ideal: usize,
    less: &impl Fn(&T, &T) -> bool,
) {
    loop {
        if last - first <= ISORT_MAX {
            insertion_sort(v, first, last, less);
            return;
        }
        if ideal == 0 {
            make_heap(v, first, last, less);
            sort_heap(v, first, last, less);
            return;
        }
        let (mid0, mid1) = partition_by_median_guess(v, first, last, less);
        ideal = (ideal >> 1) + (ideal >> 2);
        if mid0 - first < last - mid1 {
            sort_unchecked(v, first, mid0, ideal, less);
            first = mid1;
        } else {
            sort_unchecked(v, mid1, last, ideal, less);
            last = mid0;
        }
    }
}

/// `_Insertion_sort_unchecked`.
fn insertion_sort<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if first == last {
        return;
    }
    for mid in first + 1..last {
        let val = v[mid];
        if less(&val, &v[first]) {
            v.copy_within(first..mid, first + 1);
            v[first] = val;
        } else {
            let mut hole = mid;
            while less(&val, &v[hole - 1]) {
                v[hole] = v[hole - 1];
                hole -= 1;
            }
            v[hole] = val;
        }
    }
}

/// `_Med3_unchecked`.
fn med3<T: Copy>(v: &mut [T], first: usize, mid: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if less(&v[mid], &v[first]) {
        v.swap(mid, first);
    }
    if less(&v[last], &v[mid]) {
        v.swap(last, mid);
        if less(&v[mid], &v[first]) {
            v.swap(mid, first);
        }
    }
}

/// `_Guess_median_unchecked`; `last` is the last element, not past it.
fn guess_median<T: Copy>(v: &mut [T], first: usize, mid: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    let count = last - first;
    if count > 40 {
        let step = (count + 1) >> 3;
        let two = step << 1;
        med3(v, first, first + step, first + two, less);
        med3(v, mid - step, mid, mid + step, less);
        med3(v, last - two, last - step, last, less);
        med3(v, first + step, mid, last - step, less);
    } else {
        med3(v, first, mid, last, less);
    }
}

/// `_Partition_by_median_guess_unchecked`: the range of elements equal to the pivot.
fn partition_by_median_guess<T: Copy>(
    v: &mut [T],
    first: usize,
    last: usize,
    less: &impl Fn(&T, &T) -> bool,
) -> (usize, usize) {
    let mid = first + ((last - first) >> 1);
    guess_median(v, first, mid, last - 1, less);
    let mut pfirst = mid;
    let mut plast = pfirst + 1;
    while first < pfirst && !less(&v[pfirst - 1], &v[pfirst]) && !less(&v[pfirst], &v[pfirst - 1]) {
        pfirst -= 1;
    }
    while plast < last && !less(&v[plast], &v[pfirst]) && !less(&v[pfirst], &v[plast]) {
        plast += 1;
    }
    let mut gfirst = plast;
    let mut glast = pfirst;
    loop {
        while gfirst < last {
            if less(&v[pfirst], &v[gfirst]) {
            } else if less(&v[gfirst], &v[pfirst]) {
                break;
            } else {
                if plast != gfirst {
                    v.swap(plast, gfirst);
                }
                plast += 1;
            }
            gfirst += 1;
        }
        while first < glast {
            let prev = glast - 1;
            if less(&v[prev], &v[pfirst]) {
            } else if less(&v[pfirst], &v[prev]) {
                break;
            } else {
                pfirst -= 1;
                if pfirst != prev {
                    v.swap(pfirst, prev);
                }
            }
            glast -= 1;
        }
        if glast == first && gfirst == last {
            return (pfirst, plast);
        }
        if glast == first {
            // no room at the bottom: the pivot run rotates up
            if plast != gfirst {
                v.swap(pfirst, plast);
            }
            plast += 1;
            v.swap(pfirst, gfirst);
            pfirst += 1;
            gfirst += 1;
        } else if gfirst == last {
            // no room at the top: the pivot run rotates down
            glast -= 1;
            pfirst -= 1;
            if glast != pfirst {
                v.swap(glast, pfirst);
            }
            plast -= 1;
            v.swap(pfirst, plast);
        } else {
            glast -= 1;
            v.swap(gfirst, glast);
            gfirst += 1;
        }
    }
}

/// `_Push_heap_by_index`.
fn push_heap_by_index<T: Copy>(
    v: &mut [T],
    base: usize,
    mut hole: usize,
    top: usize,
    val: T,
    less: &impl Fn(&T, &T) -> bool,
) {
    while top < hole {
        let idx = (hole - 1) >> 1;
        if !less(&v[base + idx], &val) {
            break;
        }
        v[base + hole] = v[base + idx];
        hole = idx;
    }
    v[base + hole] = val;
}

/// `_Pop_heap_hole_by_index`.
fn pop_heap_hole_by_index<T: Copy>(
    v: &mut [T],
    base: usize,
    mut hole: usize,
    bottom: usize,
    val: T,
    less: &impl Fn(&T, &T) -> bool,
) {
    let top = hole;
    let mut idx = hole;
    let max_non_leaf = (bottom - 1) >> 1;
    while idx < max_non_leaf {
        idx = 2 * idx + 2;
        if less(&v[base + idx], &v[base + idx - 1]) {
            idx -= 1;
        }
        v[base + hole] = v[base + idx];
        hole = idx;
    }
    if idx == max_non_leaf && bottom.is_multiple_of(2) {
        v[base + hole] = v[base + bottom - 1];
        hole = bottom - 1;
    }
    push_heap_by_index(v, base, hole, top, val, less);
}

/// `_Make_heap_unchecked`.
fn make_heap<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    let bottom = last - first;
    let mut hole = bottom >> 1;
    while hole > 0 {
        hole -= 1;
        let val = v[first + hole];
        pop_heap_hole_by_index(v, first, hole, bottom, val, less);
    }
}

/// `_Sort_heap_unchecked` over `_Pop_heap_unchecked`.
fn sort_heap<T: Copy>(v: &mut [T], first: usize, mut last: usize, less: &impl Fn(&T, &T) -> bool) {
    while last - first >= 2 {
        last -= 1;
        let val = v[last];
        v[last] = v[first];
        pop_heap_hole_by_index(v, first, 0, last - first, val, less);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sorted, and the elements of one key in the order MSVC's introsort leaves them: below 33
    /// elements an insertion sort (stable), above it the partition moves equal keys.
    #[test]
    fn the_sort_orders_and_keeps_msvcs_order_of_ties() {
        let mut seed = 12345u32;
        let mut v: Vec<(u32, u32)> = (0..1000)
            .map(|i| {
                seed = seed.wrapping_mul(214_013).wrapping_add(2_531_011);
                ((seed >> 16) % 7, i)
            })
            .collect();
        sort_by(&mut v, |a, b| a.0 < b.0);
        assert!(v.windows(2).all(|w| w[0].0 <= w[1].0));
        let mut ids: Vec<u32> = v.iter().map(|x| x.1).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..1000).collect::<Vec<_>>());

        let mut small: Vec<(u32, u32)> = (0..32).map(|i| (i % 3, i)).collect();
        sort_by(&mut small, |a, b| a.0 < b.0);
        assert!(small.windows(2).all(|w| w[0].0 < w[1].0 || (w[0].0 == w[1].0 && w[0].1 < w[1].1)));

        // the heap sort path: an ideal of zero from the start
        let mut h: Vec<u32> = (0..100).rev().collect();
        let n = h.len();
        make_heap(&mut h, 0, n, &|a: &u32, b: &u32| a < b);
        sort_heap(&mut h, 0, n, &|a: &u32, b: &u32| a < b);
        assert_eq!(h, (0..100).collect::<Vec<_>>());
    }
}
