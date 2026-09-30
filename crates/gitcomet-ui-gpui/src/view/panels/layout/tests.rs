use super::*;
use gitcomet_core::domain::{Branch, CommitId, LogPage, RepoSpec};
use gitcomet_state::model::{Loadable, RepoId, RepoState};
use std::path::PathBuf;
use std::sync::Arc;

fn test_repo() -> RepoState {
    RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    )
}

/// Label lengths the unstaged header asks about when three files are picked:
/// `Stage (3)`, `Discard (3)`, `Stage all changes`.
fn unstaged_header_with_selection() -> [usize; 3] {
    [
        "Stage (3)".len(),
        "Discard (3)".len(),
        "Stage all changes".len(),
    ]
}

fn commit_file_filter_test_counts() -> crate::view::rows::CommitFileKindCounts {
    crate::view::rows::CommitFileKindCounts {
        all: 20,
        modified: 12,
        removed: 2,
        added: 5,
        renamed: 1,
    }
}

/// The chips get real side padding under Comfortable, and the width budget
/// that decides between full and count-only labels has to use the same
/// value or the labels overflow the pane they were measured for.
#[test]
fn comfortable_filter_chips_get_padding_the_width_budget_accounts_for() {
    let comfortable = crate::appearance::Appearance {
        density: crate::appearance::UiDensity::Comfortable,
        ..crate::appearance::Appearance::default()
    };
    let compact = crate::appearance::Appearance {
        density: crate::appearance::UiDensity::Compact,
        ..crate::appearance::Appearance::default()
    };

    assert!(commit_file_filter_tab_pad_x(comfortable) > commit_file_filter_tab_pad_x(compact));

    let counts = commit_file_filter_test_counts();
    let labels = |width: f32, metrics| {
        commit_file_filter_labels_for_width(
            px(width),
            counts,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            metrics,
        )
    };
    let mut widths_where_padding_decides = 0;
    for width in 200..600 {
        let width = width as f32;
        if labels(width, compact) != labels(width, comfortable) {
            assert_eq!(labels(width, compact), CommitFileFilterLabels::Full);
            assert_eq!(labels(width, comfortable), CommitFileFilterLabels::Compact);
            widths_where_padding_decides += 1;
        }
    }
    assert!(
        widths_where_padding_decides > 0,
        "the extra padding must reach the budget that picks the labels"
    );
}

#[test]
fn commit_file_filter_tabs_compact_for_larger_ui_fonts() {
    let counts = commit_file_filter_test_counts();
    let default = crate::appearance::Appearance::default();
    let larger_font = crate::appearance::Appearance {
        ui_font_size_px: 20,
        ..default
    };
    for scale in [100, 150, 200] {
        let width = crate::ui_scale::design_px_from_percent(500.0, scale);
        assert_eq!(
            commit_file_filter_labels_for_width(width, counts, scale, default),
            CommitFileFilterLabels::Full,
        );
        assert_eq!(
            commit_file_filter_labels_for_width(width, counts, scale, larger_font),
            CommitFileFilterLabels::Compact,
        );
    }
}

