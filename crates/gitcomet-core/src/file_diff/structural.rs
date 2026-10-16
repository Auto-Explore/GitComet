use super::plan::{apply_eof_newline_to_plan, push_plan_run};
use super::{FileDiffPlan, FileDiffPlanRun, eof_newline_delta};
use crate::diff_engine::StructuralDiff;

pub fn plan(diff: &StructuralDiff, old: &str, new: &str) -> FileDiffPlan {
    let old_lines: Vec<_> = old.split_terminator('\n').collect();
    let new_lines: Vec<_> = new.split_terminator('\n').collect();
    let mut runs = Vec::new();
    for &(old_line, new_line) in &diff.aligned_lines {
        let run = match (old_line, new_line) {
            (Some(old_start), Some(new_start))
                if old_lines[old_start] == new_lines[new_start]
                    && !diff.old_changes.contains_key(&old_start)
                    && !diff.new_changes.contains_key(&new_start) =>
            {
                FileDiffPlanRun::Context {
                    old_start,
                    new_start,
                    len: 1,
                }
            }
            (Some(old_start), Some(new_start)) => FileDiffPlanRun::Modify {
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
    apply_eof_newline_to_plan(&mut runs, eof_newline);
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

    #[test]
    fn unequal_neutral_lines_keep_both_sources_and_eof_delta() {
        let diff = StructuralDiff {
            language: "Rust".into(),
            aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1))],
            old_changes: Default::default(),
            new_changes: Default::default(),
            blocks: Vec::new(),
        };
        let result = plan(&diff, "f( a );\nend", "f(a);\nend\n");
        assert_eq!(result.row_count, 2);
        assert_eq!(result.inline_row_count, 4);
        assert_eq!(
            result.eof_newline,
            Some(super::super::FileDiffEofNewline::MissingInOld)
        );
        assert!(
            result
                .runs
                .iter()
                .all(|run| matches!(run, FileDiffPlanRun::Modify { .. }))
        );
    }
}
