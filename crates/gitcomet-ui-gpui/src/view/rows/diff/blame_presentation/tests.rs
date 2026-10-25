use super::*;

fn uncommitted_blame_line() -> gitcomet_core::services::BlameLine {
    gitcomet_core::services::BlameLine {
        commit_id: std::sync::Arc::from("0000000000000000000000000000000000000000"),
        author: std::sync::Arc::from("Not Committed Yet"),
        author_time_unix: None,
        summary: std::sync::Arc::from("Not Committed Yet"),
        body: None,
        line: String::new(),
        prior_exists: false,
        source_path: None,
        prior_commit: None,
    }
}

fn committed_blame_line(sha: &str) -> gitcomet_core::services::BlameLine {
    gitcomet_core::services::BlameLine {
        commit_id: std::sync::Arc::from(sha),
        author: std::sync::Arc::from("Jane Doe"),
        author_time_unix: Some(1_700_000_000),
        summary: std::sync::Arc::from("a commit"),
        body: None,
        line: String::new(),
        prior_exists: true,
        source_path: None,
        prior_commit: None,
    }
}

fn blame_ctx(
    lines: Vec<gitcomet_core::services::BlameLine>,
    area: Option<gitcomet_core::domain::DiffArea>,
) -> BlameRenderCtx {
    BlameRenderCtx {
        labels: std::rc::Rc::new(std::cell::RefCell::new(BlameLabelCache::default())),
        lines: std::sync::Arc::new(lines),
        range: None,
        now: std::time::SystemTime::now(),
        path: std::sync::Arc::from(std::path::Path::new("file.rs")),
        viewed_commit: None,
        area,
    }
}

#[test]
fn classify_local_change_matrix() {
    use gitcomet_core::domain::DiffArea;
    // Staged area: every local line is staged regardless of context-ness.
    assert_eq!(
        classify_local_change(DiffArea::Staged, true),
        LocalChange::Staged
    );
    assert_eq!(
        classify_local_change(DiffArea::Staged, false),
        LocalChange::Staged
    );
    // Unstaged area: an unchanged context line is a staged change; any actual
    // change (add / modify / remove) is an unstaged change.
    assert_eq!(
        classify_local_change(DiffArea::Unstaged, true),
        LocalChange::Staged
    );
    assert_eq!(
        classify_local_change(DiffArea::Unstaged, false),
        LocalChange::Unstaged
    );
}

#[test]
fn unstaged_diff_separates_staged_context_from_unstaged_add() {
    use gitcomet_core::domain::DiffArea;
    let theme = AppTheme::gitcomet_dark();
    let ctx = blame_ctx(
        vec![uncommitted_blame_line(), uncommitted_blame_line()],
        Some(DiffArea::Unstaged),
    );
    // Uncommitted context line -> staged: green diff-add bar.
    let (staged, _) =
        build_row_blame_paint_inner(&ctx, true, Some(1), Some(1), BlamePrev::default(), theme)
            .unwrap();
    assert_eq!(staged.border, theme.colors.diff.added.foreground);
    assert_eq!(staged.when.as_ref(), "Staged");
    // Added line -> unstaged: red diff-remove bar.
    let (unstaged, _) =
        build_row_blame_paint_inner(&ctx, false, None, Some(2), BlamePrev::default(), theme)
            .unwrap();
    assert_eq!(unstaged.border, theme.colors.diff.removed.foreground);
    assert_eq!(unstaged.when.as_ref(), "Unstaged");
}

#[test]
fn modified_line_with_both_sides_is_unstaged_not_staged() {
    use gitcomet_core::domain::DiffArea;
    let theme = AppTheme::gitcomet_dark();
    let ctx = blame_ctx(vec![uncommitted_blame_line()], Some(DiffArea::Unstaged));
    // A split `Modify` row has both old and new line numbers, but it is a
    // change, not unchanged context. Unstaged must override staged here.
    let (paint, _) =
        build_row_blame_paint_inner(&ctx, false, Some(1), Some(1), BlamePrev::default(), theme)
            .unwrap();
    assert_eq!(paint.border, theme.colors.diff.removed.foreground);
    assert_eq!(paint.when.as_ref(), "Unstaged");
}

#[test]
fn staged_area_labels_all_local_as_staged() {
    use gitcomet_core::domain::DiffArea;
    let theme = AppTheme::gitcomet_dark();
    let ctx = blame_ctx(vec![uncommitted_blame_line()], Some(DiffArea::Staged));
    // Even an added line is "Staged" when viewing the staged area.
    let (paint, _) =
        build_row_blame_paint_inner(&ctx, false, None, Some(1), BlamePrev::default(), theme)
            .unwrap();
    assert_eq!(paint.border, theme.colors.diff.added.foreground);
    assert_eq!(paint.when.as_ref(), "Staged");
}

#[test]
fn revision_blame_keeps_now_label() {
    let theme = AppTheme::gitcomet_dark();
    // No working-tree area (revision blame) -> legacy generic local change.
    let ctx = blame_ctx(vec![uncommitted_blame_line()], None);
    let (paint, _) =
        build_row_blame_paint_inner(&ctx, true, Some(1), Some(1), BlamePrev::default(), theme)
            .unwrap();
    assert_eq!(
        paint.border,
        crate::theme::blame_local_change_color(theme.is_dark)
    );
    assert_eq!(paint.when.as_ref(), "Now");
}

