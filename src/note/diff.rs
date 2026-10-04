//! A Myers longest common subsequence over any slice, and a unified line diff built on it.

const CONTEXT: usize = 3;

/// The index pairs `(i, j)` of one longest common subsequence of `a` and `b`, ascending in both.
/// Linear space: the middle snake splits the problem until a side is empty.
pub fn lcs<T: Eq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    solve(a, b, 0, 0, &mut out);
    out
}

fn solve<T: Eq>(a: &[T], b: &[T], ai: usize, bi: usize, out: &mut Vec<(usize, usize)>) {
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    out.extend((0..head).map(|k| (ai + k, bi + k)));
    let (a, b) = (&a[head..], &b[head..]);
    let (ai, bi) = (ai + head, bi + head);
    let tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a_mid, b_mid) = (&a[..a.len() - tail], &b[..b.len() - tail]);
    if !a_mid.is_empty() && !b_mid.is_empty() {
        let (x, y, u, v) = middle_snake(a_mid, b_mid);
        solve(&a_mid[..x], &b_mid[..y], ai, bi, out);
        out.extend((0..u - x).map(|k| (ai + x + k, bi + y + k)));
        solve(&a_mid[u..], &b_mid[v..], ai + u, bi + v, out);
    }
    let (a_end, b_end) = (ai + a_mid.len(), bi + b_mid.len());
    out.extend((0..tail).map(|k| (a_end + k, b_end + k)));
}

