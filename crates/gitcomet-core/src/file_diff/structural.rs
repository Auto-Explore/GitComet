use super::plan::push_plan_run;
use super::{FileDiffPlan, FileDiffPlanRun, eof_newline_delta};
use crate::diff_engine::StructuralDiff;

/// An aligned pair is one row unless both of its lines changed: a line with
/// no changed tokens is not shown as removed or added. A final newline added
/// or removed is not a token, so it reshapes no row; the plan only records it.
pub fn plan(diff: &StructuralDiff, old: &str, new: &str) -> FileDiffPlan {
    let mut runs = Vec::new();
    for &(old_line, new_line) in &diff.aligned_lines {
        let run = match (old_line, new_line) {
            (Some(old_start), Some(new_start))
                if diff.old_changes.contains_key(&old_start)
                    && diff.new_changes.contains_key(&new_start) =>
            {
                FileDiffPlanRun::Modify {
                    old_start,
                    new_start,
                    len: 1,
                }
            }
            (Some(old_start), Some(new_start)) => FileDiffPlanRun::Context {
                old_start,
                new_start,
                len: 1,
            },
            (Some(old_start), None) => FileDiffPlanRun::Remove { old_start, len: 1 },
            (None, Some(new_start)) => FileDiffPlanRun::Add { new_start, len: 1 },
            (None, None) => continue,
        };
        push_plan_run(&mut runs, run);
    }
    let eof_newline = eof_newline_delta(old, new);
    FileDiffPlan {
        row_count: runs.iter().map(FileDiffPlanRun::row_len).sum(),
        inline_row_count: runs.iter().map(FileDiffPlanRun::inline_row_len).sum(),
        runs,
        eof_newline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pair changed on both sides is a row per side; one changed on one
    /// side, or on neither (formatting only), is a single row.
    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn only_a_pair_changed_on_both_sides_takes_two_inline_rows() {
        let diff = StructuralDiff {
            language: "Rust".into(),
            aligned_lines: (0..4).map(|line| (Some(line), Some(line))).collect(),
            old_changes: [(1, vec![0..1]), (3, vec![0..1])].into(),
            new_changes: [(2, vec![0..1]), (3, vec![0..1])].into(),
            blocks: Vec::new(),
        };
        let result = plan(&diff, "f( a );\nx\ny\nz\n", "f(a);\nx2\ny2\nz2\n");
        assert_eq!(
            result.runs,
            [
                FileDiffPlanRun::Context {
                    old_start: 0,
                    new_start: 0,
                    len: 3,
                },
                FileDiffPlanRun::Modify {
                    old_start: 3,
                    new_start: 3,
                    len: 1,
                },
            ]
        );
        assert_eq!(result.row_count, 4);
        assert_eq!(result.inline_row_count, 5);
    }

    #[test]
    fn the_final_newline_delta_survives() {
        let diff = StructuralDiff {
            language: "Rust".into(),
            aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1))],
            old_changes: Default::default(),
            new_changes: Default::default(),
            blocks: Vec::new(),
        };
        let result = plan(&diff, "f( a );\nend", "f(a);\nend\n");
        assert_eq!(
            result.eof_newline,
            Some(super::super::FileDiffEofNewline::MissingInOld)
        );
    }

    /// The line engine's final-newline step turned the last context pair into
    /// a modification; with no changed tokens on either side Inline drew it
    /// twice, both unmarked.
    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn a_final_newline_change_keeps_the_last_pair_one_row() {
        let diff = StructuralDiff {
            language: "Rust".into(),
            aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1))],
            old_changes: [(0, vec![3..4])].into(),
            new_changes: [(0, vec![3..4])].into(),
            blocks: vec![0..1],
        };
        let result = plan(&diff, "fn a() {}\nend", "fn b() {}\nend\n");
        assert_eq!(
            result.runs,
            [
                FileDiffPlanRun::Modify {
                    old_start: 0,
                    new_start: 0,
                    len: 1,
                },
                FileDiffPlanRun::Context {
                    old_start: 1,
                    new_start: 1,
                    len: 1,
                },
            ]
        );
        assert_eq!(result.row_count, 2);
        assert_eq!(result.inline_row_count, 3);
    }
}