#[test]
fn commit_file_filter_tabs_use_full_labels_when_they_fit() {
    assert_eq!(
        commit_file_filter_labels_for_width(
            px(500.0),
            commit_file_filter_test_counts(),
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        CommitFileFilterLabels::Full
    );
}

#[test]
fn commit_file_filter_tabs_compact_in_narrow_or_scaled_panels() {
    let counts = commit_file_filter_test_counts();
    assert_eq!(
        commit_file_filter_labels_for_width(
            px(300.0),
            counts,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        CommitFileFilterLabels::Compact
    );
    assert_eq!(
        commit_file_filter_labels_for_width(
            px(600.0),
            counts,
            200,
            crate::appearance::Appearance::default(),
        ),
        CommitFileFilterLabels::Compact
    );
}

/// The budget is per-character ink, so a larger UI font has to widen it or
/// the header keeps full labels that no longer fit.
#[test]
fn the_header_budget_follows_the_ui_font() {
    let labels = |width: f32, ui_font_size_px| {
        status_action_labels_for_width(
            px(width),
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance {
                ui_font_size_px,
                ..crate::appearance::Appearance::default()
            },
        )
    };

    let mut widths_where_the_font_decides = 0;
    for width in 200..900 {
        let width = width as f32;
        if labels(width, 14) != labels(width, 24) {
            assert_eq!(labels(width, 14), StatusActionLabels::Full);
            assert_eq!(labels(width, 24), StatusActionLabels::Compact);
            widths_where_the_font_decides += 1;
        }
    }

    assert!(
        widths_where_the_font_decides > 0,
        "a larger UI font must reach the label budget"
    );
}

#[test]
fn status_action_labels_stay_full_in_a_wide_panel() {
    assert_eq!(
        status_action_labels_for_width(
            px(600.0),
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Full
    );
}

/// Guards the calibration in one direction only: a budget that runs long
/// withholds the full wording while there is visibly room for it, which is
/// the failure this pins. The number comes from measuring the shipped font
/// — `Stage (3)`, `Discard (3)` and `Stage all changes` plus their padding,
/// gaps, the `Unstaged` dropdown title and the layout/sort controls need
/// ~510px of real ink and box.
#[test]
fn status_action_labels_expand_as_soon_as_the_row_really_fits() {
    assert_eq!(
        status_action_labels_for_width(
            px(515.0),
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Full,
        "the full labels fit at this width in the real app, so the header must show them"
    );
}

#[test]
fn status_action_labels_shrink_once_the_panel_is_narrow() {
    assert_eq!(
        status_action_labels_for_width(
            px(200.0),
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Compact
    );
}

#[test]
fn status_action_labels_survive_narrower_without_a_selection() {
    // With nothing selected the header carries one button, so the width that
    // forces the three-button header to shrink is still comfortable here.
    let width = px(340.0);
    assert_eq!(
        status_action_labels_for_width(
            width,
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Compact
    );
    assert_eq!(
        status_action_labels_for_width(
            width,
            "Unstaged".len(),
            true,
            &["Stage all changes".len()],
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Full
    );
}

#[test]
fn status_action_labels_account_for_the_in_flight_spinner() {
    // Sized to fit the buttons and title exactly, so the spinner is the only
    // thing that can push it over.
    let mut width = px(0.0);
    for candidate in (200..=600).step_by(2) {
        width = px(candidate as f32);
        if status_action_labels_for_width(
            width,
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ) == StatusActionLabels::Full
        {
            break;
        }
    }
    assert_eq!(
        status_action_labels_for_width(
            width,
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            true,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Compact,
        "the spinner's own width has to count against the budget"
    );
}

#[test]
fn status_action_labels_shrink_earlier_when_zoomed_in() {
    let width = px(580.0);
    assert_eq!(
        status_action_labels_for_width(
            width,
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Full
    );
    assert_eq!(
        status_action_labels_for_width(
            width,
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            200,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Compact
    );
}

#[test]
fn status_action_labels_default_to_full_before_the_panel_is_measured() {
    assert_eq!(
        status_action_labels_for_width(
            px(0.0),
            "Unstaged".len(),
            true,
            &unstaged_header_with_selection(),
            false,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            crate::appearance::Appearance::default(),
        ),
        StatusActionLabels::Full
    );
}

#[test]
fn status_action_labels_rewrite_the_wording() {
    assert_eq!(
        status_action_count_label(StatusActionLabels::Full, "Discard", 12),
        "Discard (12)"
    );
    assert_eq!(
        status_action_count_label(StatusActionLabels::Compact, "Stage", 3),
        "Stg (3)"
    );
    assert_eq!(
        status_action_count_label(StatusActionLabels::Compact, "Discard", 12),
        "Disc (12)"
    );
    assert_eq!(
        status_action_count_label(StatusActionLabels::Compact, "Unstage", 1),
        "Ustg (1)"
    );
    assert_eq!(
        status_action_all_label(StatusActionLabels::Full, "Unstage all changes"),
        "Unstage all changes"
    );
    assert_eq!(
        status_action_all_label(StatusActionLabels::Compact, "Unstage all changes"),
        "All"
    );
}

fn file_status(path: &str, kind: FileStatusKind) -> FileStatus {
    FileStatus {
        path: PathBuf::from(path),
        kind,
        conflict: None,
    }
}

fn repo_with_status(status: RepoStatus) -> RepoState {
    let mut repo = test_repo();
    repo.worktree_status = Loadable::Ready(Arc::clone(&status.unstaged));
    repo.worktree_status_rev = 1;
    repo.staged_status = Loadable::Ready(Arc::clone(&status.staged));
    repo.staged_status_rev = 1;
    repo.status = Loadable::Ready(status.into());
    repo.status_rev = 1;
    repo
}

fn branch(name: &str, target: &str) -> Branch {
    Branch {
        name: name.to_string(),
        target: CommitId(target.into()),
        upstream: None,
        divergence: None,
    }
}

#[test]
fn commit_allowed_when_staged_changes_exist() {
    assert!(commit_allowed(false, 1));
}

#[test]
fn commit_allowed_when_merge_is_active_without_staged_changes() {
    let mut repo = test_repo();
    repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'feature'".to_string()));
    assert!(commit_allowed(merge_active(Some(&repo)), 0));
}

#[test]
fn commit_not_allowed_without_staged_changes_or_merge() {
    assert!(!commit_allowed(false, 0));
}

#[test]
fn amend_allowed_when_filtered_log_is_empty_but_head_branch_exists() {
    let mut repo = test_repo();
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "abc123")]));
    repo.log = Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    }));

    assert!(DetailsPaneView::can_submit_commit(
        Some(&repo),
        "message",
        true
    ));
}

#[test]
fn amend_not_allowed_on_unborn_head_branch() {
    let mut repo = test_repo();
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.log = Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    }));

    assert!(!DetailsPaneView::can_submit_commit(
        Some(&repo),
        "message",
        true
    ));
}

