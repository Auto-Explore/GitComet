//! Helpers for the blame/annotate column: per-line lookup with run collapsing,
//! recency normalization, and author-initial derivation.
//!
//! The blame data itself (`Vec<BlameLine>`) is produced by the existing
//! `LoadBlame` pipeline and stored in `history_state.blame`. These helpers only
//! shape it for rendering in the diff/file-content canvas.

use gitcomet_core::services::BlameLine;

/// A blame line resolved for a specific displayed row, plus whether it is the
/// first line of a consecutive run attributed to the same commit. The text
/// portion of the annotation (time/author/summary) is only painted on run
/// starts; interior lines show just the recency border.
pub(in crate::view) struct BlameAnnotation<'a> {
    pub(in crate::view) line: &'a BlameLine,
    pub(in crate::view) is_run_start: bool,
}

/// Resolve the blame entry for a displayed row by its new-side (1-based) line
/// number. Rows without a new-side line (pure deletions) return `None`. For the
/// file-content view, line numbers map 1:1 onto the blamed file.
///
/// `prev_new_line` is the new-side line number of the previous *rendered* blamed
/// row (or `None` if there was none). A row starts a new attribution run — and
/// shows the commit text — unless the previous rendered blamed line is exactly
/// file line `n - 1` and shares this line's commit. Comparing against the
/// previous *rendered* line (rather than blindly against `lines[idx - 1]`)
/// ensures the first visible line of a hunk is treated as a run start even when
/// the hidden preceding file line happens to share its commit.
pub(in crate::view) fn blame_for_new_line(
    lines: &[BlameLine],
    new_line: Option<u32>,
    prev_new_line: Option<u32>,
) -> Option<BlameAnnotation<'_>> {
    let n = new_line? as usize;
    if n == 0 {
        return None;
    }
    let idx = n - 1;
    let line = lines.get(idx)?;
    let is_run_start = prev_new_line != Some((n - 1) as u32)
        || idx
            .checked_sub(1)
            .and_then(|i| lines.get(i))
            .is_none_or(|prev| prev.commit_id != line.commit_id);
    Some(BlameAnnotation { line, is_run_start })
}

/// Smallest and largest `author_time_unix` across the loaded blame set, used to
/// normalize per-line recency. Lines without a timestamp are ignored.
pub(in crate::view) fn blame_time_range(lines: &[BlameLine]) -> Option<(i64, i64)> {
    let mut min = i64::MAX;
    let mut max = i64::MIN;
    for line in lines {
        if let Some(ts) = line.author_time_unix {
            min = min.min(ts);
            max = max.max(ts);
        }
    }
    if min <= max { Some((min, max)) } else { None }
}

/// Normalize a timestamp to `[0, 1]` (0 = oldest, 1 = newest) given the file's
/// `(min, max)` range. Degenerate ranges (single commit) map to the newest end.
pub(in crate::view) fn blame_recency_t(ts: i64, range: (i64, i64)) -> f32 {
    let (min, max) = range;
    if max <= min {
        return 1.0;
    }
    // `min`/`max` are untrusted git author timestamps; a full-i64-range spread
    // (e.g. one line near i64::MIN, another near i64::MAX) would overflow plain
    // subtraction. Saturate — the ratio still lands in [0, 1] after the clamp.
    let span = max.saturating_sub(min);
    let offset = ts.saturating_sub(min);
    (offset as f32 / span as f32).clamp(0.0, 1.0)
}

/// Up to two uppercase initials derived from an author string. Handles
/// `"Jane Doe"`, `"Jane Doe <jane@x>"`, and single-token authors.
pub(in crate::view) fn author_initials(author: &str) -> String {
    let name = author.split('<').next().unwrap_or(author).trim();
    let mut initials = String::new();
    for word in name.split_whitespace() {
        if let Some(c) = word.chars().next() {
            initials.extend(c.to_uppercase());
            if initials.chars().count() >= 2 {
                break;
            }
        }
    }
    if initials.is_empty() {
        initials.push('?');
    }
    initials
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn line(commit: &str, ts: Option<i64>) -> BlameLine {
        BlameLine {
            commit_id: Arc::from(commit),
            author: Arc::from("Jane Doe"),
            author_time_unix: ts,
            summary: Arc::from("summary"),
            body: None,
            line: String::new(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        }
    }

    #[test]
    fn blame_for_new_line_maps_and_flags_run_starts() {
        let lines = vec![
            line("aaa", Some(1)),
            line("aaa", Some(1)),
            line("bbb", Some(2)),
        ];

        // No new-side line (deletion) -> None.
        assert!(blame_for_new_line(&lines, None, None).is_none());
        // Line 0 is invalid (1-based).
        assert!(blame_for_new_line(&lines, Some(0), None).is_none());

        // Rendered contiguously: prev_new_line == n - 1.
        let l1 = blame_for_new_line(&lines, Some(1), None).unwrap();
        assert!(l1.is_run_start);
        let l2 = blame_for_new_line(&lines, Some(2), Some(1)).unwrap();
        assert!(!l2.is_run_start);
        let l3 = blame_for_new_line(&lines, Some(3), Some(2)).unwrap();
        assert!(l3.is_run_start);

        // Out of range.
        assert!(blame_for_new_line(&lines, Some(4), Some(3)).is_none());
    }

    #[test]
    fn blame_for_new_line_run_start_at_hunk_boundary() {
        // Same commit on both lines, but they are not contiguous in render order
        // (the previous rendered blamed line is line 1, not line 2). The first
        // visible line of the new hunk must still start a run.
        let lines = vec![
            line("aaa", Some(1)),
            line("aaa", Some(1)),
            line("aaa", Some(1)),
        ];
        let gapped = blame_for_new_line(&lines, Some(3), Some(1)).unwrap();
        assert!(gapped.is_run_start);
        // Contiguous continuation of the same commit is not a run start.
        let cont = blame_for_new_line(&lines, Some(3), Some(2)).unwrap();
        assert!(!cont.is_run_start);
    }

    #[test]
    fn time_range_and_recency_normalization() {
        let lines = vec![line("a", Some(100)), line("b", Some(300)), line("c", None)];
        let range = blame_time_range(&lines).unwrap();
        assert_eq!(range, (100, 300));
        assert!((blame_recency_t(100, range) - 0.0).abs() < 1e-6);
        assert!((blame_recency_t(300, range) - 1.0).abs() < 1e-6);
        assert!((blame_recency_t(200, range) - 0.5).abs() < 1e-6);
        // Degenerate range -> newest.
        assert_eq!(blame_recency_t(5, (5, 5)), 1.0);
    }

    #[test]
    fn recency_normalization_saturates_on_extreme_range() {
        // Crafted timestamps spanning the full i64 range would overflow a plain
        // `max - min`; saturating arithmetic must keep the result in [0, 1]
        // without panicking.
        for ts in [i64::MIN, 0, i64::MAX] {
            let t = blame_recency_t(ts, (i64::MIN, i64::MAX));
            assert!((0.0..=1.0).contains(&t), "t={t} out of range for ts={ts}");
        }
    }

    #[test]
    fn initials_handle_common_shapes() {
        assert_eq!(author_initials("Jane Doe"), "JD");
        assert_eq!(author_initials("Jane Doe <jane@example.com>"), "JD");
        assert_eq!(author_initials("madonna"), "M");
        assert_eq!(author_initials("  "), "?");
    }
}