/// The middle snake `(x, y)..(u, v)` of two non-empty slices with no common first or last element: the diagonal run
/// where the forward and the reverse search first meet. Each side of it has a shorter edit script than the whole.
fn middle_snake<T: Eq>(a: &[T], b: &[T]) -> (usize, usize, usize, usize) {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let delta = n - m;
    let odd = delta % 2 != 0;
    let half = (n + m + 1) / 2;
    let size = 2 * half as usize + 3;
    let at = |k: isize| (k + half + 1) as usize;
    let mut fwd = vec![0isize; size];
    let mut rev = vec![0isize; size];
    for d in 0..=half {
        for k in (-d..=d).step_by(2) {
            let mut x = if k == -d || (k != d && fwd[at(k - 1)] < fwd[at(k + 1)]) {
                fwd[at(k + 1)]
            } else {
                fwd[at(k - 1)] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            fwd[at(k)] = x;
            let kr = delta - k;
            if odd && (-(d - 1)..=d - 1).contains(&kr) && x + rev[at(kr)] >= n {
                return (x0 as usize, y0 as usize, x as usize, y as usize);
            }
        }
        for k in (-d..=d).step_by(2) {
            let mut x = if k == -d || (k != d && rev[at(k - 1)] < rev[at(k + 1)]) {
                rev[at(k + 1)]
            } else {
                rev[at(k - 1)] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[(n - 1 - x) as usize] == b[(m - 1 - y) as usize] {
                x += 1;
                y += 1;
            }
            rev[at(k)] = x;
            let kf = delta - k;
            if !odd && (-d..=d).contains(&kf) && x + fwd[at(kf)] >= n {
                return (
                    (n - x) as usize,
                    (m - y) as usize,
                    (n - x0) as usize,
                    (m - y0) as usize,
                );
            }
        }
    }
    unreachable!("two slices always meet within (n + m) / 2 edits")
}

#[derive(Clone, Copy)]
enum Op {
    Keep,
    Delete,
    Insert,
}

/// A unified diff from `old` to `new` with 3 lines of context, under the given header names; empty when the two are
/// equal. A line is compared with its newline, so a missing final newline is a difference.
pub fn unified(old_name: &str, new_name: &str, old: &str, new: &str) -> String {
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    let ops = script(&a, &b);
    if ops.iter().all(|(op, _)| matches!(op, Op::Keep)) {
        return String::new();
    }
    // `starts[k]` is how many old and new lines come before op `k`.
    let mut starts = Vec::with_capacity(ops.len() + 1);
    let (mut old_seen, mut new_seen) = (0, 0);
    for (op, _) in &ops {
        starts.push((old_seen, new_seen));
        match op {
            Op::Keep => {
                old_seen += 1;
                new_seen += 1;
            }
            Op::Delete => old_seen += 1,
            Op::Insert => new_seen += 1,
        }
    }
    starts.push((old_seen, new_seen));

    let mut out = format!("--- {old_name}\n+++ {new_name}\n");
    for (first, last) in hunks(&ops) {
        let (old_from, new_from) = starts[first];
        let (old_to, new_to) = starts[last];
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            range(old_from, old_to - old_from),
            range(new_from, new_to - new_from)
        ));
        for (op, line) in &ops[first..last] {
            out.push(match op {
                Op::Keep => ' ',
                Op::Delete => '-',
                Op::Insert => '+',
            });
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    out
}

/// `start,count` with a 1-based start; an empty range starts at the line before it, and a count of 1 is left out.
fn range(from: usize, count: usize) -> String {
    match count {
        0 => format!("{from},0"),
        1 => format!("{}", from + 1),
        _ => format!("{},{count}", from + 1),
    }
}

fn script<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(Op, &'a str)> {
    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (mi, mj) in lcs(a, b).into_iter().chain([(a.len(), b.len())]) {
        ops.extend(a[i..mi].iter().map(|line| (Op::Delete, *line)));
        ops.extend(b[j..mj].iter().map(|line| (Op::Insert, *line)));
        if mi < a.len() {
            ops.push((Op::Keep, a[mi]));
        }
        (i, j) = (mi + 1, mj + 1);
    }
    ops
}

/// The op ranges `[first, last)` of each hunk: every change with `CONTEXT` kept lines around it, merged when two
/// changes sit no more than twice that apart.
fn hunks(ops: &[(Op, &str)]) -> Vec<(usize, usize)> {
    let changed = |k: usize| !matches!(ops[k].0, Op::Keep);
    let mut hunks = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        if !changed(k) {
            k += 1;
            continue;
        }
        let first = k.saturating_sub(CONTEXT);
        let mut end = k;
        loop {
            while end < ops.len() && changed(end) {
                end += 1;
            }
            let kept = ops[end..]
                .iter()
                .take_while(|(op, _)| matches!(op, Op::Keep))
                .count();
            if end + kept < ops.len() && kept <= 2 * CONTEXT {
                end += kept;
            } else {
                end += kept.min(CONTEXT);
                break;
            }
        }
        hunks.push((first, end));
        k = end;
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("line {i}")).collect()
    }

    fn diff(old: &[String], new: &[String]) -> String {
        let join = |lines: &[String]| lines.iter().map(|l| format!("{l}\n")).collect::<String>();
        unified("a", "b", &join(old), &join(new))
    }

    /// The length of a longest common subsequence by the quadratic table.
    fn table(a: &[u8], b: &[u8]) -> usize {
        let mut row = vec![0; b.len() + 1];
        for x in a {
            let mut diag = 0;
            for (j, y) in b.iter().enumerate() {
                let up = row[j + 1];
                row[j + 1] = if x == y {
                    diag + 1
                } else {
                    row[j + 1].max(row[j])
                };
                diag = up;
            }
        }
        row[b.len()]
    }

    #[test]
    fn lcs_of_empty_sides() {
        assert!(lcs::<u8>(&[], &[]).is_empty());
        assert!(lcs(&[1, 2], &[]).is_empty());
        assert!(lcs(&[], &[1, 2]).is_empty());
    }

    #[test]
    fn lcs_matches_the_table_on_random_input() {
        let mut seed = 0x2545f491u32;
        let mut next = move |bound: u32| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) % bound
        };
        for _ in 0..3000 {
            let alphabet = 2 + next(4) as u8;
            let a: Vec<u8> = (0..next(14)).map(|_| next(alphabet as u32) as u8).collect();
            let b: Vec<u8> = (0..next(14)).map(|_| next(alphabet as u32) as u8).collect();
            let pairs = lcs(&a, &b);
            assert_eq!(pairs.len(), table(&a, &b), "{a:?} {b:?}");
            assert!(
                pairs.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1),
                "{pairs:?}"
            );
            assert!(pairs.iter().all(|&(i, j)| a[i] == b[j]), "{a:?} {b:?}");
        }
    }

    #[test]
    fn identical_input_has_no_diff() {
        let lines = numbered(5);
        assert_eq!(diff(&lines, &lines), "");
        assert_eq!(unified("a", "b", "", ""), "");
    }

    #[test]
    fn an_empty_side_is_all_added_or_all_removed() {
        assert_eq!(
            unified("a", "b", "", "x\ny\n"),
            "--- a\n+++ b\n@@ -0,0 +1,2 @@\n+x\n+y\n"
        );
        assert_eq!(
            unified("a", "b", "x\ny\n", ""),
            "--- a\n+++ b\n@@ -1,2 +0,0 @@\n-x\n-y\n"
        );
    }

    #[test]
    fn an_insertion() {
        let old = numbered(10);
        let mut new = old.clone();
        new.insert(5, "added".into());
        assert_eq!(
            diff(&old, &new),
            "--- a\n+++ b\n@@ -3,6 +3,7 @@\n line 3\n line 4\n line 5\n+added\n line 6\n line 7\n line 8\n"
        );
    }

    #[test]
    fn a_deletion() {
        let old = numbered(10);
        let mut new = old.clone();
        new.remove(5);
        assert_eq!(
            diff(&old, &new),
            "--- a\n+++ b\n@@ -3,7 +3,6 @@\n line 3\n line 4\n line 5\n-line 6\n line 7\n line 8\n line 9\n"
        );
    }

    #[test]
    fn a_change_at_either_end() {
        let old = numbered(10);
        let mut first = old.clone();
        first[0] = "new first".into();
        assert_eq!(
            diff(&old, &first),
            "--- a\n+++ b\n@@ -1,4 +1,4 @@\n-line 1\n+new first\n line 2\n line 3\n line 4\n"
        );
        let mut last = old.clone();
        last[9] = "new last".into();
        assert_eq!(
            diff(&old, &last),
            "--- a\n+++ b\n@@ -7,4 +7,4 @@\n line 7\n line 8\n line 9\n-line 10\n+new last\n"
        );
    }

    #[test]
    fn hunks_merge_when_their_context_overlaps() {
        let old = numbered(30);
        let mut near = old.clone();
        near[9] = "x".into();
        near[16] = "y".into();
        let merged = diff(&old, &near);
        assert_eq!(merged.matches("@@ -").count(), 1, "{merged}");
        assert!(merged.contains("@@ -7,14 +7,14 @@"), "{merged}");

        let mut far = old.clone();
        far[9] = "x".into();
        far[17] = "y".into();
        let split = diff(&old, &far);
        assert_eq!(split.matches("@@ -").count(), 2, "{split}");
    }

    #[test]
    fn a_missing_final_newline_is_a_difference() {
        assert_eq!(
            unified("a", "b", "x\ny", "x\ny\n"),
            "--- a\n+++ b\n@@ -1,2 +1,2 @@\n x\n-y\n\\ No newline at end of file\n+y\n"
        );
    }

    #[test]
    fn ten_thousand_lines_with_one_change_are_fast() {
        let old = numbered(10_000);
        let mut new = old.clone();
        new[5_000] = "changed".into();
        let start = std::time::Instant::now();
        let out = diff(&old, &new);
        assert!(start.elapsed() < std::time::Duration::from_millis(50));
        assert!(out.contains("-line 5001\n+changed\n"), "{out}");
    }
}