#[test]
fn amend_allowed_on_detached_head_without_visible_log_entry() {
    let mut repo = test_repo();
    repo.head_branch = Loadable::Ready("HEAD".to_string());
    repo.log = Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    }));

    assert!(DetailsPaneView::can_submit_commit(
        Some(&repo),
        "message",
        true
    ));
}

#[test]
fn split_height_clamps_to_minimum_section_heights() {
    let min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
    let total_h = px(400.0);

    let top_clamped = clamp_vertical_split_height(px(-300.0), total_h, min_h, min_h);
    let bottom_clamped = clamp_vertical_split_height(px(900.0), total_h, min_h, min_h);

    assert_eq!(top_clamped, min_h);
    assert_eq!(bottom_clamped, total_h - min_h);
}

#[test]
fn resolved_split_height_defaults_to_half_when_unset() {
    let min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
    let total_h = px(400.0);

    assert_eq!(
        resolved_vertical_split_height(None, total_h, min_h, min_h),
        px(200.0)
    );
}

#[test]
fn split_change_tracking_min_height_includes_inner_handle() {
    assert_eq!(
        min_change_tracking_stack_height(false, px(PANE_RESIZE_HANDLE_PX)),
        px(STATUS_SECTION_MIN_HEIGHT_PX)
    );
    assert_eq!(
        min_change_tracking_stack_height(true, px(PANE_RESIZE_HANDLE_PX)),
        px((STATUS_SECTION_MIN_HEIGHT_PX * 2.0) + PANE_RESIZE_HANDLE_PX)
    );
}