#[test]
fn run_breaks_at_staged_unstaged_boundary() {
    use gitcomet_core::domain::DiffArea;
    let theme = AppTheme::gitcomet_dark();
    let ctx = blame_ctx(
        vec![
            uncommitted_blame_line(),
            uncommitted_blame_line(),
            uncommitted_blame_line(),
        ],
        Some(DiffArea::Unstaged),
    );
    let prev = std::cell::Cell::new(BlamePrev::default());
    // Line 1: staged context -> run start, label shown.
    let p1 =
        build_row_blame_paint_tracked(&ctx, true, Some(1), Some(1), &prev, None, theme).unwrap();
    assert!(p1.show_text);
    assert_eq!(p1.when.as_ref(), "Staged");
    // Line 2: staged context, contiguous -> not a run start (label not repeated).
    let p2 =
        build_row_blame_paint_tracked(&ctx, true, Some(2), Some(2), &prev, None, theme).unwrap();
    assert!(!p2.show_text);
    // Line 3: unstaged add. The new-side line is still contiguous, but the
    // staged->unstaged group change must start a new run with its label.
    let p3 = build_row_blame_paint_tracked(&ctx, false, None, Some(3), &prev, None, theme).unwrap();
    assert!(p3.show_text);
    assert_eq!(p3.when.as_ref(), "Unstaged");
}

#[test]
fn wrapped_continuation_rows_do_not_repeat_annotation_text() {
    // Regression: when a long line wraps, each wrapped visual row carries the
    // same logical new-side line, so `is_run_start` is recomputed as true for
    // the continuation rows. Without the wrap_ix gate the time/author/summary
    // label is duplicated down every wrapped line. Continuation rows must keep
    // their recency border but suppress the repeated text.
    let theme = AppTheme::gitcomet_dark();
    let sha = "1111111111111111111111111111111111111111";
    let ctx = blame_ctx(
        vec![committed_blame_line(sha)],
        Some(gitcomet_core::domain::DiffArea::Unstaged),
    );
    let prev = std::cell::Cell::new(BlamePrev::default());
    let wrap = |wrap_ix: usize| {
        Some(diff_canvas::DiffTextWrapSlice {
            wrap_ix,
            wrap_columns: 80,
            primary_range: diff_canvas::DiffWrapByteRange::default(),
            secondary_range: diff_canvas::DiffWrapByteRange::default(),
        })
    };

    // First visual row of the wrapped line (wrap_ix == 0): run start, label shown.
    let first =
        build_row_blame_paint_tracked(&ctx, false, None, Some(1), &prev, wrap(0), theme).unwrap();
    assert!(first.show_text, "the wrap_ix == 0 row shows the annotation");
    let border = first.border;

    // Continuation rows (wrap_ix > 0) of the SAME line: border kept, text hidden.
    for wrap_ix in 1..=2 {
        let cont =
            build_row_blame_paint_tracked(&ctx, false, None, Some(1), &prev, wrap(wrap_ix), theme)
                .unwrap();
        assert!(
            !cont.show_text,
            "wrap_ix == {wrap_ix} continuation row must not repeat the annotation text"
        );
        assert_eq!(cont.border, border, "the recency bar stays continuous");
    }
}

#[test]
fn untracked_content_view_collapses_consecutive_same_commit_lines() {
    // The full file-content view uses the untracked `build_row_blame_paint`
    // (no threaded group). Consecutive lines of the same commit must collapse
    // into one run: the message shows on the first line only, not repeated.
    let theme = AppTheme::gitcomet_dark();
    let sha = "1111111111111111111111111111111111111111";
    let ctx = blame_ctx(
        vec![committed_blame_line(sha), committed_blame_line(sha)],
        Some(gitcomet_core::domain::DiffArea::Unstaged),
    );
    // Line 1 (no previous rendered line) starts the run -> shows the summary.
    let first = build_row_blame_paint(&ctx, false, None, Some(1), None, theme).unwrap();
    assert!(first.show_text);
    assert_eq!(first.summary.as_ref(), "a commit");
    // Line 2 is contiguous and same commit -> not a run start, no repeated text.
    let second = build_row_blame_paint(&ctx, false, None, Some(2), Some(1), theme).unwrap();
    assert!(!second.show_text);
    assert!(second.when.as_ref().is_empty());
    assert!(second.summary.as_ref().is_empty());
}

#[test]
fn removal_rows_get_a_local_bar_only_in_working_tree_blame() {
    use gitcomet_core::domain::DiffArea;
    let theme = AppTheme::gitcomet_dark();
    // Pure removal (old side only) in the unstaged area -> unstaged bar + label,
    // even though there is no `BlameLine` for a deleted line.
    let ctx = blame_ctx(Vec::new(), Some(DiffArea::Unstaged));
    let prev = std::cell::Cell::new(BlamePrev::default());
    let removal =
        build_row_blame_paint_tracked(&ctx, false, Some(5), None, &prev, None, theme).unwrap();
    assert_eq!(removal.border, theme.colors.diff.removed.foreground);
    assert_eq!(removal.when.as_ref(), "Unstaged");
    // Revision blame has no staged/unstaged concept, so removals get no bar.
    let ctx_rev = blame_ctx(Vec::new(), None);
    let prev_rev = std::cell::Cell::new(BlamePrev::default());
    assert!(
        build_row_blame_paint_tracked(&ctx_rev, false, Some(5), None, &prev_rev, None, theme)
            .is_none()
    );
}