#[test]
fn restored_status_section_heights_clamp_to_visible_minimums() {
    assert_eq!(
        DetailsPaneView::sanitized_restored_change_tracking_height(
            ChangeTrackingView::Combined,
            Some(1),
        ),
        Some(px(STATUS_SECTION_MIN_HEIGHT_PX))
    );
    assert_eq!(
        DetailsPaneView::sanitized_restored_change_tracking_height(
            ChangeTrackingView::SplitUntracked,
            Some(1),
        ),
        Some(px(
            (STATUS_SECTION_MIN_HEIGHT_PX * 2.0) + PANE_RESIZE_HANDLE_PX
        ))
    );
    assert_eq!(
        DetailsPaneView::sanitized_restored_untracked_height(Some(1)),
        Some(px(STATUS_SECTION_MIN_HEIGHT_PX))
    );
}

#[test]
fn status_section_action_selection_falls_back_to_active_combined_unstaged_row() {
    let repo = repo_with_status(RepoStatus {
        unstaged: std::sync::Arc::new(vec![file_status("src/lib.rs", FileStatusKind::Modified)]),
        staged: std::sync::Arc::new(Vec::new()),
    });
    let diff_target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);

    let selection = status_section_action_selection(
        &repo,
        Some(&diff_target),
        None,
        StatusSection::CombinedUnstaged,
    );

    assert_eq!(
        selection,
        StatusSectionActionSelection {
            paths: vec![PathBuf::from("src/lib.rs")],
            from_explicit_selection: false,
        }
    );
    assert_eq!(selection.popover_path(), Some(PathBuf::from("src/lib.rs")));
}

#[test]
fn another_sections_selection_does_not_suppress_active_row_fallback() {
    let repo = repo_with_status(RepoStatus {
        unstaged: std::sync::Arc::new(vec![file_status("a.txt", FileStatusKind::Modified)]),
        staged: std::sync::Arc::new(vec![file_status("b.txt", FileStatusKind::Modified)]),
    });
    let target = DiffTarget::working_tree("a.txt".into(), DiffArea::Unstaged);
    for staged in [vec!["b.txt".into()], Vec::new()] {
        let selected = StatusMultiSelection {
            explicit_section: Some(StatusSection::Staged),
            staged,
            ..Default::default()
        };
        let result = status_section_action_selection(
            &repo,
            Some(&target),
            Some(&selected),
            StatusSection::CombinedUnstaged,
        );
        assert_eq!(result.paths, vec![PathBuf::from("a.txt")]);
        assert!(!result.from_explicit_selection);
    }
}

#[test]
fn status_section_action_selection_limits_active_row_to_matching_split_section() {
    let repo = repo_with_status(RepoStatus {
        unstaged: std::sync::Arc::new(vec![
            file_status("new.txt", FileStatusKind::Untracked),
            file_status("src/lib.rs", FileStatusKind::Modified),
        ]),
        staged: std::sync::Arc::new(Vec::new()),
    });
    let diff_target = DiffTarget::working_tree(PathBuf::from("new.txt"), DiffArea::Unstaged);

    let untracked =
        status_section_action_selection(&repo, Some(&diff_target), None, StatusSection::Untracked);
    let unstaged =
        status_section_action_selection(&repo, Some(&diff_target), None, StatusSection::Unstaged);

    assert_eq!(
        untracked,
        StatusSectionActionSelection {
            paths: vec![PathBuf::from("new.txt")],
            from_explicit_selection: false,
        }
    );
    assert!(unstaged.paths.is_empty());
}

/// The header counts through `status_section_action_count`; it must agree
/// with the selection the actions use, for every section and target.
#[test]
fn status_section_action_count_matches_the_selection() {
    let repo = repo_with_status(RepoStatus {
        unstaged: Arc::new(vec![
            file_status("new.txt", FileStatusKind::Untracked),
            file_status("src/lib.rs", FileStatusKind::Modified),
        ]),
        staged: Arc::new(vec![file_status("staged.rs", FileStatusKind::Added)]),
    });
    let targets = [
        None,
        Some(DiffTarget::working_tree(
            "new.txt".into(),
            DiffArea::Unstaged,
        )),
        Some(DiffTarget::working_tree(
            "src/lib.rs".into(),
            DiffArea::Unstaged,
        )),
        Some(DiffTarget::working_tree(
            "staged.rs".into(),
            DiffArea::Staged,
        )),
        Some(DiffTarget::working_tree(
            "gone.rs".into(),
            DiffArea::Unstaged,
        )),
    ];
    let selections = [
        None,
        Some(StatusMultiSelection {
            explicit_section: Some(StatusSection::Untracked),
            ..Default::default()
        }),
        Some(StatusMultiSelection {
            untracked: vec!["new.txt".into()],
            unstaged: vec!["src/lib.rs".into(), "x.rs".into()],
            ..Default::default()
        }),
        Some(StatusMultiSelection {
            explicit_section: Some(StatusSection::Staged),
            staged: vec!["staged.rs".into()],
            ..Default::default()
        }),
    ];
    for target in &targets {
        for selection in &selections {
            for section in [
                StatusSection::CombinedUnstaged,
                StatusSection::Untracked,
                StatusSection::Unstaged,
                StatusSection::Staged,
            ] {
                assert_eq!(
                    status_section_action_count(
                        &repo,
                        target.as_ref(),
                        selection.as_ref(),
                        section
                    ),
                    status_section_action_selection(
                        &repo,
                        target.as_ref(),
                        selection.as_ref(),
                        section
                    )
                    .paths
                    .len(),
                    "{section:?} {target:?} {selection:?}"
                );
            }
        }
    }
}

#[test]
fn status_explicit_empty_selection_does_not_fall_back_to_the_preview() {
    let repo = repo_with_status(RepoStatus {
        staged: Arc::new(vec![file_status("a.rs", FileStatusKind::Modified)]),
        unstaged: Arc::new(vec![file_status("a.rs", FileStatusKind::Modified)]),
    });
    for section in [StatusSection::CombinedUnstaged, StatusSection::Staged] {
        let selection = StatusMultiSelection {
            explicit_section: Some(section),
            ..Default::default()
        };
        let target = DiffTarget::working_tree("a.rs".into(), section.diff_area());
        let action =
            status_section_action_selection(&repo, Some(&target), Some(&selection), section);
        assert!(action.paths.is_empty());
        assert!(action.from_explicit_selection);
    }
}

#[test]
fn status_section_action_selection_prefers_explicit_selection_over_active_row() {
    let selected_a = PathBuf::from("src/lib.rs");
    let selected_b = PathBuf::from("src/main.rs");
    let repo = repo_with_status(RepoStatus {
        unstaged: std::sync::Arc::new(vec![
            file_status(
                selected_a.to_string_lossy().as_ref(),
                FileStatusKind::Modified,
            ),
            file_status(
                selected_b.to_string_lossy().as_ref(),
                FileStatusKind::Modified,
            ),
        ]),
        staged: std::sync::Arc::new(Vec::new()),
    });
    let diff_target = DiffTarget::working_tree(PathBuf::from("src/other.rs"), DiffArea::Unstaged);
    let selection = StatusMultiSelection {
        unstaged: vec![selected_a.clone(), selected_b.clone()],
        ..Default::default()
    };

    let action_selection = status_section_action_selection(
        &repo,
        Some(&diff_target),
        Some(&selection),
        StatusSection::CombinedUnstaged,
    );

    assert_eq!(
        action_selection,
        StatusSectionActionSelection {
            paths: vec![selected_a, selected_b],
            from_explicit_selection: true,
        }
    );
    assert_eq!(action_selection.popover_path(), None);
}
