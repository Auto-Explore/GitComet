#![allow(dead_code)]
#![allow(clippy::type_complexity)]

use super::*;
use std::path::PathBuf;

fn fixture_repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("test fixtures should run from the workspace root")
        .to_path_buf()
}

fn push_inline_submodule_diff_content_mode_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
) -> gitcomet_core::domain::DiffTarget {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_inline_root",
        std::process::id(),
        fixture_name
    ));
    let submodule_workdir = workdir.join("vendor/submodule");
    let _ = std::fs::create_dir_all(&submodule_workdir);
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::CommitRange {
        from_commit_id: gitcomet_core::domain::CommitId("aaaa".into()),
        to_commit_id: gitcomet_core::domain::CommitId("bbbb".into()),
        path: Some(path.clone()),
    };
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-old value
+new value
 unchanged
";
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), unified);
    let file_diff = gitcomet_core::domain::FileDiffText::new(
        path.clone(),
        Some("old value\nunchanged\n".to_string()),
        Some("new value\nunchanged\n".to_string()),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: PathBuf::from("vendor/submodule"),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.inline_submodule_diff =
                Some(gitcomet_state::model::InlineSubmoduleDiffState {
                    submodule_repo_path: submodule_workdir.clone(),
                    parent_submodule_path: PathBuf::from("vendor/submodule"),
                    entries: vec![gitcomet_state::model::InlineSubmoduleDiffEntry {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        target: target.clone(),
                        section: gitcomet_state::model::InlineSubmoduleDiffSection::Range(
                            gitcomet_core::domain::SubmoduleDiffRangeKind::CommitHistory,
                        ),
                    }],
                    selected_ix: 0,
                    target: target.clone(),
                    rev: 1,
                    diff_rev: 1,
                    diff: gitcomet_state::model::Loadable::Ready(Arc::new(diff)),
                    diff_file_rev: 1,
                    diff_file: gitcomet_state::model::Loadable::Ready(Some(Arc::new(file_diff))),
                    diff_file_image: gitcomet_state::model::Loadable::NotLoaded,
                });

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    target
}

fn push_regular_diff_content_mode_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    path: PathBuf,
    unified: String,
    old_text: String,
    new_text: String,
) -> gitcomet_core::domain::DiffTarget {
    push_regular_diff_content_mode_state_with_rev(
        cx,
        view,
        repo_id,
        fixture_name,
        path,
        1,
        unified,
        old_text,
        new_text,
    )
}

fn push_regular_diff_content_mode_state_with_rev(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    path: PathBuf,
    diff_rev: u64,
    unified: String,
    old_text: String,
    new_text: String,
) -> gitcomet_core::domain::DiffTarget {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_regular_root",
        std::process::id(),
        fixture_name
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
        path: Some(path.clone()),
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);
    let file_diff =
        gitcomet_core::domain::FileDiffText::new(path.clone(), Some(old_text), Some(new_text));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = diff_rev;
            repo.diff_state.diff_rev = diff_rev;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file_rev = diff_rev;
            repo.diff_state.diff_file =
                gitcomet_state::model::Loadable::Ready(Some(Arc::new(file_diff)));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    target
}

fn build_collapsed_diff_fixture_texts() -> (String, String, String) {
    let old_lines = (1..=70usize)
        .map(|line| {
            if line == 35 {
                "old value 35".to_string()
            } else {
                format!("line {line}")
            }
        })
        .collect::<Vec<_>>();
    let new_lines = (1..=70usize)
        .map(|line| {
            if line == 35 {
                "new value 35".to_string()
            } else {
                format!("line {line}")
            }
        })
        .collect::<Vec<_>>();

    let old_text = format!("{}\n", old_lines.join("\n"));
    let new_text = format!("{}\n", new_lines.join("\n"));
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -32,7 +32,7 @@
 {}
 {}
 {}
-{}
+{}
 {}
 {}
 {}
",
        old_lines[31],
        old_lines[32],
        old_lines[33],
        old_lines[34],
        new_lines[34],
        old_lines[35],
        old_lines[36],
        old_lines[37],
    );
    (unified, old_text, new_text)
}

fn build_collapsed_diff_horizontal_scroll_fixture_texts() -> (String, String, String) {
    let old_lines = (1..=70usize)
        .map(|line| {
            if line == 35 {
                format!("old value 35 {}", "left_payload_".repeat(160))
            } else {
                format!("line {line}")
            }
        })
        .collect::<Vec<_>>();
    let new_lines = (1..=70usize)
        .map(|line| {
            if line == 35 {
                format!("new value 35 {}", "right_payload_".repeat(160))
            } else {
                format!("line {line}")
            }
        })
        .collect::<Vec<_>>();

    let old_text = format!("{}\n", old_lines.join("\n"));
    let new_text = format!("{}\n", new_lines.join("\n"));
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -32,7 +32,7 @@
 {}
 {}
 {}
-{}
+{}
 {}
 {}
 {}
",
        old_lines[31],
        old_lines[32],
        old_lines[33],
        old_lines[34],
        new_lines[34],
        old_lines[35],
        old_lines[36],
        old_lines[37],
    );
    (unified, old_text, new_text)
}

fn build_collapsed_diff_multi_hunk_fixture_texts(
    changes: &[(usize, &'static str, &'static str)],
) -> (String, String, String) {
    let total_lines = 100usize;
    let mut old_lines = (1..=total_lines)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
    let mut new_lines = old_lines.clone();
    let mut sorted_changes = changes.to_vec();
    sorted_changes.sort_by_key(|(line, _, _)| *line);
    for (line, old_text, new_text) in sorted_changes.iter().copied() {
        old_lines[line - 1] = old_text.to_string();
        new_lines[line - 1] = new_text.to_string();
    }

    let old_text = format!("{}\n", old_lines.join("\n"));
    let new_text = format!("{}\n", new_lines.join("\n"));
    let mut unified = String::from(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
",
    );
    for (line, _, _) in sorted_changes {
        let context_start = line.saturating_sub(3).max(1);
        let context_end = (line + 3).min(total_lines);
        let context_count = context_end.saturating_sub(context_start).saturating_add(1);
        unified.push_str(&format!(
            "@@ -{context_start},{context_count} +{context_start},{context_count} @@\n"
        ));
        for current_line in context_start..=context_end {
            if current_line == line {
                unified.push_str(&format!("-{}\n", old_lines[current_line - 1]));
                unified.push_str(&format!("+{}\n", new_lines[current_line - 1]));
            } else {
                unified.push_str(&format!(" {}\n", old_lines[current_line - 1]));
            }
        }
    }

    (unified, old_text, new_text)
}

fn build_collapsed_diff_long_gap_fixture_texts() -> (String, String, String) {
    build_collapsed_diff_multi_hunk_fixture_texts(&[
        (20, "old value 20", "new value 20"),
        (60, "old value 60", "new value 60"),
    ])
}

fn build_collapsed_diff_short_gap_fixture_texts() -> (String, String, String) {
    build_collapsed_diff_multi_hunk_fixture_texts(&[
        (20, "old value 20", "new value 20"),
        (34, "old value 34", "new value 34"),
    ])
}

fn activate_collapsed_diff_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
    unified: String,
    old_text: String,
    new_text: String,
) -> gitcomet_core::domain::DiffTarget {
    let path = PathBuf::from("src/lib.rs");
    let target = push_regular_diff_content_mode_state(
        cx,
        view,
        repo_id,
        fixture_name,
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff fixture activates full file diff first",
        |pane| {
            pane.is_file_diff_view_active() && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "mode={:?} file_diff_active={} target={:?}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    set_diff_content_mode_for_test(cx, view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff projection becomes active",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && !pane.collapsed_diff_hunk_visible_indices.is_empty()
        },
        |pane| {
            format!(
                "collapsed_active={} diff_view={:?} visible_len={} hunk_rows={:?}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_view,
                pane.diff_visible_len(),
                pane.collapsed_diff_hunk_visible_indices,
            )
        },
    );

    target
}

fn debug_selector_center(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> gpui::Point<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected `{selector}` bounds"))
        .center()
}

fn collapsed_hunk_visible_ix_for_src_ix(
    pane: &crate::view::panes::main::MainPaneView,
    src_ix: usize,
) -> usize {
    pane.patch_hunk_entries()
        .into_iter()
        .find_map(|(visible_ix, candidate_src_ix)| {
            (candidate_src_ix == src_ix).then_some(visible_ix)
        })
        .unwrap_or_else(|| panic!("expected a collapsed hunk anchor for src_ix={src_ix}"))
}

fn collapsed_file_row_visible_ix(
    pane: &crate::view::panes::main::MainPaneView,
    target_row_ix: usize,
) -> usize {
    (0..pane.diff_visible_len())
        .find(|&visible_ix| {
            matches!(
                pane.collapsed_visible_row(visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix })
                    if row_ix == target_row_ix
            )
        })
        .unwrap_or_else(|| panic!("expected a collapsed file row for row_ix={target_row_ix}"))
}

fn collapsed_diff_cache_rebuild_snapshot(pane: &crate::view::panes::main::MainPaneView) -> String {
    let active = pane.active_repo();
    format!(
        "mode={:?} rev={} active_rev={:?} target={:?} active_target={:?} signature={:?} file_rev={} active_file_rev={:?} file_target={:?} file_path={:?} collapsed_active={} hunks={:?}",
        pane.diff_content_mode,
        pane.diff_cache_rev,
        active.map(|repo| repo.diff_state.diff_rev),
        pane.diff_cache_target,
        active.and_then(|repo| repo.diff_state.diff_target.clone()),
        pane.diff_cache_content_signature,
        pane.file_diff_cache_rev,
        active.map(|repo| repo.diff_state.diff_file_rev),
        pane.file_diff_cache_target,
        pane.file_diff_cache_path,
        pane.is_collapsed_diff_projection_active(),
        pane.collapsed_diff_hunks,
    )
}

fn diff_text_hitbox_top_for_visible_ix(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    visible_ix: usize,
    region: DiffTextRegion,
) -> f32 {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_text_hitboxes
            .get(&(visible_ix, region))
            .unwrap_or_else(|| {
                panic!("expected diff text hitbox for visible_ix={visible_ix} region={region:?}")
            })
            .bounds
            .top()
            .into()
    })
}

fn diff_scroll_offset_y(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> f32 {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_scroll.0.borrow().base_handle.offset().y.into()
    })
}

fn diff_split_right_scroll_offset_y(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> f32 {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y
            .into()
    })
}

fn scroll_collapsed_visible_ix_to_center(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    visible_ix: usize,
) {
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
        });
    });
    draw_and_drain_test_window(cx);
}

fn reveal_collapsed_diff_hunk_side_fully(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    src_ix: usize,
    reveal_up: bool,
) {
    loop {
        let hidden = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            if reveal_up {
                pane.collapsed_diff_hidden_up_rows(src_ix)
            } else {
                pane.collapsed_diff_hidden_down_rows(src_ix)
            }
        });
        if hidden == 0 {
            break;
        }

        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            main_pane.update(app, |pane, cx| {
                if reveal_up {
                    pane.collapsed_diff_reveal_hunk_up(src_ix, cx);
                } else {
                    pane.collapsed_diff_reveal_hunk_down(src_ix, cx);
                }
            });
        });
        draw_and_drain_test_window(cx);
    }
}

fn assert_collapsed_hunk_header_hides_after_full_reveal(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        unified,
        old_text,
        new_text,
    );

    let hunk_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .first()
            .map(|hunk| hunk.src_ix)
            .expect("expected collapsed diff fixture to expose one hunk")
    });

    reveal_collapsed_diff_hunk_side_fully(cx, view, hunk_src_ix, true);

    let hidden_down_after_up = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            0,
            "fully revealing the upper side should consume the hidden-up budget"
        );
        let anchor_visible_ix = pane
            .collapsed_diff_hunk_visible_indices
            .first()
            .copied()
            .expect("expected collapsed diff hunk anchor after revealing upward");
        assert!(
            matches!(
                pane.collapsed_visible_row(anchor_visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { .. })
            ),
            "once the upper gap is fully consumed, the hunk anchor should move to the first visible file row"
        );
        assert_eq!(
            pane.patch_hunk_entries(),
            vec![(anchor_visible_ix, hunk_src_ix)],
            "patch hunk entries should keep pointing at the merged hunk anchor after the top expansion row disappears"
        );
        assert_eq!(
            pane.diff_nav_entries(),
            vec![anchor_visible_ix],
            "diff navigation should continue using the merged hunk anchor once the top expansion row disappears"
        );
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_up > 0,
        "fixture should still keep hidden rows below the hunk after revealing only upward context"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            matches!(
                pane.collapsed_visible_row(pane.diff_visible_len().saturating_sub(1)),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                    display_src_ix: None,
                    ..
                })
            ),
            "expected the trailing down-expansion row to remain in the collapsed projection while hidden rows still exist below the merged hunk"
        );
    });

    reveal_collapsed_diff_hunk_side_fully(cx, view, hunk_src_ix, false);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.collapsed_diff_hidden_up_rows(hunk_src_ix), 0);
        assert_eq!(pane.collapsed_diff_hidden_down_rows(hunk_src_ix), 0);

        let anchor_visible_ix = pane
            .collapsed_diff_hunk_visible_indices
            .first()
            .copied()
            .expect("expected collapsed diff hunk anchor");
        assert!(
            matches!(
                pane.collapsed_visible_row(anchor_visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { .. })
            ),
            "fully revealed collapsed hunks should anchor navigation to the first file row instead of a synthetic header"
        );
        assert_eq!(
            pane.patch_hunk_entries(),
            vec![(anchor_visible_ix, hunk_src_ix)],
            "patch hunk entries should keep pointing at the same source hunk when the synthetic header disappears"
        );
        assert_eq!(
            pane.diff_nav_entries(),
            vec![anchor_visible_ix],
            "diff navigation should continue using the fully revealed hunk anchor"
        );
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !matches!(
                pane.collapsed_visible_row(pane.diff_visible_len().saturating_sub(1)),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                    display_src_ix: None,
                    ..
                })
            ),
            "expected the trailing down-expansion row to disappear once the remaining hidden rows are fully revealed"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_survives_projection_reset(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(188);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_reset",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });
    assert!(
        hidden_down_before > 0,
        "fixture should start with hidden rows below the hunk"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let hidden_down_after_reveal = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.reset_collapsed_diff_projection(false);
            pane.ensure_diff_visible_indices();
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_after_reveal,
            "non-clearing projection resets should preserve revealed collapsed diff context"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.reset_collapsed_diff_projection(true);
            pane.ensure_diff_visible_indices();
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before,
            "clearing projection resets should restore the collapsed default"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_survives_window_resize(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(189);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_resize",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let (hidden_down_after_reveal, visible_len_after_reveal) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            pane.diff_visible_len(),
        )
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    cx.simulate_resize(gpui::size(px(900.0), px(620.0)));
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_after_reveal,
            "window resize should not reset revealed collapsed diff context"
        );
        assert_eq!(
            pane.diff_visible_len(),
            visible_len_after_reveal,
            "window resize should preserve the collapsed projection row count"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_survives_same_content_diff_cache_rebuild(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(286);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_same_content_rebuild",
        DiffViewMode::Inline,
        unified.clone(),
        old_text.clone(),
        new_text.clone(),
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let hidden_down_after_reveal = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_same_content_rebuild",
        PathBuf::from("src/lib.rs"),
        2,
        unified,
        old_text,
        new_text,
    );
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content patch diff cache rebuild completes",
        |pane| {
            pane.diff_cache_rev == 2
                && pane.diff_cache_content_signature.is_some()
                && pane.is_collapsed_diff_projection_active()
                && !pane.collapsed_diff_hunks.is_empty()
        },
        collapsed_diff_cache_rebuild_snapshot,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_after_reveal,
            "same-content patch diff cache rebuilds should preserve revealed collapsed diff context"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_resets_when_diff_content_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(287);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_resets_on_content_change",
        DiffViewMode::Inline,
        unified.clone(),
        old_text.clone(),
        new_text.clone(),
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let hidden_down_after_reveal = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    let changed_unified = unified.replace("new value 35", "new value 35 updated");
    let changed_new_text = new_text.replace("new value 35", "new value 35 updated");
    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_resets_on_content_change",
        PathBuf::from("src/lib.rs"),
        2,
        changed_unified,
        old_text,
        changed_new_text,
    );
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "changed-content patch diff cache rebuild completes",
        |pane| {
            pane.diff_cache_rev == 2
                && pane.diff_cache_content_signature.is_some()
                && pane.is_collapsed_diff_projection_active()
                && !pane.collapsed_diff_hunks.is_empty()
        },
        collapsed_diff_cache_rebuild_snapshot,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before,
            "changed patch diff content should reset revealed collapsed diff context"
        );
    });
}

#[gpui::test]
fn collapsed_diff_split_header_shows_stats_without_file_header(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(288);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_header_stats",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunk_visible_indices.first().copied(),
            Some(0),
            "collapsed mode should start at the hunk expansion row, not a file-path header"
        );
        assert_eq!(
            pane.collapsed_diff_total_file_stat(),
            Some((1, 1)),
            "fixture should expose one added and one removed row for the split header counters"
        );
    });
    assert!(
        cx.debug_bounds("diff_split_header_removed_stat").is_some(),
        "expected the removed counter to be rendered in the A (local / before) header"
    );
    assert!(
        cx.debug_bounds("diff_split_header_added_stat").is_some(),
        "expected the added counter to be rendered in the B (remote / after) header"
    );
}

#[gpui::test]
fn collapsed_diff_revealed_hunk_header_hides_context_and_updates_ranges(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(289);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let unified = unified.replace("@@ -32,7 +32,7 @@", "@@ -32,7 +32,7 @@ impl MainPaneView {");
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_header_dynamic_range",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, hunk_visible_ix, header_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, hunk.src_ix);
        let header = pane
            .diff_text_line_for_region(visible_ix, DiffTextRegion::Inline)
            .to_string();
        (hunk.src_ix, visible_ix, header)
    });
    assert_eq!(header_before, "-32,7 +32,7  impl MainPaneView {");

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_up(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_line_for_region(hunk_visible_ix, DiffTextRegion::Inline)
                .as_ref(),
            "-12,27 +12,27",
            "revealing context above should hide the static context label and expand the displayed old/new ranges"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_line_for_region(hunk_visible_ix, DiffTextRegion::Inline)
                .as_ref(),
            "-12,47 +12,47",
            "revealing context below should also expand the displayed old/new ranges"
        );
    });
}

#[gpui::test]
fn diff_content_mode_main_pane_persist_path_does_not_reenter_main_pane_updates(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            main_pane.update(app, |pane, cx| {
                pane.set_diff_content_mode_and_persist(DiffContentMode::Collapsed, cx);
            });
        });
    }));
    assert!(
        result.is_ok(),
        "main-pane diff content mode persistence should not re-enter MainPaneView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::diff_content_mode(view.read(app)),
            DiffContentMode::Collapsed,
        );
        assert_eq!(
            view.read(app).main_pane.read(app).diff_content_mode,
            DiffContentMode::Collapsed,
        );
    });
}

#[gpui::test]
fn diff_content_mode_switches_regular_file_diff_between_patch_and_content(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(186);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_content_mode_regular",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
        path: Some(path.clone()),
    };
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-old value
+new value
 unchanged
";
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), unified);
    let file_diff = gitcomet_core::domain::FileDiffText::new(
        path.clone(),
        Some("old value\nunchanged\n".to_string()),
        Some("new value\nunchanged\n".to_string()),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file =
                gitcomet_state::model::Loadable::Ready(Some(Arc::new(file_diff)));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "regular file diff content mode activates file diff view",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "content_mode={:?} file_diff_active={} inflight={:?} patch_rows={} file_rows={}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        &view,
        "regular file diff changed-lines mode activates patch view",
        |pane| !pane.is_file_diff_view_active() && pane.patch_diff_split_row_len() > 0,
        |pane| {
            format!(
                "content_mode={:?} file_diff_active={} patch_rows={} file_rows={}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);

    wait_for_main_pane_condition(
        cx,
        &view,
        "regular file diff switches back to file diff view",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "content_mode={:?} file_diff_active={} inflight={:?} patch_rows={} file_rows={}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );
}

#[gpui::test]
fn diff_content_mode_switches_inline_submodule_diff_between_patch_and_content(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(187);
    let target = push_inline_submodule_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "diff_content_mode_switches",
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline submodule content mode activates file diff view",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} is_file_preview={} supports_toggle={} wants_file_view={} file_diff_active={} inflight={:?} cache_repo_id={:?} cache_rev={} cache_target={:?} cache_path={:?} rendered_identity={:?} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_preview_active(),
                pane.supports_diff_content_mode_toggle(pane.is_file_preview_active()),
                pane.wants_file_diff_view(pane.is_file_preview_active()),
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target,
                pane.file_diff_cache_path,
                pane.rendered_file_diff_identity(),
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline submodule changed-lines mode activates patch view",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && !pane.is_file_diff_view_active()
                && pane.patch_diff_split_row_len() > 0
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} file_diff_active={} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline submodule switches back to file diff view",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} file_diff_active={} inflight={:?} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );
}

#[gpui::test]
fn diff_content_mode_inline_submodule_persist_path_does_not_panic(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(188);
    let target = push_inline_submodule_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "diff_content_mode_click",
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline submodule content mode activates file diff view before pane-owned toggle",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} file_diff_active={} inflight={:?} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    let changed_lines_click = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            main_pane.update(app, |pane, cx| {
                pane.set_diff_content_mode_and_persist(DiffContentMode::Collapsed, cx);
            });
        });
    }));
    assert!(
        changed_lines_click.is_ok(),
        "switching to Changed lines from the inline submodule pane should not panic"
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline submodule changed-lines toolbar click activates patch view",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && pane.diff_content_mode == DiffContentMode::Collapsed
                && !pane.is_file_diff_view_active()
                && pane.patch_diff_split_row_len() > 0
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} file_diff_active={} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    let content_click = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            main_pane.update(app, |pane, cx| {
                pane.set_diff_content_mode_and_persist(DiffContentMode::Full, cx);
            });
        });
    }));
    assert!(
        content_click.is_ok(),
        "switching back to Content from the inline submodule pane should not panic"
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline submodule content pane-owned toggle restores file diff view",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && pane.diff_content_mode == DiffContentMode::Full
                && pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} file_diff_active={} inflight={:?} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
            )
        },
    );
}

#[gpui::test]
fn collapsed_diff_hunk_header_click_does_not_create_row_selection(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(189);
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_header_click",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed diff fixture activates full file diff first",
        |pane| {
            pane.is_file_diff_view_active() && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "mode={:?} file_diff_active={} target={:?}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
            )
        },
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed diff projection becomes active",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && !pane.collapsed_diff_hunk_visible_indices.is_empty()
        },
        |pane| {
            format!(
                "collapsed_active={} visible_len={} hunk_rows={:?}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_visible_len(),
                pane.collapsed_diff_hunk_visible_indices,
            )
        },
    );

    let hunk_visible_ix = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .collapsed_diff_hunk_visible_indices[0]
    });

    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        hunk_visible_ix,
        DiffTextRegion::SplitLeft,
        0..1,
        "collapsed hunk header click target",
    );
    simulate_counted_click(cx, click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor, None,
            "collapsed hunk header click should not create a row selection anchor"
        );
        assert_eq!(
            pane.diff_selection_range, None,
            "collapsed hunk header click should not create a row selection range"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_controls_expand_visible_context(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(190);
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_reveal",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed reveal fixture activates full file diff first",
        |pane| {
            pane.is_file_diff_view_active() && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "mode={:?} file_diff_active={} target={:?}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
            )
        },
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed reveal projection becomes active",
        |pane| pane.is_collapsed_diff_projection_active() && !pane.collapsed_diff_hunks.is_empty(),
        |pane| {
            format!(
                "collapsed_active={} visible_len={} hunks={:?}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_visible_len(),
                pane.collapsed_diff_hunks,
            )
        },
    );

    let (hunk_src_ix, visible_before, hidden_up_before, hidden_down_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let hunk = pane
                .collapsed_diff_hunks
                .first()
                .copied()
                .expect("expected collapsed diff fixture to expose one hunk");
            (
                hunk.src_ix,
                pane.diff_visible_len(),
                pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
                pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
            )
        });

    assert!(
        hidden_up_before >= 20 && hidden_down_before >= 20,
        "fixture should expose enough hidden context for 20-line reveal steps"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_up(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 20,
            "revealing above the hunk should add 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            hidden_up_before - 20,
            "revealing above the hunk should reduce the hidden-up budget by 20 rows"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 40,
            "revealing below the hunk should add another 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before - 20,
            "revealing below the hunk should reduce the hidden-down budget by 20 rows"
        );
    });
}

#[gpui::test]
fn collapsed_diff_inline_hunk_header_hides_after_full_reveal(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_hunk_header_hides_after_full_reveal(
        cx,
        &view,
        gitcomet_state::model::RepoId(195),
        "collapsed_inline_full_reveal",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_hunk_header_hides_after_full_reveal(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_hunk_header_hides_after_full_reveal(
        cx,
        &view,
        gitcomet_state::model::RepoId(196),
        "collapsed_split_full_reveal",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn collapsed_diff_long_gap_exposes_up_both_and_trailing_down_expansions(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(197);
    let (unified, old_text, new_text) = build_collapsed_diff_long_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_long_gap",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            2,
            "expected the long-gap fixture to expose two collapsed sections"
        );
        let first_anchor = pane.collapsed_diff_hunk_visible_indices[0];
        let second_anchor = pane.collapsed_diff_hunk_visible_indices[1];
        assert!(
            matches!(
                pane.collapsed_visible_row(first_anchor),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Up,
                    ..
                })
            ),
            "the first collapsed section should expose only an upward expansion row"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(second_anchor),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Both,
                    ..
                })
            ),
            "the second collapsed section should expose a both-direction expansion row for the long interior gap"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(pane.diff_visible_len().saturating_sub(1)),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                    display_src_ix: None,
                    ..
                })
            ),
            "a trailing dummy expansion row should remain at the bottom when there is hidden context below the last section"
        );
    });
}

#[gpui::test]
fn collapsed_diff_short_gap_uses_single_expand_all_and_merges_sections(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(198);
    let (unified, old_text, new_text) = build_collapsed_diff_short_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_short_gap",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let second_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            2,
            "expected the short-gap fixture to expose two collapsed sections before merging"
        );
        let second_anchor = pane.collapsed_diff_hunk_visible_indices[1];
        assert!(
            matches!(
                pane.collapsed_visible_row(second_anchor),
                Some(
                    crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                        expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Short,
                        ..
                    }
                )
            ),
            "the second collapsed section should expose a single short-gap expansion row"
        );
        pane.collapsed_diff_hunks[1].src_ix
    });

    assert!(
        cx.debug_bounds("collapsed_diff_inline_hunk_short")
            .is_some(),
        "expected the short-gap control to be rendered before expanding it"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_short(second_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            1,
            "expanding a short gap should merge the neighboring collapsed sections"
        );
        assert_eq!(
            pane.collapsed_diff_hunk_visible_indices.len(),
            1,
            "merged short gaps should leave a single collapsed-section anchor"
        );
        assert_eq!(
            pane.patch_hunk_entries().len(),
            1,
            "merged short gaps should behave as one change section for diff navigation"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.reset_collapsed_diff_projection(false);
            pane.ensure_diff_visible_indices();
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            1,
            "projection rebuilds should keep a fully revealed short gap merged"
        );
        assert_eq!(
            pane.patch_hunk_entries().len(),
            1,
            "projection rebuilds should not split merged short-gap navigation"
        );
    });
    assert!(
        cx.debug_bounds("collapsed_diff_inline_hunk_short")
            .is_none(),
        "expected the short-gap control to disappear after the sections merge"
    );
}

#[gpui::test]
fn collapsed_diff_inline_hunk_header_stays_pinned_during_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(191);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_inline_hscroll",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed inline diff horizontal overflow becomes available",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    let shell_before = cx
        .debug_bounds("collapsed_diff_inline_hunk_shell")
        .expect("expected collapsed inline hunk shell bounds before scroll");
    let (file_visible_ix, row_before_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk_visible_ix = pane.collapsed_diff_hunk_visible_indices[0];
        let file_visible_ix = hunk_visible_ix + 1;
        assert!(
            matches!(
                pane.collapsed_visible_row(file_visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { .. })
            ),
            "expected the row after the collapsed hunk header to be file content"
        );
        let row_x: f32 = pane
            .diff_text_hitboxes
            .get(&(file_visible_ix, DiffTextRegion::Inline))
            .expect("expected inline file-row hitbox before scroll")
            .bounds
            .left()
            .into();
        (file_visible_ix, row_x)
    });
    let shell_before_x: f32 = shell_before.left().into();

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let max_offset = handle.max_offset();
            handle.set_offset(point(-max_offset.x.min(px(600.0)), px(0.0)));
        });
    });
    draw_and_drain_test_window(cx);

    let shell_after = cx
        .debug_bounds("collapsed_diff_inline_hunk_shell")
        .expect("expected collapsed inline hunk shell bounds after scroll");
    let (row_after_x, offset_after_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let row_x: f32 = pane
            .diff_text_hitboxes
            .get(&(file_visible_ix, DiffTextRegion::Inline))
            .expect("expected inline file-row hitbox after scroll")
            .bounds
            .left()
            .into();
        let offset_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        (row_x, offset_x)
    });
    let shell_after_x: f32 = shell_after.left().into();

    assert!(
        offset_after_x < 0.0,
        "expected inline collapsed diff to scroll horizontally, got offset={offset_after_x}"
    );
    assert!(
        (shell_after_x - shell_before_x).abs() < 0.01,
        "collapsed inline hunk shell should stay pinned (before={shell_before_x}, after={shell_after_x})"
    );
    assert!(
        (row_after_x - row_before_x).abs() > 1.0,
        "collapsed inline file rows should still scroll horizontally (before={row_before_x}, after={row_after_x})"
    );
}

#[gpui::test]
fn collapsed_diff_split_hunk_headers_stay_pinned_during_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(192);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_hscroll",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split diff horizontal overflow becomes available",
        |pane| {
            pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                && pane
                    .diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x
                    > px(0.0)
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    let left_shell_before = cx
        .debug_bounds("collapsed_diff_split_left_hunk_shell")
        .expect("expected collapsed split left hunk shell bounds before scroll");
    let right_shell_before = cx
        .debug_bounds("collapsed_diff_split_right_hunk_shell")
        .expect("expected collapsed split right hunk shell bounds before scroll");
    let (file_visible_ix, left_row_before_x, right_row_before_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk_visible_ix = pane.collapsed_diff_hunk_visible_indices[0];
        let file_visible_ix = hunk_visible_ix + 1;
        assert!(
            matches!(
                pane.collapsed_visible_row(file_visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { .. })
            ),
            "expected the row after the collapsed hunk header to be file content"
        );
        let left_row_x: f32 = pane
            .diff_text_hitboxes
            .get(&(file_visible_ix, DiffTextRegion::SplitLeft))
            .expect("expected split-left file-row hitbox before scroll")
            .bounds
            .left()
            .into();
        let right_row_x: f32 = pane
            .diff_text_hitboxes
            .get(&(file_visible_ix, DiffTextRegion::SplitRight))
            .expect("expected split-right file-row hitbox before scroll")
            .bounds
            .left()
            .into();
        (file_visible_ix, left_row_x, right_row_x)
    });
    let left_shell_before_x: f32 = left_shell_before.left().into();
    let right_shell_before_x: f32 = right_shell_before.left().into();

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
            let left_max = left_handle.max_offset();
            let right_max = right_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(540.0)), px(0.0)));
            right_handle.set_offset(point(-right_max.x.min(px(1080.0)), px(0.0)));
        });
    });
    draw_and_drain_test_window(cx);

    let left_shell_after = cx
        .debug_bounds("collapsed_diff_split_left_hunk_shell")
        .expect("expected collapsed split left hunk shell bounds after scroll");
    let right_shell_after = cx
        .debug_bounds("collapsed_diff_split_right_hunk_shell")
        .expect("expected collapsed split right hunk shell bounds after scroll");
    let (left_row_after_x, right_row_after_x, left_offset_after_x, right_offset_after_x) = cx
        .update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let left_row_x: f32 = pane
                .diff_text_hitboxes
                .get(&(file_visible_ix, DiffTextRegion::SplitLeft))
                .expect("expected split-left file-row hitbox after scroll")
                .bounds
                .left()
                .into();
            let right_row_x: f32 = pane
                .diff_text_hitboxes
                .get(&(file_visible_ix, DiffTextRegion::SplitRight))
                .expect("expected split-right file-row hitbox after scroll")
                .bounds
                .left()
                .into();
            let left_offset_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
            let right_offset_x: f32 = pane
                .diff_split_right_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .x
                .into();
            (left_row_x, right_row_x, left_offset_x, right_offset_x)
        });
    let left_shell_after_x: f32 = left_shell_after.left().into();
    let right_shell_after_x: f32 = right_shell_after.left().into();

    assert!(
        left_offset_after_x < 0.0 && right_offset_after_x < 0.0,
        "expected both split columns to scroll horizontally, got left={left_offset_after_x} right={right_offset_after_x}"
    );
    assert_ne!(
        left_offset_after_x, right_offset_after_x,
        "expected split columns to keep independent horizontal offsets when sync is disabled"
    );
    assert!(
        (left_shell_after_x - left_shell_before_x).abs() < 0.01,
        "collapsed split left hunk shell should stay pinned (before={left_shell_before_x}, after={left_shell_after_x})"
    );
    assert!(
        (right_shell_after_x - right_shell_before_x).abs() < 0.01,
        "collapsed split right hunk shell should stay pinned (before={right_shell_before_x}, after={right_shell_after_x})"
    );
    assert!(
        (left_row_after_x - left_row_before_x).abs() > 1.0,
        "collapsed split left file rows should still scroll horizontally (before={left_row_before_x}, after={left_row_after_x})"
    );
    assert!(
        (right_row_after_x - right_row_before_x).abs() > 1.0,
        "collapsed split right file rows should still scroll horizontally (before={right_row_before_x}, after={right_row_after_x})"
    );
}

#[gpui::test]
fn collapsed_diff_inline_reveal_buttons_expand_context_without_creating_selection(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(193);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_inline_buttons",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, visible_before, hidden_up_before, hidden_down_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let hunk = pane
                .collapsed_diff_hunks
                .first()
                .copied()
                .expect("expected collapsed inline fixture to expose one hunk");
            (
                hunk.src_ix,
                pane.diff_visible_len(),
                pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
                pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
            )
        });

    assert!(
        hidden_up_before >= 20 && hidden_down_before >= 20,
        "fixture should expose enough hidden context for inline reveal buttons"
    );

    let up_click = debug_selector_center(cx, "collapsed_diff_inline_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 20,
            "clicking the inline reveal-up gutter button should add 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            hidden_up_before - 20,
            "clicking the inline reveal-up gutter button should reduce the hidden-up budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    let down_click = debug_selector_center(cx, "collapsed_diff_inline_hunk_down");
    simulate_counted_click(cx, down_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 40,
            "clicking the inline reveal-down gutter button should add another 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before - 20,
            "clicking the inline reveal-down gutter button should reduce the hidden-down budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });
}

#[gpui::test]
fn collapsed_diff_split_reveal_buttons_expand_context_without_creating_selection(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(194);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_buttons",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, visible_before, hidden_up_before, hidden_down_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let hunk = pane
                .collapsed_diff_hunks
                .first()
                .copied()
                .expect("expected collapsed split fixture to expose one hunk");
            (
                hunk.src_ix,
                pane.diff_visible_len(),
                pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
                pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
            )
        });

    assert!(
        hidden_up_before >= 20 && hidden_down_before >= 20,
        "fixture should expose enough hidden context for split reveal buttons"
    );

    let up_click = debug_selector_center(cx, "collapsed_diff_split_left_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 20,
            "clicking the split reveal-up gutter button should add 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            hidden_up_before - 20,
            "clicking the split reveal-up gutter button should reduce the hidden-up budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    let down_click = debug_selector_center(cx, "collapsed_diff_split_left_hunk_down");
    simulate_counted_click(cx, down_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 40,
            "clicking the split reveal-down gutter button should add another 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before - 20,
            "clicking the split reveal-down gutter button should reduce the hidden-down budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });
}

#[gpui::test]
fn collapsed_diff_split_reveal_arrows_show_directional_tooltips(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(203);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_reveal_arrow_tooltips",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );

    let up_hover = debug_selector_center(cx, "collapsed_diff_split_left_hunk_up");
    cx.simulate_mouse_move(up_hover, None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Show hidden lines above".into())
    );

    let down_hover = debug_selector_center(cx, "collapsed_diff_split_left_hunk_down");
    cx.simulate_mouse_move(down_hover, None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Show hidden lines below".into())
    );
}

#[gpui::test]
fn collapsed_diff_inline_up_reveal_keeps_header_above_revealed_context(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(199);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_inline_anchor_up",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, hunk_base_row_start) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .first()
            .map(|hunk| (hunk.src_ix, hunk.base_row_start))
            .expect("expected collapsed inline fixture to expose one hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed inline reveal-up anchor becomes scrollable",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    let hunk_visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, hunk_visible_ix);

    let scroll_y_before = diff_scroll_offset_y(cx, &view);
    let header_top_before =
        diff_text_hitbox_top_for_visible_ix(cx, &view, hunk_visible_ix, DiffTextRegion::Inline);
    let hunk_first_visible_ix_before = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });

    let up_click = debug_selector_center(cx, "collapsed_diff_inline_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    let hunk_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix);
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader { .. })
            ),
            "expected the hunk header to remain visible after a partial upward reveal"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix + 1),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix })
                    if row_ix < hunk_base_row_start
            ),
            "expected newly revealed upward context to appear below the collapsed hunk header"
        );
        visible_ix
    });
    let hunk_first_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });
    let header_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        hunk_visible_ix_after,
        DiffTextRegion::Inline,
    );
    let scroll_y_after = diff_scroll_offset_y(cx, &view);

    assert!(
        (scroll_y_after - scroll_y_before).abs() < 0.01,
        "expected reveal-up to keep the inline diff scroll offset unchanged (before={scroll_y_before}, after={scroll_y_after})"
    );
    assert_eq!(
        hunk_visible_ix_after, hunk_visible_ix,
        "expected the collapsed inline hunk header to stay at the hidden-context boundary"
    );
    assert!(
        (header_top_after - header_top_before).abs() < 0.01,
        "expected the collapsed inline hunk header to remain visually fixed while revealed context is inserted below it (before={header_top_before}, after={header_top_after})"
    );
    assert!(
        hunk_first_visible_ix_after > hunk_first_visible_ix_before,
        "expected the hunk body to move down below newly revealed upward context (before={hunk_first_visible_ix_before}, after={hunk_first_visible_ix_after})"
    );
}

#[gpui::test]
fn collapsed_diff_split_up_reveal_keeps_header_above_revealed_context(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(202);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_anchor_up",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    let (hunk_src_ix, hunk_base_row_start) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .first()
            .map(|hunk| (hunk.src_ix, hunk.base_row_start))
            .expect("expected collapsed split fixture to expose one hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split reveal-up anchor becomes scrollable",
        |pane| {
            pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0)
                && pane
                    .diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .y
                    > px(0.0)
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    let hunk_visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, hunk_visible_ix);

    let left_scroll_y_before = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_before = diff_split_right_scroll_offset_y(cx, &view);
    let left_top_before =
        diff_text_hitbox_top_for_visible_ix(cx, &view, hunk_visible_ix, DiffTextRegion::SplitLeft);
    let right_top_before =
        diff_text_hitbox_top_for_visible_ix(cx, &view, hunk_visible_ix, DiffTextRegion::SplitRight);
    let hunk_first_visible_ix_before = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });

    let up_click = debug_selector_center(cx, "collapsed_diff_split_left_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    let hunk_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix);
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader { .. })
            ),
            "expected the split hunk header to remain visible after a partial upward reveal"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix + 1),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix })
                    if row_ix < hunk_base_row_start
            ),
            "expected newly revealed split upward context to appear below the collapsed hunk header"
        );
        visible_ix
    });
    let hunk_first_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });
    let left_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        hunk_visible_ix_after,
        DiffTextRegion::SplitLeft,
    );
    let right_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        hunk_visible_ix_after,
        DiffTextRegion::SplitRight,
    );
    let left_scroll_y_after = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_after = diff_split_right_scroll_offset_y(cx, &view);

    assert!(
        (left_scroll_y_after - left_scroll_y_before).abs() < 0.01,
        "expected reveal-up to keep the split-left scroll offset unchanged (before={left_scroll_y_before}, after={left_scroll_y_after})"
    );
    assert!(
        (right_scroll_y_after - right_scroll_y_before).abs() < 0.01,
        "expected reveal-up to keep the split-right scroll offset unchanged (before={right_scroll_y_before}, after={right_scroll_y_after})"
    );
    assert_eq!(
        hunk_visible_ix_after, hunk_visible_ix,
        "expected the collapsed split hunk header to stay at the hidden-context boundary"
    );
    assert!(
        (left_top_after - left_top_before).abs() < 0.01,
        "expected the split-left collapsed hunk header to remain visually fixed while revealed context is inserted below it (before={left_top_before}, after={left_top_after})"
    );
    assert!(
        (right_top_after - right_top_before).abs() < 0.01,
        "expected the split-right collapsed hunk header to remain visually fixed while revealed context is inserted below it (before={right_top_before}, after={right_top_after})"
    );
    assert!(
        hunk_first_visible_ix_after > hunk_first_visible_ix_before,
        "expected the split hunk body to move down below newly revealed upward context (before={hunk_first_visible_ix_before}, after={hunk_first_visible_ix_after})"
    );
}

#[gpui::test]
fn collapsed_diff_split_down_before_reveal_moves_both_columns_without_vertical_sync(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(200);
    let (unified, old_text, new_text) = build_collapsed_diff_long_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_anchor_down_before",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    let second_hunk_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .get(1)
            .map(|hunk| hunk.src_ix)
            .expect("expected long-gap fixture to expose a second collapsed hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, second_hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split down-before anchor becomes scrollable",
        |pane| {
            pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0)
                && pane
                    .diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .y
                    > px(0.0)
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    let target_visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_hunk_visible_ix_for_src_ix(pane, second_hunk_src_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, target_visible_ix);

    let left_scroll_y_before = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_before = diff_split_right_scroll_offset_y(cx, &view);
    let left_top_before = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix,
        DiffTextRegion::SplitLeft,
    );
    let right_top_before = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix,
        DiffTextRegion::SplitRight,
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down_before(second_hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let target_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, second_hunk_src_ix);
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader { .. })
            ),
            "expected the second collapsed hunk header to remain visible after a partial down-before reveal"
        );
        visible_ix
    });
    let left_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix_after,
        DiffTextRegion::SplitLeft,
    );
    let right_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix_after,
        DiffTextRegion::SplitRight,
    );
    let left_scroll_y_after = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_after = diff_split_right_scroll_offset_y(cx, &view);

    assert!(
        (left_scroll_y_after - left_scroll_y_before).abs() < 0.01,
        "expected down-before reveal to keep the split-left scroll offset unchanged (before={left_scroll_y_before}, after={left_scroll_y_after})"
    );
    assert!(
        (right_scroll_y_after - right_scroll_y_before).abs() < 0.01,
        "expected down-before reveal to keep the split-right scroll offset unchanged (before={right_scroll_y_before}, after={right_scroll_y_after})"
    );
    assert!(
        left_top_after > left_top_before,
        "expected the split-left collapsed hunk header to move down during down-before reveal (before={left_top_before}, after={left_top_after})"
    );
    assert!(
        right_top_after > right_top_before,
        "expected the split-right collapsed hunk header to move down during down-before reveal (before={right_top_before}, after={right_top_after})"
    );
}

#[gpui::test]
fn collapsed_diff_short_gap_merge_moves_following_file_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(201);
    let (unified, old_text, new_text) = build_collapsed_diff_short_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_short_gap_anchor",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let second_hunk_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .get(1)
            .map(|hunk| hunk.src_ix)
            .expect("expected short-gap fixture to expose a second collapsed hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, second_hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed short-gap merge becomes scrollable",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    let (target_visible_ix, tracked_row_ix) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, second_hunk_src_ix);
        let row_ix = match pane.collapsed_visible_row(visible_ix + 1) {
            Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix }) => row_ix,
            other => panic!("expected a file row after the short-gap header, got {other:?}"),
        };
        (visible_ix, row_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, target_visible_ix);

    let tracked_visible_ix_before = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, tracked_row_ix)
    });
    let scroll_y_before = diff_scroll_offset_y(cx, &view);
    let row_top_before = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        tracked_visible_ix_before,
        DiffTextRegion::Inline,
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_short(second_hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let tracked_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, tracked_row_ix)
    });
    let row_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        tracked_visible_ix_after,
        DiffTextRegion::Inline,
    );
    let scroll_y_after = diff_scroll_offset_y(cx, &view);

    assert!(
        (scroll_y_after - scroll_y_before).abs() < 0.01,
        "expected short-gap merge to keep the inline diff scroll offset unchanged (before={scroll_y_before}, after={scroll_y_after})"
    );
    assert!(
        row_top_after > row_top_before,
        "expected the first visible row after a short-gap merge to move down when rows are inserted before it (before={row_top_before}, after={row_top_after})"
    );
}

fn fixture_git_command(repo_root: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new("git");
    command
        .current_dir(repo_root)
        .args(["-c", &format!("safe.directory={}", repo_root.display())]);
    command
}

fn fixture_git_show(repo_root: &std::path::Path, spec: &str, context: &str) -> String {
    let output = fixture_git_command(repo_root)
        .args(["show", spec])
        .output()
        .unwrap_or_else(|_| panic!("git show should run for {context}"));
    assert!(
        output.status.success(),
        "git show {spec} failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("git show output should be valid UTF-8")
}

fn fixture_git_diff(
    repo_root: &std::path::Path,
    old_spec: &str,
    new_spec: &str,
    context: &str,
) -> String {
    let output = fixture_git_command(repo_root)
        .args(["diff", old_spec, new_spec])
        .output()
        .unwrap_or_else(|_| panic!("git diff should run for {context}"));
    assert!(
        output.status.success(),
        "git diff for {context} failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("git diff output should be valid UTF-8")
}

#[gpui::test]
fn patch_view_applies_syntax_highlighting_to_context_lines(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(2);
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_patch", std::process::id()));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let target = gitcomet_core::domain::DiffTarget::Commit {
                commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
                path: None,
            };

            let diff = gitcomet_core::domain::Diff {
                target: target.clone(),
                lines: vec![
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Header,
                        text: "diff --git a/foo.rs b/foo.rs".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Hunk,
                        text: "@@ -1,1 +1,1 @@".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Context,
                        text: " fn main() { let x = 1; }".into(),
                    },
                ],
            };

            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target);
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(diff.into());

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let styled = pane
            .diff_text_segments_cache
            .get(2)
            .and_then(|v| v.as_ref().map(|entry| &entry.styled))
            .expect("expected context line to be syntax-highlighted and cached");
        assert!(
            !styled.highlights.is_empty(),
            "expected syntax highlighting highlights for context line"
        );
    });
}

#[gpui::test]
fn patch_diff_text_multi_clicks_match_editor_selection_behavior(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(901);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_patch_diff_text_multi_clicks",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/multi_click.rs");
    let old_text = "alpha_beta = delta;\n";
    let new_text = "alpha_beta = gamma;\n";

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, old_text, new_text);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Inline;
            cx.notify();
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file diff multi-click fixture activation",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.diff_visible_len() >= 1
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.text.as_ref().contains("gamma"))
        },
        |pane| {
            format!(
                "cache_inflight={:?} cache_path={:?} diff_view={:?} visible_len={} inline_rows={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.diff_view,
                pane.diff_visible_len(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| format!("{:?}:{}", line.kind, line.text.as_ref()))
                    .collect::<Vec<_>>(),
            )
        },
    );

    let (visible_ix, expected_line) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = (0..pane.diff_visible_len())
            .find(|&visible_ix| {
                let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return false;
                };
                pane.file_diff_inline_row(inline_ix)
                    .is_some_and(|line| line.text.as_ref().contains("gamma"))
            })
            .expect("expected visible file-diff row for changed line");
        let expected_line = pane
            .diff_text_line_for_region(visible_ix, DiffTextRegion::Inline)
            .to_string();
        (visible_ix, expected_line)
    });
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        visible_ix,
        DiffTextRegion::Inline,
        2..6,
        "file diff multi-click target row hitbox",
    );
    let expected_word = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let offset = pane
            .diff_text_offset_for_position_for_tests(visible_ix, DiffTextRegion::Inline, click)
            .expect("expected diff text offset for click");
        let word_range = crate::text_selection::token_range_for_offset(&expected_line, offset);
        expected_line[word_range].to_string()
    });

    simulate_counted_click(cx, click, 2);

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.copy_selected_diff_text_to_clipboard(cx)
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected_word.clone())
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_range, None,
            "double click on diff text should not also select the row"
        );
    });

    simulate_counted_click(cx, click, 3);

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.copy_selected_diff_text_to_clipboard(cx)
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected_line)
    );

    simulate_counted_click(cx, click, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.diff_text_has_selection(),
            "single click should clear the text selection"
        );
        assert_eq!(
            pane.diff_selection_range, None,
            "single click used to clear text selection should not trigger row selection"
        );
    });
}

#[gpui::test]
fn yaml_commit_file_diff_keeps_consistent_highlighting_for_added_paths_and_keys(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn quoted_scalar_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start == quote_start && range.end == text.len()).then_some(color)
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= dash_ix && range.end >= dash_ix.saturating_add(1)).then_some(color)
        })
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            FileDiffRowKind,
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let payload = split_right_cached_styled_by_new_line(pane, line_no).and_then(
                    |(_text, styled)| {
                        let kind = split_right_row_by_new_line(pane, line_no)?.kind;
                        Some((
                            kind,
                            styled.text.to_string(),
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        ))
                    },
                );
                (line_no, payload)
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            DiffLineKind,
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let payload =
                    inline_cached_styled_by_new_line(pane, line_no).and_then(|(_text, styled)| {
                        let kind = inline_row_by_new_line(pane, line_no)?.kind;
                        Some((
                            kind,
                            styled.text.to_string(),
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        ))
                    });
                (line_no, payload)
            })
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(81);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_file_diff",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "YAML commit file-diff regression fixture");
    let git_diff = || {
        fixture_git_diff(
            &repo_root,
            "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml",
            "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml",
            "YAML commit file-diff regression fixture",
        )
    };
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");
    let unified = git_diff();

    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(path.clone()),
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    let baseline_path_line = 17u32;
    let affected_path_lines = [18u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_nested_key_line = 4u32;
    let affected_nested_key_lines = [19u32, 34u32];
    let baseline_top_key_line = 3u32;
    let affected_top_key_lines = [36u32];
    let affected_add_lines = [18u32, 33u32];
    let affected_context_lines = [19u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 34, 36];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });

            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.clone()),
                    Some(new_text.clone()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_repo_id == Some(repo_id)
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "repo_id={:?} rev={} target={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit split syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some(baseline_path_color) =
                quoted_scalar_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };

            if affected_add_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_color(styled, text) != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} split_debug={:?}",
                pane.diff_view,
                split_debug(pane, &lines),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit inline syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some(baseline_path_color) =
                quoted_scalar_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };

            if affected_add_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_color(styled, text) != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, &lines),
            )
        },
    );
}

#[gpui::test]
fn yaml_commit_patch_diff_keeps_consistent_highlighting_for_added_paths_and_keys(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(
        FileDiffRowKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &super::CachedDiffStyledText,
    )> {
        for row_ix in 0..pane.patch_diff_split_row_len() {
            let PatchSplitRow::Aligned {
                row, new_src_ix, ..
            } = pane.patch_diff_split_row(row_ix)?
            else {
                continue;
            };
            if row.new_line != Some(new_line) {
                continue;
            }
            let src_ix = new_src_ix?;
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                row.kind,
                src_ix,
                row.new.as_deref()?.to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(
        DiffLineKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &super::CachedDiffStyledText,
    )> {
        for src_ix in 0..pane.patch_diff_row_len() {
            let line = pane.patch_diff_row(src_ix)?;
            if line.new_line != Some(new_line) {
                continue;
            }
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                line.kind,
                src_ix,
                diff_content_text(&line).to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn quoted_scalar_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start == quote_start && range.end == text.len()).then_some(color)
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= dash_ix && range.end >= dash_ix.saturating_add(1)).then_some(color)
        })
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            FileDiffRowKind,
            Option<rows::DiffSyntaxLanguage>,
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let payload = split_right_cached_styled_by_new_line(pane, line_no).map(
                    |(kind, _src_ix, text, language, styled)| {
                        (
                            kind,
                            language,
                            text,
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        )
                    },
                );
                (line_no, payload)
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            DiffLineKind,
            Option<rows::DiffSyntaxLanguage>,
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let payload = inline_cached_styled_by_new_line(pane, line_no).map(
                    |(kind, _src_ix, text, language, styled)| {
                        (
                            kind,
                            language,
                            text,
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        )
                    },
                );
                (line_no, payload)
            })
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(82);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_diff",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let repo_root = fixture_repo_root();
    let unified = fixture_git_diff(
        &repo_root,
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml",
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml",
        "YAML commit patch-diff regression fixture",
    );

    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    let baseline_path_line = 17u32;
    let affected_path_lines = [18u32, 30, 31, 32, 33];
    let baseline_key_line = 19u32;
    let affected_key_lines = [21u32, 34u32, 36u32];
    let affected_add_lines = [18u32, 33u32];
    let affected_context_lines = [21u32, 30, 31, 32, 34, 36];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit patch-diff cache and language assignment",
        |pane| {
            pane.patch_diff_row_len() > 0
                && pane.patch_diff_split_row_len() > 0
                && pane.diff_language_for_src_ix.len() == pane.patch_diff_row_len()
                && (0..pane.patch_diff_row_len()).any(|src_ix| {
                    pane.patch_diff_row(src_ix)
                        .is_some_and(|line| line.new_line == Some(36))
                })
        },
        |pane| {
            format!(
                "diff_view={:?} rows={} split_rows={} visible_len={} languages={:?}",
                pane.diff_view,
                pane.patch_diff_row_len(),
                pane.patch_diff_split_row_len(),
                pane.diff_visible_len(),
                (0..pane.patch_diff_row_len())
                    .filter_map(|src_ix| {
                        pane.patch_diff_row(src_ix).map(|line| {
                            (
                                src_ix,
                                line.kind,
                                line.new_line,
                                pane.diff_language_for_src_ix.get(src_ix).copied().flatten(),
                            )
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit patch split syntax stays consistent for added paths and keys",
        |pane| {
            let Some((
                baseline_kind,
                _baseline_src_ix,
                baseline_text,
                baseline_language,
                baseline_styled,
            )) = split_right_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            if baseline_kind != FileDiffRowKind::Context
                || baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_dash_color) = list_item_dash_color(baseline_styled, &baseline_text)
            else {
                return false;
            };
            let Some(baseline_path_color) = quoted_scalar_color(baseline_styled, &baseline_text)
            else {
                return false;
            };

            if affected_add_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, &text) != Some(baseline_dash_color)
                    || quoted_scalar_color(styled, &text) != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((
                baseline_key_kind,
                _baseline_key_src_ix,
                baseline_key_text,
                baseline_key_language,
                baseline_key_styled,
            )) = split_right_cached_styled_by_new_line(pane, baseline_key_line)
            else {
                return false;
            };
            if baseline_key_kind != FileDiffRowKind::Context
                || baseline_key_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_key_color) =
                mapping_key_color(baseline_key_styled, &baseline_key_text)
            else {
                return false;
            };
            !affected_key_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, &text) != Some(baseline_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_key_line);
            lines.extend(affected_key_lines);
            format!(
                "diff_view={:?} split_debug={:?}",
                pane.diff_view,
                split_debug(pane, &lines),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit patch inline syntax stays consistent for added paths and keys",
        |pane| {
            let Some((
                baseline_kind,
                _baseline_src_ix,
                baseline_text,
                baseline_language,
                baseline_styled,
            )) = inline_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            if baseline_kind != DiffLineKind::Context
                || baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_dash_color) = list_item_dash_color(baseline_styled, &baseline_text)
            else {
                return false;
            };
            let Some(baseline_path_color) = quoted_scalar_color(baseline_styled, &baseline_text)
            else {
                return false;
            };

            if affected_add_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    inline_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, &text) != Some(baseline_dash_color)
                    || quoted_scalar_color(styled, &text) != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((
                baseline_key_kind,
                _baseline_key_src_ix,
                baseline_key_text,
                baseline_key_language,
                baseline_key_styled,
            )) = inline_cached_styled_by_new_line(pane, baseline_key_line)
            else {
                return false;
            };
            if baseline_key_kind != DiffLineKind::Context
                || baseline_key_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_key_color) =
                mapping_key_color(baseline_key_styled, &baseline_key_text)
            else {
                return false;
            };
            !affected_key_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    inline_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, &text) != Some(baseline_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_key_line);
            lines.extend(affected_key_lines);
            format!(
                "diff_view={:?} inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, &lines),
            )
        },
    );
}

#[gpui::test]
fn yaml_commit_patch_diff_full_fixture_keeps_consistent_highlighting_across_files(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_cached_styled_by_file_and_new_line<'a>(
        pane: &'a MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<(
        FileDiffRowKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &'a super::CachedDiffStyledText,
    )> {
        for row_ix in 0..pane.patch_diff_split_row_len() {
            let PatchSplitRow::Aligned {
                row, new_src_ix, ..
            } = pane.patch_diff_split_row(row_ix)?
            else {
                continue;
            };
            if row.new_line != Some(new_line) {
                continue;
            }
            let src_ix = new_src_ix?;
            if pane
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|path| path.as_deref())
                != Some(file_path)
            {
                continue;
            }
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                row.kind,
                src_ix,
                row.new.as_deref()?.to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn inline_cached_styled_by_file_and_new_line<'a>(
        pane: &'a MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<(
        DiffLineKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &'a super::CachedDiffStyledText,
    )> {
        for src_ix in 0..pane.patch_diff_row_len() {
            let line = pane.patch_diff_row(src_ix)?;
            if line.new_line != Some(new_line) {
                continue;
            }
            if pane
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|path| path.as_deref())
                != Some(file_path)
            {
                continue;
            }
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                line.kind,
                src_ix,
                diff_content_text(&line).to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn quoted_scalar_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start == quote_start && range.end == text.len()).then_some(color)
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= dash_ix && range.end >= dash_ix.saturating_add(1)).then_some(color)
        })
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= key_start && range.end >= key_end).then_some(color)
        })
    }

    fn scalar_color_after_colon(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let value_start = text.find(':')?.checked_add(1).and_then(|start| {
            text[start..]
                .find(|ch: char| !ch.is_ascii_whitespace())
                .map(|offset| start.saturating_add(offset))
        })?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= value_start && range.end > value_start).then_some(color)
        })
    }

    fn split_debug(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            FileDiffRowKind,
            Option<rows::DiffSyntaxLanguage>,
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let payload = split_right_cached_styled_by_file_and_new_line(
                    pane, file_path, line_no,
                )
                .map(|(kind, _src_ix, text, language, styled)| {
                    (
                        kind,
                        language,
                        text,
                        styled
                            .highlights
                            .iter()
                            .map(|(range, style)| {
                                (range.clone(), style.color, style.background_color)
                            })
                            .collect(),
                    )
                });
                (line_no, payload)
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            DiffLineKind,
            Option<rows::DiffSyntaxLanguage>,
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let payload = inline_cached_styled_by_file_and_new_line(pane, file_path, line_no)
                    .map(|(kind, _src_ix, text, language, styled)| {
                        (
                            kind,
                            language,
                            text,
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        )
                    });
                (line_no, payload)
            })
            .collect()
    }

    fn split_visible_ix_by_file_and_new_line(
        pane: &MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            let Some(PatchSplitRow::Aligned {
                row, new_src_ix, ..
            }) = pane.patch_diff_split_row(row_ix)
            else {
                return false;
            };
            let Some(src_ix) = new_src_ix else {
                return false;
            };
            row.new_line == Some(new_line)
                && pane
                    .diff_file_for_src_ix
                    .get(src_ix)
                    .and_then(|path| path.as_deref())
                    == Some(file_path)
        })
    }

    fn inline_visible_ix_by_file_and_new_line(
        pane: &MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(src_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            let Some(line) = pane.patch_diff_row(src_ix) else {
                return false;
            };
            line.new_line == Some(new_line)
                && pane
                    .diff_file_for_src_ix
                    .get(src_ix)
                    .and_then(|path| path.as_deref())
                    == Some(file_path)
        })
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    #[derive(Clone, Copy, Debug)]
    struct ExpectedPaintRow {
        line_no: u32,
        visible_ix: usize,
        expects_add_bg: bool,
    }

    fn split_draw_rows_for_lines(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = split_visible_ix_by_file_and_new_line(pane, file_path, line_no)
                    .unwrap_or_else(|| {
                        panic!("expected split visible row for {file_path} line {line_no}")
                    });
                let row_ix = pane
                    .diff_mapped_ix_for_visible_ix(visible_ix)
                    .unwrap_or_else(|| {
                        panic!("expected split mapped row for {file_path} line {line_no}")
                    });
                let PatchSplitRow::Aligned { row, .. } =
                    pane.patch_diff_split_row(row_ix).unwrap_or_else(|| {
                        panic!("expected aligned split row for {file_path} line {line_no}")
                    })
                else {
                    panic!("expected aligned split row for {file_path} line {line_no}");
                };
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg: row.kind == FileDiffRowKind::Add,
                }
            })
            .collect()
    }

    fn inline_draw_rows_for_lines(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = inline_visible_ix_by_file_and_new_line(pane, file_path, line_no)
                    .unwrap_or_else(|| {
                        panic!("expected inline visible row for {file_path} line {line_no}")
                    });
                let src_ix = pane
                    .diff_mapped_ix_for_visible_ix(visible_ix)
                    .unwrap_or_else(|| {
                        panic!("expected inline mapped row for {file_path} line {line_no}")
                    });
                let kind = pane
                    .patch_diff_row(src_ix)
                    .unwrap_or_else(|| {
                        panic!("expected inline diff row for {file_path} line {line_no}")
                    })
                    .kind;
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg: kind == DiffLineKind::Add,
                }
            })
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    fn assert_split_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        file_path: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::SplitRight,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let Some((_kind, _src_ix, text, _language, styled)) =
                    split_right_cached_styled_by_file_and_new_line(
                        pane,
                        file_path,
                        expected.line_no,
                    )
                else {
                    panic!(
                        "expected cached split-right styled text for {file_path} line {}",
                        expected.line_no
                    );
                };
                (text, highlight_snapshot(styled.highlights.as_ref()))
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    fn assert_inline_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        file_path: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::Inline,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let Some((_kind, _src_ix, text, _language, styled)) =
                    inline_cached_styled_by_file_and_new_line(pane, file_path, expected.line_no)
                else {
                    panic!(
                        "expected cached inline styled text for {file_path} line {}",
                        expected.line_no
                    );
                };
                (text, highlight_snapshot(styled.highlights.as_ref()))
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(85);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_full_fixture",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let unified =
        std::fs::read_to_string(fixture_repo_root().join("test_data/commit-bd8b4a04.patch"))
            .expect("should read multi-file YAML commit patch regression fixture");
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    let build_release_file = ".github/workflows/build-release-artifacts.yml";
    let build_release_baseline_secret_key_line = 20u32;
    let build_release_affected_secret_key_lines = [22u32, 24u32];
    let build_release_baseline_required_line = 21u32;
    let build_release_affected_required_lines = [23u32];
    let build_release_add_lines = [20u32, 21u32];
    let build_release_context_lines = [22u32, 23u32, 24u32];
    let build_release_draw_lines = [20u32, 21, 22, 23, 24];

    let deployment_file = ".github/workflows/deployment-ci.yml";
    let deployment_baseline_path_line = 17u32;
    let deployment_affected_path_lines = [18u32, 30u32, 31u32, 32u32, 33u32];
    let deployment_baseline_key_line = 19u32;
    let deployment_affected_key_lines = [21u32, 34u32, 36u32];
    let deployment_add_lines = [18u32, 33u32];
    let deployment_context_lines = [21u32, 30u32, 31u32, 32u32, 34u32, 36u32];
    let deployment_draw_lines = [17u32, 18, 19, 21, 30, 31, 32, 33, 34, 36];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch-diff cache and language assignment",
        |pane| {
            pane.patch_diff_row_len() > 0
                && pane.patch_diff_split_row_len() > 0
                && pane.diff_language_for_src_ix.len() == pane.patch_diff_row_len()
                && (0..pane.patch_diff_row_len()).any(|src_ix| {
                    pane.patch_diff_row(src_ix).is_some_and(|line| {
                        line.new_line == Some(36)
                            && pane
                                .diff_file_for_src_ix
                                .get(src_ix)
                                .and_then(|path| path.as_deref())
                                == Some(deployment_file)
                    })
                })
                && (0..pane.patch_diff_row_len()).any(|src_ix| {
                    pane.patch_diff_row(src_ix).is_some_and(|line| {
                        line.new_line == Some(24)
                            && pane
                                .diff_file_for_src_ix
                                .get(src_ix)
                                .and_then(|path| path.as_deref())
                                == Some(build_release_file)
                    })
                })
        },
        |pane| {
            format!(
                "diff_view={:?} rows={} split_rows={} visible_len={} files={:?}",
                pane.diff_view,
                pane.patch_diff_row_len(),
                pane.patch_diff_split_row_len(),
                pane.diff_visible_len(),
                (0..pane.patch_diff_row_len())
                    .filter_map(|src_ix| {
                        pane.patch_diff_row(src_ix).map(|line| {
                            (
                                src_ix,
                                pane.diff_file_for_src_ix
                                    .get(src_ix)
                                    .and_then(|path| path.as_deref())
                                    .map(str::to_owned),
                                line.kind,
                                line.new_line,
                                pane.diff_language_for_src_ix.get(src_ix).copied().flatten(),
                            )
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch split syntax stays consistent for build-release top hunk",
        |pane| {
            let Some((
                build_release_baseline_kind,
                _build_release_baseline_src_ix,
                build_release_baseline_text,
                build_release_baseline_language,
                build_release_baseline_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_secret_key_line,
            )
            else {
                return false;
            };
            if build_release_baseline_kind != FileDiffRowKind::Add
                || build_release_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(build_release_baseline_key_color) =
                mapping_key_color(build_release_baseline_styled, &build_release_baseline_text)
            else {
                return false;
            };
            if build_release_add_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_context_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_affected_secret_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(build_release_baseline_key_color)
                })
            {
                return false;
            }

            let Some((
                _build_release_required_kind,
                _build_release_required_src_ix,
                build_release_required_text,
                _build_release_required_language,
                build_release_required_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_required_line,
            )
            else {
                return false;
            };
            let Some(build_release_required_color) = scalar_color_after_colon(
                build_release_required_styled,
                &build_release_required_text,
            ) else {
                return false;
            };
            !build_release_affected_required_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    scalar_color_after_colon(styled, &text) != Some(build_release_required_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} build_release_split_debug={:?}",
                pane.diff_view,
                split_debug(pane, build_release_file, &build_release_draw_lines),
            )
        },
    );

    let build_release_split_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, build_release_file, &build_release_draw_lines)
    });
    assert_split_rows_match_render_cache(
        cx,
        &view,
        "build-release split",
        build_release_file,
        build_release_split_expected,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_diff_visible_indices();
                let target_visible_ix = split_visible_ix_by_file_and_new_line(
                    pane,
                    deployment_file,
                    deployment_baseline_path_line,
                )
                .expect("deployment workflow should have a visible split row in the full fixture");
                pane.scroll_diff_to_item_strict(target_visible_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch split syntax stays consistent for deployment workflow rows",
        |pane| {
            let Some((
                deployment_baseline_kind,
                _deployment_baseline_src_ix,
                deployment_baseline_text,
                deployment_baseline_language,
                deployment_baseline_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_path_line,
            )
            else {
                return false;
            };
            if deployment_baseline_kind != FileDiffRowKind::Context
                || deployment_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(deployment_baseline_dash_color) =
                list_item_dash_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            let Some(deployment_baseline_path_color) =
                quoted_scalar_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            if deployment_add_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_context_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_affected_path_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            deployment_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    list_item_dash_color(styled, &text) != Some(deployment_baseline_dash_color)
                        || quoted_scalar_color(styled, &text)
                            != Some(deployment_baseline_path_color)
                })
            {
                return false;
            }

            let Some((
                _deployment_key_kind,
                _deployment_key_src_ix,
                deployment_key_text,
                _deployment_key_language,
                deployment_key_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_key_line,
            )
            else {
                return false;
            };
            let Some(deployment_key_color) =
                mapping_key_color(deployment_key_styled, &deployment_key_text)
            else {
                return false;
            };
            !deployment_affected_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            deployment_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(deployment_key_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} deployment_split_debug={:?}",
                pane.diff_view,
                split_debug(pane, deployment_file, &deployment_draw_lines),
            )
        },
    );

    let deployment_split_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, deployment_file, &deployment_draw_lines)
    });
    assert_split_rows_match_render_cache(
        cx,
        &view,
        "deployment split",
        deployment_file,
        deployment_split_expected,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_diff_visible_indices();
                let target_visible_ix = inline_visible_ix_by_file_and_new_line(
                    pane,
                    build_release_file,
                    build_release_baseline_secret_key_line,
                )
                .expect(
                    "build-release workflow should have a visible inline row in the full fixture",
                );
                pane.scroll_diff_to_item_strict(target_visible_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch inline syntax stays consistent for build-release top hunk",
        |pane| {
            let Some((
                build_release_baseline_kind,
                _build_release_baseline_src_ix,
                build_release_baseline_text,
                build_release_baseline_language,
                build_release_baseline_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_secret_key_line,
            )
            else {
                return false;
            };
            if build_release_baseline_kind != DiffLineKind::Add
                || build_release_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(build_release_baseline_key_color) =
                mapping_key_color(build_release_baseline_styled, &build_release_baseline_text)
            else {
                return false;
            };
            if build_release_add_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_context_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_affected_secret_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(build_release_baseline_key_color)
                })
            {
                return false;
            }

            let Some((
                _build_release_required_kind,
                _build_release_required_src_ix,
                build_release_required_text,
                _build_release_required_language,
                build_release_required_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_required_line,
            )
            else {
                return false;
            };
            let Some(build_release_required_color) = scalar_color_after_colon(
                build_release_required_styled,
                &build_release_required_text,
            ) else {
                return false;
            };
            !build_release_affected_required_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    scalar_color_after_colon(styled, &text) != Some(build_release_required_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} build_release_inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, build_release_file, &build_release_draw_lines),
            )
        },
    );

    let build_release_inline_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, build_release_file, &build_release_draw_lines)
    });
    assert_inline_rows_match_render_cache(
        cx,
        &view,
        "build-release inline",
        build_release_file,
        build_release_inline_expected,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_diff_visible_indices();
                let target_visible_ix = inline_visible_ix_by_file_and_new_line(
                    pane,
                    deployment_file,
                    deployment_baseline_path_line,
                )
                .expect("deployment workflow should have a visible inline row in the full fixture");
                pane.scroll_diff_to_item_strict(target_visible_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch inline syntax stays consistent for deployment workflow rows",
        |pane| {
            let Some((
                deployment_baseline_kind,
                _deployment_baseline_src_ix,
                deployment_baseline_text,
                deployment_baseline_language,
                deployment_baseline_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_path_line,
            )
            else {
                return false;
            };
            if deployment_baseline_kind != DiffLineKind::Context
                || deployment_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(deployment_baseline_dash_color) =
                list_item_dash_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            let Some(deployment_baseline_path_color) =
                quoted_scalar_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            if deployment_add_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_context_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_affected_path_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    else {
                        return true;
                    };
                    list_item_dash_color(styled, &text) != Some(deployment_baseline_dash_color)
                        || quoted_scalar_color(styled, &text)
                            != Some(deployment_baseline_path_color)
                })
            {
                return false;
            }

            let Some((
                _deployment_key_kind,
                _deployment_key_src_ix,
                deployment_key_text,
                _deployment_key_language,
                deployment_key_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_key_line,
            )
            else {
                return false;
            };
            let Some(deployment_key_color) =
                mapping_key_color(deployment_key_styled, &deployment_key_text)
            else {
                return false;
            };
            !deployment_affected_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(deployment_key_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} deployment_inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, deployment_file, &deployment_draw_lines),
            )
        },
    );

    let deployment_inline_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, deployment_file, &deployment_draw_lines)
    });
    assert_inline_rows_match_render_cache(
        cx,
        &view,
        "deployment inline",
        deployment_file,
        deployment_inline_expected,
    );
}

#[gpui::test]
fn yaml_commit_patch_diff_matches_commit_file_diff_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn parse_hunk_start(text: &str) -> Option<(u32, u32)> {
        let text = text.strip_prefix("@@")?.trim_start();
        let text = text.split("@@").next()?.trim();
        let mut parts = text.split_whitespace();
        let old = parts.next()?.strip_prefix('-')?;
        let new = parts.next()?.strip_prefix('+')?;
        let old_start = old.split(',').next()?.parse::<u32>().ok()?;
        let new_start = new.split(',').next()?.parse::<u32>().ok()?;
        Some((old_start, new_start))
    }

    fn patch_visible_line_numbers(
        diff: &gitcomet_core::domain::Diff,
    ) -> (BTreeSet<u32>, BTreeSet<u32>) {
        let mut old_lines = BTreeSet::new();
        let mut new_lines = BTreeSet::new();
        let mut old_line = None;
        let mut new_line = None;

        for line in &diff.lines {
            match line.kind {
                DiffLineKind::Header => {}
                DiffLineKind::Hunk => {
                    if let Some((old_start, new_start)) = parse_hunk_start(line.text.as_ref()) {
                        old_line = Some(old_start);
                        new_line = Some(new_start);
                    } else {
                        old_line = None;
                        new_line = None;
                    }
                }
                DiffLineKind::Context => {
                    if let Some(line_no) = old_line {
                        old_lines.insert(line_no);
                        old_line = Some(line_no.saturating_add(1));
                    }
                    if let Some(line_no) = new_line {
                        new_lines.insert(line_no);
                        new_line = Some(line_no.saturating_add(1));
                    }
                }
                DiffLineKind::Remove => {
                    if let Some(line_no) = old_line {
                        old_lines.insert(line_no);
                        old_line = Some(line_no.saturating_add(1));
                    }
                }
                DiffLineKind::Add => {
                    if let Some(line_no) = new_line {
                        new_lines.insert(line_no);
                        new_line = Some(line_no.saturating_add(1));
                    }
                }
            }
        }

        (old_lines, new_lines)
    }

    fn one_based_line_byte_range(
        text: &str,
        line_starts: &[usize],
        line_no: u32,
    ) -> Option<std::ops::Range<usize>> {
        let line_ix = usize::try_from(line_no).ok()?.checked_sub(1)?;
        let start = (*line_starts.get(line_ix)?).min(text.len());
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        Some(start..end)
    }

    fn shared_text_and_line_starts(text: &str) -> (gpui::SharedString, Arc<[usize]>) {
        let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
        line_starts.push(0usize);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix.saturating_add(1));
            }
        }
        (text.to_string().into(), Arc::from(line_starts))
    }

    fn prepared_document_snapshot_for_line(
        theme: AppTheme,
        text: &str,
        line_starts: &[usize],
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        let byte_range = one_based_line_byte_range(text, line_starts, line_no)?;
        let line_text = text.get(byte_range.clone())?.to_string();
        let started = std::time::Instant::now();

        loop {
            let highlights = rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                text,
                line_starts,
                document,
                language,
                byte_range.clone(),
            )?;

            if !highlights.pending {
                return Some(LineSyntaxSnapshot {
                    text: line_text.clone(),
                    syntax: highlights
                        .highlights
                        .into_iter()
                        .filter(|(_, style)| style.background_color.is_none())
                        .map(|(range, style)| {
                            (
                                range.start.saturating_sub(byte_range.start)
                                    ..range.end.saturating_sub(byte_range.start),
                                style.color,
                            )
                        })
                        .collect(),
                });
            }

            let completed =
                rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if completed == 0 && started.elapsed() >= std::time::Duration::from_secs(2) {
                return None;
            }
            if completed == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn yaml_patch_snapshot_for_src_ix(
        pane: &MainPaneView,
        theme: AppTheme,
        string_color: gpui::Hsla,
        src_ix: usize,
        text: &str,
    ) -> LineSyntaxSnapshot {
        let force_full_string = pane
            .diff_yaml_block_scalar_for_src_ix
            .get(src_ix)
            .copied()
            .unwrap_or(false);

        if force_full_string {
            return LineSyntaxSnapshot {
                text: text.to_string(),
                syntax: (!text.is_empty())
                    .then_some(vec![(0..text.len(), Some(string_color))])
                    .unwrap_or_default(),
            };
        }

        let highlights = rows::syntax_highlights_for_line(
            theme,
            text,
            rows::DiffSyntaxLanguage::Yaml,
            pane.patch_diff_syntax_mode(),
        );
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: highlights
                .into_iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range, style.color))
                .collect(),
        }
    }

    fn patch_split_snapshot_by_line(
        pane: &MainPaneView,
        region: DiffTextRegion,
        theme: AppTheme,
        string_color: gpui::Hsla,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        for row_ix in 0..pane.patch_diff_split_row_len() {
            let PatchSplitRow::Aligned {
                row,
                old_src_ix,
                new_src_ix,
            } = pane.patch_diff_split_row(row_ix)?
            else {
                continue;
            };

            let (src_ix, text) = match region {
                DiffTextRegion::SplitLeft if row.old_line == Some(line_no) => {
                    (old_src_ix?, row.old.as_deref()?)
                }
                DiffTextRegion::SplitRight if row.new_line == Some(line_no) => {
                    (new_src_ix?, row.new.as_deref()?)
                }
                DiffTextRegion::Inline | DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight => {
                    continue;
                }
            };

            return Some(yaml_patch_snapshot_for_src_ix(
                pane,
                theme,
                string_color,
                src_ix,
                text,
            ));
        }

        None
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);
    let yaml_string_color = rows::syntax_highlights_for_line(
        theme,
        "\"yaml-string\"",
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
    )
    .into_iter()
    .find_map(|(_, style)| style.color)
    .expect("expected YAML string token color");

    let repo_id = gitcomet_state::model::RepoId(83);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_file_parity",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "YAML commit patch/file parity fixture");
    let unified = fixture_git_diff(
        &repo_root,
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
        "YAML commit patch/file parity fixture",
    );
    let old_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    );
    let new_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    );

    let file_target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(path.clone()),
    };
    let file_diff = gitcomet_core::domain::Diff::from_unified(file_target.clone(), &unified);
    let patch_target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let patch_diff = gitcomet_core::domain::Diff::from_unified(patch_target.clone(), &unified);
    let (visible_old_lines, visible_new_lines) = patch_visible_line_numbers(&patch_diff);
    let (old_shared_text, old_line_starts) = shared_text_and_line_starts(old_text.as_str());
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let old_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        old_shared_text,
        Arc::clone(&old_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared old YAML baseline document, got {other:?}"),
    };
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared new YAML baseline document, got {other:?}"),
    };
    let baseline_old_by_line = visible_old_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                old_text.as_str(),
                old_line_starts.as_ref(),
                old_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for old line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let baseline_new_by_line = visible_new_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for new line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });

            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(file_target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(file_diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.clone()),
                    Some(new_text.clone()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit file-diff baseline prepared syntax ready",
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);

            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && pane.file_diff_cache_repo_id == Some(repo_id)
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_target == Some(file_target.clone())
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && left_doc.is_some()
                && right_doc.is_some()
                && left_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && right_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} file_diff_active={} rev={} old_lines={} new_lines={} left_doc={:?} right_doc={:?}",
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_rev,
                visible_old_lines.len(),
                visible_new_lines.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(patch_target.clone());
            repo.diff_state.diff_rev = 2;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(patch_diff));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit patch rows ready for build-release split parity check",
        |pane| {
            !pane.is_file_diff_view_active()
                && pane.patch_diff_row_len() > 0
                && pane.patch_diff_split_row_len() > 0
                && pane.diff_yaml_block_scalar_for_src_ix.len() == pane.patch_diff_row_len()
                && visible_old_lines.iter().copied().all(|line_no| {
                    patch_split_snapshot_by_line(
                        pane,
                        DiffTextRegion::SplitLeft,
                        theme,
                        yaml_string_color,
                        line_no,
                    )
                    .is_some()
                })
                && visible_new_lines.iter().copied().all(|line_no| {
                    patch_split_snapshot_by_line(
                        pane,
                        DiffTextRegion::SplitRight,
                        theme,
                        yaml_string_color,
                        line_no,
                    )
                    .is_some()
                })
        },
        |pane| {
            format!(
                "diff_view={:?} file_diff_active={} split_rows={} block_scalar_flags={} left_ready={}/{} right_ready={}/{}",
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.patch_diff_split_row_len(),
                pane.diff_yaml_block_scalar_for_src_ix.len(),
                visible_old_lines
                    .iter()
                    .filter(|&&line_no| {
                        patch_split_snapshot_by_line(
                            pane,
                            DiffTextRegion::SplitLeft,
                            theme,
                            yaml_string_color,
                            line_no,
                        )
                        .is_some()
                    })
                    .count(),
                visible_old_lines.len(),
                visible_new_lines
                    .iter()
                    .filter(|&&line_no| {
                        patch_split_snapshot_by_line(
                            pane,
                            DiffTextRegion::SplitRight,
                            theme,
                            yaml_string_color,
                            line_no,
                        )
                        .is_some()
                    })
                    .count(),
                visible_new_lines.len(),
            )
        },
    );

    let split_mismatches = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let mut mismatches = Vec::new();

        for (&line_no, expected) in &baseline_old_by_line {
            let actual = patch_split_snapshot_by_line(
                pane,
                DiffTextRegion::SplitLeft,
                theme,
                yaml_string_color,
                line_no,
            );
            if actual.as_ref() != Some(expected) && mismatches.len() < 16 {
                mismatches.push(("left", line_no, actual, expected.clone()));
            }
        }

        for (&line_no, expected) in &baseline_new_by_line {
            let actual = patch_split_snapshot_by_line(
                pane,
                DiffTextRegion::SplitRight,
                theme,
                yaml_string_color,
                line_no,
            );
            if actual.as_ref() != Some(expected) && mismatches.len() < 16 {
                mismatches.push(("right", line_no, actual, expected.clone()));
            }
        }

        mismatches
    });
    assert!(
        split_mismatches.is_empty(),
        "patch split YAML highlighting should match commit file-diff highlighting for build-release-artifacts.yml: {split_mismatches:?}",
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit patch rows ready for build-release inline parity check",
        |pane| {
            !pane.is_file_diff_view_active()
                && pane.patch_diff_row_len() > 0
                && pane.diff_yaml_block_scalar_for_src_ix.len() == pane.patch_diff_row_len()
        },
        |pane| {
            format!(
                "diff_view={:?} file_diff_active={} rows={} block_scalar_flags={}",
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.patch_diff_row_len(),
                pane.diff_yaml_block_scalar_for_src_ix.len(),
            )
        },
    );

    let inline_mismatches = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let mut mismatches = Vec::new();

        for src_ix in 0..pane.patch_diff_row_len() {
            let Some(line) = pane.patch_diff_row(src_ix) else {
                continue;
            };

            let expected = match line.kind {
                DiffLineKind::Context | DiffLineKind::Remove => line
                    .old_line
                    .and_then(|line_no| baseline_old_by_line.get(&line_no)),
                DiffLineKind::Add => line
                    .new_line
                    .and_then(|line_no| baseline_new_by_line.get(&line_no)),
                DiffLineKind::Header | DiffLineKind::Hunk => None,
            };
            let Some(expected) = expected else {
                continue;
            };

            let actual = Some(yaml_patch_snapshot_for_src_ix(
                pane,
                theme,
                yaml_string_color,
                src_ix,
                diff_content_text(&line),
            ));
            if actual.as_ref() != Some(expected) && mismatches.len() < 16 {
                mismatches.push((
                    line.kind,
                    line.old_line,
                    line.new_line,
                    actual,
                    expected.clone(),
                ));
            }
        }

        mismatches
    });
    assert!(
        inline_mismatches.is_empty(),
        "patch inline YAML highlighting should match commit file-diff highlighting for build-release-artifacts.yml: {inline_mismatches:?}",
    );
}

#[gpui::test]
fn smoke_tests_diff_draw_stabilizes_without_notify_churn(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(46);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_smoke_tests_diff_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");
    let old_text = include_str!("../../../smoke_tests.rs");
    let new_text = format!("{old_text}\n// refresh-loop-regression\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path,
                    Some(old_text.to_string()),
                    Some(new_text),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    let root_notifies = Arc::new(AtomicUsize::new(0));
    let _root_notify_sub = cx.update(|_window, app| {
        let root_notifies = Arc::clone(&root_notifies);
        view.update(app, |_this, cx| {
            cx.observe_self(move |_this, _cx| {
                root_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });

    let main_notifies = Arc::new(AtomicUsize::new(0));
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let _main_notify_sub = cx.update(|_window, app| {
        let main_notifies = Arc::clone(&main_notifies);
        main_pane.update(app, |_pane, cx| {
            cx.observe_self(move |_pane, _cx| {
                main_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "steady smoke_tests.rs diff warmup",
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && left_doc.is_some()
                && right_doc.is_some()
                && left_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && right_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && pane.syntax_chunk_poll_task.is_none()
        },
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            (
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.is_file_diff_view_active(),
                left_doc,
                right_doc,
                left_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                pane.syntax_chunk_poll_task.is_some(),
            )
        },
    );

    root_notifies.store(0, Ordering::Relaxed);
    main_notifies.store(0, Ordering::Relaxed);

    for _ in 0..8 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let root_notify_count = root_notifies.load(Ordering::Relaxed);
    let main_notify_count = main_notifies.load(Ordering::Relaxed);
    assert!(
        root_notify_count <= 1,
        "root view kept notifying during steady smoke_tests.rs diff draws: {root_notify_count}",
    );
    assert!(
        main_notify_count <= 1,
        "main pane kept notifying during steady smoke_tests.rs diff draws: {main_notify_count}",
    );
}

#[gpui::test]
fn file_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_smoke_tests_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");
    let stable_left_line = "    x += 1;";
    let stable_right_line = "    x += 1;";
    let old_text = "fn smoke_test_fixture() {\n    let mut x = 1;\n    x += 1;\n}\n".repeat(64);
    let new_text = format!("{old_text}\n// file-diff-cache-rev-stability\n");

    let set_state = |cx: &mut gpui::VisualTestContext, diff_file_rev: u64| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                repo.diff_state.diff_file_rev = diff_file_rev;
                repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                    gitcomet_core::domain::FileDiffText::new(
                        path.clone(),
                        Some(old_text.clone()),
                        Some(new_text.clone()),
                    ),
                )));

                let next_state = app_state_with_repo(repo, repo_id);

                push_test_state(this, Arc::clone(&next_state), cx);
            });
        });
    };

    set_state(cx, 1);

    wait_for_main_pane_condition(
        cx,
        &view,
        "initial file-diff cache build for rev-stability check",
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && left_doc.is_some()
                && right_doc.is_some()
                && left_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && right_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && pane.syntax_chunk_poll_task.is_none()
        },
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            format!(
                "seq={} inflight={:?} repo_id={:?} rev={} target={:?} path={:?} inline_rows={} left_doc={:?} right_doc={:?} left_pending={:?} right_pending={:?} chunk_poll={} active_diff_rev={:?} active_target={:?} file_diff_active={}",
                pane.file_diff_cache_seq,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target,
                pane.file_diff_cache_path,
                pane.file_diff_inline_cache.len(),
                left_doc,
                right_doc,
                left_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                pane.syntax_chunk_poll_task.is_some(),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.is_file_diff_view_active(),
            )
        },
    );

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_diff_cache_seq);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (left_epoch_before, right_epoch_before, left_hash_before, right_hash_before) =
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, _cx| {
                    let left_row_ix =
                        file_diff_split_row_ix(pane, DiffTextRegion::SplitLeft, stable_left_line)
                            .expect(
                                "expected left split row to exist before seeding the row cache",
                            );
                    let right_row_ix =
                        file_diff_split_row_ix(pane, DiffTextRegion::SplitRight, stable_right_line)
                            .expect(
                                "expected right split row to exist before seeding the row cache",
                            );
                    let left_key = pane
                        .file_diff_split_cache_key(left_row_ix, DiffTextRegion::SplitLeft)
                        .expect("left split row should produce a cache key");
                    let right_key = pane
                        .file_diff_split_cache_key(right_row_ix, DiffTextRegion::SplitRight)
                        .expect("right split row should produce a cache key");
                    let left_epoch =
                        pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
                    let right_epoch =
                        pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
                    let make_seeded =
                        |text: &str, hue: f32, hash: u64| super::CachedDiffStyledText {
                            text: text.to_string().into(),
                            highlights: Arc::from(vec![(
                                0..text.len().min(4),
                                gpui::HighlightStyle {
                                    color: Some(gpui::hsla(hue, 1.0, 0.5, 1.0)),
                                    ..gpui::HighlightStyle::default()
                                },
                            )]),
                            highlights_hash: hash,
                            text_hash: hash.wrapping_mul(31),
                        };
                    pane.diff_text_segments_cache_set(
                        left_key,
                        left_epoch,
                        make_seeded(stable_left_line, 0.0, 0xA11CE),
                    );
                    pane.diff_text_segments_cache_set(
                        right_key,
                        right_epoch,
                        make_seeded(stable_right_line, 0.6, 0xBEEF),
                    );

                    let left_cached = file_diff_split_cached_styled(
                        pane,
                        DiffTextRegion::SplitLeft,
                        stable_left_line,
                    )
                    .expect("seeded left split row should be immediately readable");
                    let right_cached = file_diff_split_cached_styled(
                        pane,
                        DiffTextRegion::SplitRight,
                        stable_right_line,
                    )
                    .expect("seeded right split row should be immediately readable");
                    (
                        left_epoch,
                        right_epoch,
                        left_cached.highlights_hash,
                        right_cached.highlights_hash,
                    )
                })
            })
        });

    for rev in 2..=6 {
        set_state(cx, rev);
        wait_for_main_pane_condition(
            cx,
            &view,
            "identical file-diff payload refresh to settle",
            |pane| {
                let left_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
                let right_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
                pane.file_diff_cache_rev == rev
                    && pane.file_diff_cache_inflight.is_none()
                    && left_doc.is_some()
                    && right_doc.is_some()
                    && left_doc.is_some_and(|document| {
                        !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                    })
                    && right_doc.is_some_and(|document| {
                        !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                    })
                    && pane.syntax_chunk_poll_task.is_none()
            },
            |pane| {
                let left_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
                let right_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
                (
                    pane.file_diff_cache_seq,
                    pane.file_diff_cache_inflight,
                    pane.file_diff_cache_rev,
                    left_doc,
                    right_doc,
                    left_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                    right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                    pane.syntax_chunk_poll_task.is_some(),
                )
            },
        );

        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_diff_cache_seq, baseline_seq,
                "identical diff payload should not trigger file-diff rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_diff_cache_inflight.is_none(),
                "file-diff cache should remain built with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_diff_cache_rev, rev,
                "identical payload refresh should still advance the active file-diff rev marker"
            );
            assert_eq!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                left_epoch_before,
                "identical payload refresh should preserve the left split style epoch"
            );
            assert_eq!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
                right_epoch_before,
                "identical payload refresh should preserve the right split style epoch"
            );
            assert!(
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some(),
                "identical payload refresh should keep the left prepared syntax document reachable"
            );
            assert!(
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
                "identical payload refresh should keep the right prepared syntax document reachable"
            );
            let left_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, stable_left_line)
                    .expect("identical payload refresh should preserve the cached left split row");
            let right_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, stable_right_line)
                    .expect("identical payload refresh should preserve the cached right split row");
            assert_eq!(
                left_cached.highlights_hash, left_hash_before,
                "identical payload refresh should keep the cached left split styling intact"
            );
            assert_eq!(
                right_cached.highlights_hash, right_hash_before,
                "identical payload refresh should keep the cached right split styling intact"
            );
        });
    }
}

#[gpui::test]
fn file_image_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(147);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_image_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/gitcomet.png");
    let image_bytes =
        include_bytes!("../../../../../../assets/linux/hicolor/32x32/apps/gitcomet.png").to_vec();

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(image_bytes.as_slice()),
        Some(image_bytes.as_slice()),
    );
    wait_for_file_image_diff_cache(cx, &view, "initial image diff cache build", |_| true);

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_image_diff_cache_seq);

    for rev in 2..=6 {
        seed_file_image_diff_state_with_rev(
            cx,
            &view,
            repo_id,
            &workdir,
            &path,
            rev,
            Some(image_bytes.as_slice()),
            Some(image_bytes.as_slice()),
        );
        draw_and_drain_test_window(cx);

        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_image_diff_cache_seq, baseline_seq,
                "identical image diff payload should not trigger cache rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_image_diff_cache_inflight.is_none(),
                "image diff cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_image_diff_cache_rev, rev,
                "identical payload refresh should still advance the image diff cache rev marker"
            );
            assert!(
                pane.is_file_image_diff_view_active(),
                "image diff preview should remain active across rev-only refreshes"
            );
        });
    }
}

#[gpui::test]
fn file_image_diff_cache_keeps_valid_svg_on_render_fast_path_across_rev_refreshes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(148);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");
    let svg_bytes = image_diff_svg_fixture(4096, 2048, "#00aaff");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(svg_bytes.as_slice()),
        Some(svg_bytes.as_slice()),
    );
    wait_for_file_image_diff_cache(cx, &view, "initial svg image diff cache build", |pane| {
        pane.file_image_diff_cache_old.is_some()
            && pane.file_image_diff_cache_new.is_some()
            && pane.file_image_diff_cache_old_svg_path.is_none()
            && pane.file_image_diff_cache_new_svg_path.is_none()
    });

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_image_diff_cache_seq);

    for rev in 2..=6 {
        seed_file_image_diff_state_with_rev(
            cx,
            &view,
            repo_id,
            &workdir,
            &path,
            rev,
            Some(svg_bytes.as_slice()),
            Some(svg_bytes.as_slice()),
        );
        draw_and_drain_test_window(cx);

        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_image_diff_cache_seq, baseline_seq,
                "identical svg image diff payload should not trigger cache rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_image_diff_cache_inflight.is_none(),
                "svg image diff cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_image_diff_cache_rev, rev,
                "identical svg payload refresh should still advance the image diff cache rev marker"
            );
            assert!(
                pane.file_image_diff_cache_old.is_some() && pane.file_image_diff_cache_new.is_some(),
                "valid svg payload should stay on the rasterized render-image path"
            );
            assert!(
                pane.file_image_diff_cache_old_svg_path.is_none()
                    && pane.file_image_diff_cache_new_svg_path.is_none(),
                "valid svg payload should not fall back to cached svg file paths"
            );
            assert!(
                pane.is_file_image_diff_view_active(),
                "svg image diff preview should remain active across rev-only refreshes"
            );
        });
    }
}

#[gpui::test]
fn file_image_diff_cache_keeps_distinct_valid_svg_sides_on_render_fast_path(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(149);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_distinct",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");
    let old_svg = image_diff_svg_fixture(4096, 2048, "#00aaff");
    let new_svg = image_diff_svg_fixture(2048, 4096, "#ffaa00");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(old_svg.as_slice()),
        Some(new_svg.as_slice()),
    );
    wait_for_file_image_diff_cache(
        cx,
        &view,
        "distinct svg image diff render cache build",
        |pane| {
            pane.file_image_diff_cache_old.is_some()
                && pane.file_image_diff_cache_new.is_some()
                && pane.file_image_diff_cache_old_svg_path.is_none()
                && pane.file_image_diff_cache_new_svg_path.is_none()
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let old = pane
            .file_image_diff_cache_old
            .as_ref()
            .expect("old render image");
        let new = pane
            .file_image_diff_cache_new
            .as_ref()
            .expect("new render image");
        assert_eq!(old.size(0).width.0, 1024);
        assert_eq!(old.size(0).height.0, 512);
        assert_eq!(new.size(0).width.0, 512);
        assert_eq!(new.size(0).height.0, 1024);
    });
}

#[gpui::test]
fn file_image_diff_cache_falls_back_to_cached_svg_paths_for_invalid_svg_payloads(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(150);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_invalid",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(&b"<not-valid-svg-old>"[..]),
        Some(&b"<not-valid-svg-new>"[..]),
    );
    wait_for_file_image_diff_cache(
        cx,
        &view,
        "invalid svg image diff fallback cache build",
        |pane| {
            pane.file_image_diff_cache_old.is_none()
                && pane.file_image_diff_cache_new.is_none()
                && pane.file_image_diff_cache_old_svg_path.is_some()
                && pane.file_image_diff_cache_new_svg_path.is_some()
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_image_diff_cache_old_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
        assert!(
            pane.file_image_diff_cache_new_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
    });
}

#[gpui::test]
fn file_diff_view_renders_split_and_inline_syntax_from_real_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(49);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_syntax_view",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/file_diff_projection.rs");
    let removed_line = "struct Removed {}";
    let added_line = "fn added() { let value = 2; }";
    let removed_inline_text = format!("-{removed_line}");
    let added_inline_text = format!("+{added_line}");
    let old_text = format!("const KEEP: i32 = 1;\n{removed_line}\nconst AFTER: i32 = 2;\n");
    let new_text = format!("const KEEP: i32 = 1;\nconst AFTER: i32 = 2;\n{added_line}\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} repo_id={:?} cache_rev={} cache_target={:?} cache_path={:?} file_diff_active={} active_repo={:?} active_diff_file_rev={:?} active_diff_target={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target.clone(),
                pane.file_diff_cache_path.clone(),
                pane.is_file_diff_view_active(),
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff split syntax render",
        |pane| {
            let Some(remove_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, removed_line)
            else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_line
                && add_styled.text.as_ref() == added_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 0..6)
                && highlights_include_range(add_styled.highlights.as_ref(), 0..2)
        },
        |pane| {
            let remove_row_ix =
                file_diff_split_row_ix(pane, DiffTextRegion::SplitLeft, removed_line);
            let add_row_ix = file_diff_split_row_ix(pane, DiffTextRegion::SplitRight, added_line);
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_line);
            format!(
                "file_diff_active={} diff_view={:?} visible_len={} cache_path={:?} cache_repo_id={:?} cache_rev={} cache_target={:?} active_repo={:?} active_diff_file_rev={:?} active_diff_target={:?} remove_row_ix={remove_row_ix:?} add_row_ix={add_row_ix:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.diff_visible_len(),
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target.clone(),
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff inline syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_line
                && add_styled.text.as_ref() == added_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 0..6)
                && highlights_include_range(add_styled.highlights.as_ref(), 0..2)
        },
        |pane| {
            let remove_inline_ix = file_diff_inline_ix(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_inline_ix = file_diff_inline_ix(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            format!(
                "file_diff_active={} diff_view={:?} visible_len={} remove_inline_ix={remove_inline_ix:?} add_inline_ix={add_inline_ix:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.diff_visible_len(),
            )
        },
    );
}

#[gpui::test]
fn html_file_diff_renders_injected_attribute_syntax_from_real_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(77);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_html_attribute_injections",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/file_diff_attribute_injections.html");
    let removed_onclick_line = r#"<button onclick="const value = 1;">go</button>"#;
    let added_onclick_line = r#"<button onclick="const value = 2;">go</button>"#;
    let added_style_line = r#"<div style="color: red; display: block">ok</div>"#;
    let removed_inline_text = format!("-{removed_onclick_line}");
    let added_inline_text = format!("+{added_onclick_line}");
    let style_inline_text = format!("+{added_style_line}");
    let old_text = format!("<p>keep</p>\n{removed_onclick_line}\n<p>after</p>\n");
    let new_text = format!("<p>keep</p>\n<p>after</p>\n{added_onclick_line}\n{added_style_line}\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_onclick_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_onclick_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_style_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == style_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff split attribute injection syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_split_cached_styled(
                pane,
                DiffTextRegion::SplitLeft,
                removed_onclick_line,
            ) else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_onclick_line)
            else {
                return false;
            };
            let Some(style_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_style_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_onclick_line
                && add_styled.text.as_ref() == added_onclick_line
                && style_styled.text.as_ref() == added_style_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(add_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(add_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(style_styled.highlights.as_ref(), 12..17)
                && highlights_include_range(style_styled.highlights.as_ref(), 24..31)
        },
        |pane| {
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_onclick_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_onclick_line);
            let style_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_style_line);
            format!(
                "diff_view={:?} remove_cached={remove_cached:?} add_cached={add_cached:?} style_cached={style_cached:?}",
                pane.diff_view,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff inline attribute injection syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };
            let Some(style_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &style_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_onclick_line
                && add_styled.text.as_ref() == added_onclick_line
                && style_styled.text.as_ref() == added_style_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(add_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(add_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(style_styled.highlights.as_ref(), 12..17)
                && highlights_include_range(style_styled.highlights.as_ref(), 24..31)
        },
        |pane| {
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            let style_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &style_inline_text,
            );
            format!(
                "diff_view={:?} remove_cached={remove_cached:?} add_cached={add_cached:?} style_cached={style_cached:?}",
                pane.diff_view,
            )
        },
    );
}

#[gpui::test]
fn xml_file_diff_renders_syntax_highlights_from_real_documents(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(79);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_xml_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("config/settings.xml");
    let removed_tag_line = r#"<server port="8080">"#;
    let added_tag_line = r#"<server port="9090" mode="prod">"#;
    let comment_line = "<!-- configuration -->";
    let removed_inline_text = format!("-{removed_tag_line}");
    let added_inline_text = format!("+{added_tag_line}");
    let old_text = format!("{comment_line}\n{removed_tag_line}\n  <name>app</name>\n</server>\n");
    let new_text = format!("{comment_line}\n{added_tag_line}\n  <name>app</name>\n</server>\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Xml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_tag_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_tag_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} language={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff split syntax render",
        |pane| {
            let Some(remove_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, removed_tag_line)
            else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_tag_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_tag_line
                && add_styled.text.as_ref() == added_tag_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(add_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 20..24)
        },
        |pane| {
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_tag_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_tag_line);
            format!(
                "diff_view={:?} language={:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.diff_view, pane.file_diff_cache_language,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff inline syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_tag_line
                && add_styled.text.as_ref() == added_tag_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(add_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 20..24)
        },
        |pane| {
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            format!(
                "diff_view={:?} language={:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.diff_view, pane.file_diff_cache_language,
            )
        },
    );
}

#[gpui::test]
fn yaml_file_diff_keeps_consistent_highlighting_for_added_paths_and_keys(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn force_file_diff_fallback_mode(pane: &mut MainPaneView) {
        pane.file_diff_syntax_generation = pane.file_diff_syntax_generation.wrapping_add(1);
        for view_mode in [
            PreparedSyntaxViewMode::FileDiffSplitLeft,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        ] {
            if let Some(key) = pane.file_diff_prepared_syntax_key(view_mode) {
                pane.prepared_syntax_documents.remove(&key);
            }
        }
        pane.clear_diff_text_style_caches();
    }

    fn quoted_scalar_style(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<(std::ops::Range<usize>, gpui::Hsla)> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start == quote_start
                && range.end == text.len())
            .then_some((range.clone(), color))
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start <= dash_ix
                && range.end >= dash_ix.saturating_add(1))
            .then_some(color)
        })
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn line_debug(
        line: Option<(&str, &super::CachedDiffStyledText)>,
    ) -> Option<(
        String,
        Vec<(
            std::ops::Range<usize>,
            Option<gpui::Hsla>,
            Option<gpui::Hsla>,
        )>,
    )> {
        let (text, styled) = line?;
        Some((
            text.to_string(),
            styled
                .highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
        ))
    }

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(split_right_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(inline_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn split_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<FileDiffRowKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    split_right_row_by_new_line(pane, line).map(|row| row.kind),
                )
            })
            .collect()
    }

    fn inline_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<DiffLineKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| (line, inline_row_by_new_line(pane, line).map(|row| row.kind)))
            .collect()
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    #[derive(Clone, Copy, Debug)]
    struct ExpectedPaintRow {
        line_no: u32,
        visible_ix: usize,
        expects_add_bg: bool,
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn split_draw_rows_for_lines(pane: &MainPaneView, lines: &[u32]) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = split_visible_ix_by_new_line(pane, line_no)
                    .unwrap_or_else(|| panic!("expected split visible row for line {line_no}"));
                let expects_add_bg = split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add);
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg,
                }
            })
            .collect()
    }

    fn inline_draw_rows_for_lines(pane: &MainPaneView, lines: &[u32]) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = inline_visible_ix_by_new_line(pane, line_no)
                    .unwrap_or_else(|| panic!("expected inline visible row for line {line_no}"));
                let expects_add_bg = inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add);
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg,
                }
            })
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    fn assert_split_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::SplitRight,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let (text, styled) = split_right_cached_styled_by_new_line(pane, expected.line_no)
                    .unwrap_or_else(|| {
                        panic!(
                            "expected cached split-right styled text for line {}",
                            expected.line_no
                        )
                    });
                (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref()),
                )
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    fn assert_inline_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::Inline,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let (text, styled) = inline_cached_styled_by_new_line(pane, expected.line_no)
                    .unwrap_or_else(|| {
                        panic!(
                            "expected cached inline styled text for line {}",
                            expected.line_no
                        )
                    });
                (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref()),
                )
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(80);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let repo_root = fixture_repo_root();
    let git_show = |spec: &str| fixture_git_show(&repo_root, spec, "YAML diff regression fixture");
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");

    let baseline_path_line = 17u32;
    let affected_path_lines = [18u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_nested_key_line = 4u32;
    let affected_nested_key_lines = [19u32, 34u32];
    let baseline_top_key_line = 3u32;
    let affected_top_key_lines = [36u32];
    let affected_add_lines = [18u32, 33u32];
    let affected_context_lines = [19u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 34, 36];
    let render_lines = [
        17u32, 18, 19, 21, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36,
    ];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 0, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff cache build before fallback highlighting checks",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // Other YAML tests can warm the shared prepared-syntax cache before this
                // test runs. Clear the local prepared documents and invalidate any in-flight
                // background parse so the next draw deterministically exercises fallback mode.
                force_file_diff_fallback_mode(pane);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff fallback mode forced for highlight checks",
        |pane| {
            pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let (baseline_path_text, baseline_path_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_path_line)
                .expect("fallback split draw should cache the baseline YAML path row");
        let baseline_dash_color = list_item_dash_color(baseline_path_styled, baseline_path_text)
            .expect("fallback split draw should syntax-highlight the YAML list dash");
        let (_, baseline_path_color) = quoted_scalar_style(baseline_path_styled, baseline_path_text)
            .expect("fallback split draw should syntax-highlight the YAML quoted path");
        for line_no in affected_path_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML row {line_no}"));
            assert_eq!(
                list_item_dash_color(styled, text),
                Some(baseline_dash_color),
                "fallback split draw should keep YAML list punctuation highlighting on line {line_no}",
            );
            assert_eq!(
                quoted_scalar_style(styled, text).map(|(_, color)| color),
                Some(baseline_path_color),
                "fallback split draw should keep YAML quoted-string highlighting on line {line_no}",
            );
        }

        let (baseline_nested_key_text, baseline_nested_key_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_nested_key_line)
                .expect("fallback split draw should cache the baseline YAML nested key row");
        let baseline_nested_key_color = mapping_key_color(
            baseline_nested_key_styled,
            baseline_nested_key_text,
        )
        .expect("fallback split draw should syntax-highlight the YAML nested key");
        for line_no in affected_nested_key_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_nested_key_color),
                "fallback split draw should keep YAML key highlighting on line {line_no}",
            );
        }

        let (baseline_top_key_text, baseline_top_key_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_top_key_line)
                .expect("fallback split draw should cache the baseline YAML top-level key row");
        let baseline_top_key_color =
            mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
                .expect("fallback split draw should syntax-highlight the YAML top-level key");
        for line_no in affected_top_key_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML top-level key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_top_key_color),
                "fallback split draw should keep YAML top-level key highlighting on line {line_no}",
            );
        }
    });

    let fallback_split_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, &render_lines)
    });
    assert_split_rows_match_render_cache(cx, &view, "fallback split", fallback_split_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let (baseline_path_text, baseline_path_styled) =
            inline_cached_styled_by_new_line(pane, baseline_path_line)
                .expect("fallback inline draw should cache the baseline YAML path row");
        let baseline_dash_color = list_item_dash_color(baseline_path_styled, baseline_path_text)
            .expect("fallback inline draw should syntax-highlight the YAML list dash");
        let (_, baseline_path_color) = quoted_scalar_style(baseline_path_styled, baseline_path_text)
            .expect("fallback inline draw should syntax-highlight the YAML quoted path");
        for line_no in affected_path_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML row {line_no}"));
            assert_eq!(
                list_item_dash_color(styled, text),
                Some(baseline_dash_color),
                "fallback inline draw should keep YAML list punctuation highlighting on line {line_no}",
            );
            assert_eq!(
                quoted_scalar_style(styled, text).map(|(_, color)| color),
                Some(baseline_path_color),
                "fallback inline draw should keep YAML quoted-string highlighting on line {line_no}",
            );
        }

        let (baseline_nested_key_text, baseline_nested_key_styled) =
            inline_cached_styled_by_new_line(pane, baseline_nested_key_line)
                .expect("fallback inline draw should cache the baseline YAML nested key row");
        let baseline_nested_key_color = mapping_key_color(
            baseline_nested_key_styled,
            baseline_nested_key_text,
        )
        .expect("fallback inline draw should syntax-highlight the YAML nested key");
        for line_no in affected_nested_key_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_nested_key_color),
                "fallback inline draw should keep YAML key highlighting on line {line_no}",
            );
        }

        let (baseline_top_key_text, baseline_top_key_styled) =
            inline_cached_styled_by_new_line(pane, baseline_top_key_line)
                .expect("fallback inline draw should cache the baseline YAML top-level key row");
        let baseline_top_key_color =
            mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
                .expect("fallback inline draw should syntax-highlight the YAML top-level key");
        for line_no in affected_top_key_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML top-level key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_top_key_color),
                "fallback inline draw should keep YAML top-level key highlighting on line {line_no}",
            );
        }
    });

    let fallback_inline_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, &render_lines)
    });
    assert_inline_rows_match_render_cache(cx, &view, "fallback inline", fallback_inline_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &old_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff baseline revision prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 2, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff split syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some((_, baseline_path_color)) =
                quoted_scalar_style(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            if affected_add_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_style(styled, text).map(|(_, color)| color)
                        != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} split_kinds={:?} split_debug={:?}",
                pane.diff_view,
                split_kind_debug(pane, &lines),
                split_debug(pane, &lines),
            )
        },
    );

    let prepared_split_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, &render_lines)
    });
    assert_split_rows_match_render_cache(cx, &view, "prepared split", prepared_split_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff inline syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some((_, baseline_path_color)) =
                quoted_scalar_style(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            if affected_add_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_style(styled, text).map(|(_, color)| color)
                        != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} inline_kinds={:?} inline_debug={:?}",
                pane.diff_view,
                inline_kind_debug(pane, &lines),
                inline_debug(pane, &lines),
            )
        },
    );

    let prepared_inline_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, &render_lines)
    });
    assert_inline_rows_match_render_cache(cx, &view, "prepared inline", prepared_inline_draw_rows);
}

#[gpui::test]
fn yaml_file_diff_fallback_matches_prepared_document_for_deployment_ci(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn one_based_line_byte_range(
        text: &str,
        line_starts: &[usize],
        line_no: u32,
    ) -> Option<std::ops::Range<usize>> {
        let line_ix = usize::try_from(line_no).ok()?.checked_sub(1)?;
        let start = (*line_starts.get(line_ix)?).min(text.len());
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        Some(start..end)
    }

    fn shared_text_and_line_starts(text: &str) -> (gpui::SharedString, Arc<[usize]>) {
        let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
        line_starts.push(0usize);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix.saturating_add(1));
            }
        }
        (text.to_string().into(), Arc::from(line_starts))
    }

    fn prepared_document_snapshot_for_line(
        theme: AppTheme,
        text: &str,
        line_starts: &[usize],
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        let byte_range = one_based_line_byte_range(text, line_starts, line_no)?;
        let line_text = text.get(byte_range.clone())?.to_string();
        let started = std::time::Instant::now();

        loop {
            let highlights = rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                text,
                line_starts,
                document,
                language,
                byte_range.clone(),
            )?;

            if !highlights.pending {
                return Some(LineSyntaxSnapshot {
                    text: line_text.clone(),
                    syntax: highlights
                        .highlights
                        .into_iter()
                        .filter(|(_, style)| style.background_color.is_none())
                        .map(|(range, style)| {
                            (
                                range.start.saturating_sub(byte_range.start)
                                    ..range.end.saturating_sub(byte_range.start),
                                style.color,
                            )
                        })
                        .collect(),
                });
            }

            let completed =
                rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if completed == 0 && started.elapsed() >= std::time::Duration::from_secs(2) {
                return None;
            }
            if completed == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(180);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_fallback_prepared_baseline",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "YAML fallback prepared baseline fixture");
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");
    let (old_shared_text, old_line_starts) = shared_text_and_line_starts(old_text.as_str());
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let old_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        old_shared_text,
        Arc::clone(&old_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared old YAML baseline document, got {other:?}"),
    };
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared new YAML baseline document, got {other:?}"),
    };

    let old_lines = [3u32, 4];
    let new_lines = [
        3u32, 4, 17, 18, 19, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36,
    ];
    let baseline_old_by_line = old_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                old_text.as_str(),
                old_line_starts.as_ref(),
                old_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for old line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let baseline_new_by_line = new_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for new line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "deployment-ci YAML rows ready for prepared-baseline comparison",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in new_lines {
            let actual = split_right_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected fallback split-right styled text for deployment-ci line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for deployment-ci line {line_no}"));
            assert_eq!(
                actual, expected,
                "fallback split-right YAML highlighting should match prepared baseline for deployment-ci line {line_no}"
            );
        }
    });

    let split_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                split_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected split visible row for deployment-ci line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    for (&line_no, &visible_ix) in new_lines.iter().zip(split_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!(
                        "expected cached split-right styled text for deployment-ci line {line_no}"
                    )
                });
            let kind = split_right_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-right row for deployment-ci line {line_no}")
                })
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "deployment-ci split render text should match cache for line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "deployment-ci split render highlights should match cache for line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify),
            "deployment-ci split render should preserve diff background for line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                inline_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected inline visible row for deployment-ci line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in new_lines {
            let actual = inline_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected fallback inline styled text for deployment-ci line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for deployment-ci line {line_no}"));
            assert_eq!(
                actual, expected,
                "fallback inline YAML highlighting should match prepared baseline for deployment-ci line {line_no}"
            );
        }
    });

    for (&line_no, &visible_ix) in new_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) =
                inline_cached_styled_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected cached inline styled text for deployment-ci line {line_no}")
                });
            let kind = inline_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("expected inline row for deployment-ci line {line_no}"))
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "deployment-ci inline render text should match cache for line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "deployment-ci inline render highlights should match cache for line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, DiffLineKind::Add | DiffLineKind::Remove),
            "deployment-ci inline render should preserve diff background for line {line_no}"
        );
    }

    assert_eq!(
        baseline_old_by_line.len(),
        old_lines.len(),
        "old-side YAML baselines should be materialized for the deployment-ci fixture"
    );
}

#[gpui::test]
fn yaml_file_diff_keeps_consistent_highlighting_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_left_row_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.old_line == Some(old_line))
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_left_cached_styled_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.old_line == Some(old_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.old.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitLeft)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn scalar_color_after_colon(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let value_start = text.find(':')?.checked_add(1).and_then(|start| {
            text[start..]
                .find(|ch: char| !ch.is_ascii_whitespace())
                .map(|offset| start.saturating_add(offset))
        })?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start <= value_start
                && range.end > value_start)
                .then_some(color)
        })
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn expected_yaml_snapshot(
        theme: AppTheme,
        text: &str,
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlight_snapshot(
            rows::syntax_highlights_for_line(
                theme,
                text,
                rows::DiffSyntaxLanguage::Yaml,
                rows::DiffSyntaxMode::Auto,
            )
            .as_slice(),
        )
    }

    fn line_debug(
        line: Option<(&str, &super::CachedDiffStyledText)>,
    ) -> Option<(
        String,
        Vec<(
            std::ops::Range<usize>,
            Option<gpui::Hsla>,
            Option<gpui::Hsla>,
        )>,
    )> {
        let (text, styled) = line?;
        Some((
            text.to_string(),
            styled
                .highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
        ))
    }

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(split_right_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(inline_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn split_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<FileDiffRowKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    split_right_row_by_new_line(pane, line).map(|row| row.kind),
                )
            })
            .collect()
    }

    fn inline_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<DiffLineKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| (line, inline_row_by_new_line(pane, line).map(|row| row.kind)))
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(84);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_build_release_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show = |spec: &str| {
        fixture_git_show(
            &repo_root,
            spec,
            "build-release YAML file-diff regression fixture",
        )
    };
    let old_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    );
    let new_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    );

    let baseline_secret_key_line = 20u32;
    let affected_secret_key_lines = [22u32, 24, 26, 28, 30, 32];
    let baseline_required_line = 21u32;
    let affected_required_lines = [23u32, 25, 27, 29, 31, 33];
    let add_lines = [20u32, 21u32];
    let context_lines = [22u32, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];
    let old_baseline_secret_key_line = 20u32;
    let old_affected_secret_key_lines = [22u32, 24, 26, 28, 30];
    let old_baseline_required_line = 21u32;
    let old_affected_required_lines = [23u32, 25, 27, 29, 31];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 0, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(33))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(33))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff split syntax keeps repeated secret keys and booleans consistent",
        |pane| {
            let Some((baseline_secret_key_text, baseline_secret_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_secret_key_line)
            else {
                return false;
            };
            let Some(baseline_secret_key_color) =
                mapping_key_color(baseline_secret_key_styled, baseline_secret_key_text)
            else {
                return false;
            };
            if add_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add)
            }) {
                return false;
            }
            if context_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Context)
            }) {
                return false;
            }
            if affected_secret_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_secret_key_color)
            }) {
                return false;
            }

            let Some((baseline_required_text, baseline_required_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_required_line)
            else {
                return false;
            };
            let Some(baseline_required_color) =
                scalar_color_after_colon(baseline_required_styled, baseline_required_text)
            else {
                return false;
            };
            !affected_required_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                scalar_color_after_colon(styled, text) != Some(baseline_required_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_secret_key_line);
            lines.extend(affected_secret_key_lines);
            lines.push(baseline_required_line);
            lines.extend(affected_required_lines);
            format!(
                "diff_view={:?} split_kinds={:?} split_debug={:?}",
                pane.diff_view,
                split_kind_debug(pane, &lines),
                split_debug(pane, &lines),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let mut old_lines = Vec::new();
        old_lines.push(old_baseline_secret_key_line);
        old_lines.extend(old_affected_secret_key_lines);
        old_lines.push(old_baseline_required_line);
        old_lines.extend(old_affected_required_lines);

        for old_line in old_lines {
            let Some(row) = split_left_row_by_old_line(pane, old_line) else {
                panic!("expected split-left row for old line {old_line}");
            };
            assert_eq!(
                row.kind,
                FileDiffRowKind::Context,
                "expected build-release old line {old_line} to remain a context row on the left side"
            );
            let Some((text, styled)) = split_left_cached_styled_by_old_line(pane, old_line) else {
                panic!("expected cached split-left styled text for old line {old_line}");
            };
            let expected = expected_yaml_snapshot(theme, text);
            let actual = highlight_snapshot(styled.highlights.as_ref());
            assert_eq!(
                actual, expected,
                "split-left YAML highlighting should match direct single-line YAML highlights for build-release old line {old_line}: text={text:?}"
            );
        }
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff inline syntax keeps repeated secret keys and booleans consistent",
        |pane| {
            let Some((baseline_secret_key_text, baseline_secret_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_secret_key_line)
            else {
                return false;
            };
            let Some(baseline_secret_key_color) =
                mapping_key_color(baseline_secret_key_styled, baseline_secret_key_text)
            else {
                return false;
            };
            if add_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add)
            }) {
                return false;
            }
            if context_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Context)
            }) {
                return false;
            }
            if affected_secret_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_secret_key_color)
            }) {
                return false;
            }

            let Some((baseline_required_text, baseline_required_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_required_line)
            else {
                return false;
            };
            let Some(baseline_required_color) =
                scalar_color_after_colon(baseline_required_styled, baseline_required_text)
            else {
                return false;
            };
            !affected_required_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                scalar_color_after_colon(styled, text) != Some(baseline_required_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_secret_key_line);
            lines.extend(affected_secret_key_lines);
            lines.push(baseline_required_line);
            lines.extend(affected_required_lines);
            format!(
                "diff_view={:?} inline_kinds={:?} inline_debug={:?}",
                pane.diff_view,
                inline_kind_debug(pane, &lines),
                inline_debug(pane, &lines),
            )
        },
    );
}

#[gpui::test]
fn yaml_file_diff_matches_prepared_document_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_left_cached_styled_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.old_line == Some(old_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.old.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitLeft)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_left_row_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.old_line == Some(old_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn one_based_line_byte_range(
        text: &str,
        line_starts: &[usize],
        line_no: u32,
    ) -> Option<std::ops::Range<usize>> {
        let line_ix = usize::try_from(line_no).ok()?.checked_sub(1)?;
        let start = (*line_starts.get(line_ix)?).min(text.len());
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        Some(start..end)
    }

    fn shared_text_and_line_starts(text: &str) -> (gpui::SharedString, Arc<[usize]>) {
        let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
        line_starts.push(0usize);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix.saturating_add(1));
            }
        }
        (text.to_string().into(), Arc::from(line_starts))
    }

    fn prepared_document_snapshot_for_line(
        theme: AppTheme,
        text: &str,
        line_starts: &[usize],
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        let byte_range = one_based_line_byte_range(text, line_starts, line_no)?;
        let line_text = text.get(byte_range.clone())?.to_string();
        let started = std::time::Instant::now();

        loop {
            let highlights = rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                text,
                line_starts,
                document,
                language,
                byte_range.clone(),
            )?;

            if !highlights.pending {
                return Some(LineSyntaxSnapshot {
                    text: line_text.clone(),
                    syntax: highlights
                        .highlights
                        .into_iter()
                        .filter(|(_, style)| style.background_color.is_none())
                        .map(|(range, style)| {
                            (
                                range.start.saturating_sub(byte_range.start)
                                    ..range.end.saturating_sub(byte_range.start),
                                style.color,
                            )
                        })
                        .collect(),
                });
            }

            let completed =
                rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if completed == 0 && started.elapsed() >= std::time::Duration::from_secs(2) {
                return None;
            }
            if completed == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(184);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_build_release_prepared_baseline",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "build-release prepared-baseline fixture");
    let old_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    );
    let new_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    );
    let (old_shared_text, old_line_starts) = shared_text_and_line_starts(old_text.as_str());
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let old_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        old_shared_text,
        Arc::clone(&old_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared old YAML baseline document, got {other:?}"),
    };
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared new YAML baseline document, got {other:?}"),
    };

    let old_lines = [20u32, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31];
    let new_lines = [20u32, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_old_by_line = old_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                old_text.as_str(),
                old_line_starts.as_ref(),
                old_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for old line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let baseline_new_by_line = new_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for new line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML rows ready for prepared-baseline comparison",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(33))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(33))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in old_lines {
            let actual = split_left_cached_styled_by_old_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected split-left styled text for build-release old line {line_no}")
                });
            let expected = baseline_old_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release old line {line_no}"));
            assert_eq!(
                actual, expected,
                "split-left YAML highlighting should match prepared baseline for build-release old line {line_no}"
            );
        }

        for line_no in new_lines {
            let actual = split_right_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected split-right styled text for build-release new line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release new line {line_no}"));
            assert_eq!(
                actual, expected,
                "split-right YAML highlighting should match prepared baseline for build-release new line {line_no}"
            );
        }
    });

    let split_left_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        old_lines
            .iter()
            .copied()
            .map(|line_no| {
                (0..pane.diff_visible_len())
                    .find(|&visible_ix| {
                        let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                            return false;
                        };
                        pane.file_diff_split_row(row_ix)
                            .is_some_and(|row| row.old_line == Some(line_no))
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "expected split-left visible row for build-release old line {line_no}"
                        )
                    })
            })
            .collect::<Vec<_>>()
    });
    let split_right_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                split_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected split-right visible row for build-release new line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_left_visible_indices.as_slice());
    draw_rows_for_visible_indices(cx, &view, split_right_visible_indices.as_slice());

    for (&line_no, &visible_ix) in old_lines.iter().zip(split_left_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitLeft);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) = split_left_cached_styled_by_old_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected cached split-left styled text for build-release old line {line_no}")
                });
            let kind = split_left_row_by_old_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-left row for build-release old line {line_no}")
                })
                .kind;
            (text.to_string(), highlight_snapshot(styled.highlights.as_ref()), kind)
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release split-left render text should match cache for old line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release split-left render highlights should match cache for old line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Remove | FileDiffRowKind::Modify),
            "build-release split-left render should preserve diff background for old line {line_no}"
        );
    }

    for (&line_no, &visible_ix) in new_lines.iter().zip(split_right_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected cached split-right styled text for build-release new line {line_no}")
                });
            let kind = split_right_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-right row for build-release new line {line_no}")
                })
                .kind;
            (text.to_string(), highlight_snapshot(styled.highlights.as_ref()), kind)
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release split-right render text should match cache for new line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release split-right render highlights should match cache for new line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify),
            "build-release split-right render should preserve diff background for new line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                inline_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected inline visible row for build-release new line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in new_lines {
            let actual = inline_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected inline styled text for build-release new line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release new line {line_no}"));
            assert_eq!(
                actual, expected,
                "inline YAML highlighting should match prepared baseline for build-release new line {line_no}"
            );
        }
    });

    for (&line_no, &visible_ix) in new_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) =
                inline_cached_styled_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!(
                        "expected cached inline styled text for build-release new line {line_no}"
                    )
                });
            let kind = inline_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected inline row for build-release new line {line_no}")
                })
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release inline render text should match cache for new line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release inline render highlights should match cache for new line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, DiffLineKind::Add | DiffLineKind::Remove),
            "build-release inline render should preserve diff background for new line {line_no}"
        );
    }
}

#[gpui::test]
fn yaml_commit_file_diff_transition_from_patch_clears_stale_split_cache(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffTarget;

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn expected_yaml_snapshot(
        theme: AppTheme,
        text: &str,
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlight_snapshot(
            rows::syntax_highlights_for_line(
                theme,
                text,
                rows::DiffSyntaxLanguage::Yaml,
                rows::DiffSyntaxMode::Auto,
            )
            .as_slice(),
        )
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);
    let repo_id = gitcomet_state::model::RepoId(85);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_to_file_transition",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let patch_text =
        std::fs::read_to_string(fixture_repo_root().join("test_data/commit-bd8b4a04.patch"))
            .expect("read patch fixture");
    let patch_target = DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let patch_diff = gitcomet_core::domain::Diff::from_unified(patch_target.clone(), &patch_text);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(patch_target);
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(patch_diff));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "patch diff split cache seeded before switching to file diff",
        |pane| {
            !pane.is_file_diff_view_active()
                && pane.patch_diff_split_row_len() > 0
                && !pane.diff_text_segments_cache.is_empty()
        },
        |pane| {
            format!(
                "file_diff_active={} diff_view={:?} patch_rows={} split_rows={} text_cache_len={}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.patch_diff_row_len(),
                pane.patch_diff_split_row_len(),
                pane.diff_text_segments_cache.len(),
            )
        },
    );

    let repo_root = fixture_repo_root();
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "patch->file YAML transition fixture");
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");
    let unified = fixture_git_diff(
        &repo_root,
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml",
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml",
        "patch->file YAML transition fixture",
    );
    let file_target = DiffTarget::Commit {
        commit_id,
        path: Some(path.clone()),
    };
    let file_diff = gitcomet_core::domain::Diff::from_unified(file_target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(file_target.clone());
            repo.diff_state.diff_rev = 2;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(file_diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.clone()),
                    Some(new_text.clone()),
                ),
            )));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "patch -> file diff transition yields fresh deployment-ci split highlights",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(file_target.clone())
                && split_right_cached_styled_by_new_line(pane, 17).is_some()
                && split_right_cached_styled_by_new_line(pane, 18).is_some()
                && split_right_cached_styled_by_new_line(pane, 33).is_some()
        },
        |pane| {
            format!(
                "file_diff_active={} inflight={:?} cache_target={:?} active_target={:?} cache_len={} split17={:?} split18={:?} split33={:?}",
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target.clone(),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.diff_text_segments_cache.len(),
                split_right_cached_styled_by_new_line(pane, 17).map(|(text, styled)| (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref())
                )),
                split_right_cached_styled_by_new_line(pane, 18).map(|(text, styled)| (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref())
                )),
                split_right_cached_styled_by_new_line(pane, 33).map(|(text, styled)| (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref())
                )),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for new_line in [17u32, 18, 22, 33] {
            let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, new_line) else {
                panic!("expected cached split-right styled text for deployment-ci new line {new_line}");
            };
            let expected = expected_yaml_snapshot(theme, text);
            let actual = highlight_snapshot(styled.highlights.as_ref());
            assert_eq!(
                actual, expected,
                "patch->file transition should not reuse stale split-right styling for deployment-ci new line {new_line}: text={text:?}"
            );
        }
    });
}

#[allow(dead_code)]
fn yaml_same_content_rev_refresh_invalidates_cached_heuristic_file_diff_rows(
    cx: &mut gpui::TestAppContext,
) {
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn one_based_line_byte_range(
        text: &str,
        line_starts: &[usize],
        line_no: u32,
    ) -> Option<std::ops::Range<usize>> {
        let line_ix = usize::try_from(line_no).ok()?.checked_sub(1)?;
        let start = (*line_starts.get(line_ix)?).min(text.len());
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        Some(start..end)
    }

    fn shared_text_and_line_starts(text: &str) -> (gpui::SharedString, Arc<[usize]>) {
        let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
        line_starts.push(0usize);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix.saturating_add(1));
            }
        }
        (text.to_string().into(), Arc::from(line_starts))
    }

    fn prepared_document_snapshot_for_line(
        theme: AppTheme,
        text: &str,
        line_starts: &[usize],
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        let byte_range = one_based_line_byte_range(text, line_starts, line_no)?;
        let line_text = text.get(byte_range.clone())?.to_string();
        let started = std::time::Instant::now();

        loop {
            let highlights = rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                text,
                line_starts,
                document,
                language,
                byte_range.clone(),
            )?;

            if !highlights.pending {
                return Some(LineSyntaxSnapshot {
                    text: line_text.clone(),
                    syntax: highlights
                        .highlights
                        .into_iter()
                        .filter(|(_, style)| style.background_color.is_none())
                        .map(|(range, style)| {
                            (
                                range.start.saturating_sub(byte_range.start)
                                    ..range.end.saturating_sub(byte_range.start),
                                style.color,
                            )
                        })
                        .collect(),
                });
            }

            let completed =
                rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if completed == 0 && started.elapsed() >= std::time::Duration::from_secs(2) {
                return None;
            }
            if completed == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn paint_snapshot(record: &rows::DiffPaintRecord) -> LineSyntaxSnapshot {
        LineSyntaxSnapshot {
            text: record.text.to_string(),
            syntax: record
                .highlights
                .iter()
                .filter(|(_, _, bg)| bg.is_none())
                .map(|(range, color, _)| (range.clone(), *color))
                .collect(),
        }
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    fn split_mismatch_lines(
        pane: &MainPaneView,
        baselines: &BTreeMap<u32, LineSyntaxSnapshot>,
        lines: &[u32],
    ) -> Vec<u32> {
        lines
            .iter()
            .copied()
            .filter(|line| {
                let Some(actual) =
                    split_right_cached_styled_by_new_line(pane, *line).map(cached_snapshot)
                else {
                    return false;
                };
                baselines
                    .get(line)
                    .is_some_and(|expected| actual != *expected)
            })
            .collect()
    }

    fn inline_mismatch_lines(
        pane: &MainPaneView,
        baselines: &BTreeMap<u32, LineSyntaxSnapshot>,
        lines: &[u32],
    ) -> Vec<u32> {
        lines
            .iter()
            .copied()
            .filter(|line| {
                let Some(actual) =
                    inline_cached_styled_by_new_line(pane, *line).map(cached_snapshot)
                else {
                    return false;
                };
                baselines
                    .get(line)
                    .is_some_and(|expected| actual != *expected)
            })
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);
    let repo_id = gitcomet_state::model::RepoId(87);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_same_content_rev_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show = |spec: &str| {
        fixture_git_show(
            &repo_root,
            spec,
            "same-content YAML refresh regression fixture",
        )
    };
    fn append_yaml_padding(text: &str) -> String {
        use std::fmt::Write as _;

        const PADDING_LINES: usize = 65_536;
        let mut out = String::with_capacity(text.len().saturating_add(PADDING_LINES * 64));
        out.push_str(text);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for ix in 0..PADDING_LINES {
            let _ = writeln!(
                out,
                "# syntax-padding-{ix:05}-abcdefghijklmnopqrstuvwxyz0123456789"
            );
        }
        out
    }

    let old_text = append_yaml_padding(&git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    ));
    let new_text = append_yaml_padding(&git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    ));
    let affected_lines = [173u32, 175, 176, 183, 190, 193, 206, 212, 218, 221];
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(5),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!(
            "expected prepared YAML baseline document for same-content refresh, got {other:?}"
        ),
    };
    let baseline_new_by_line = affected_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| {
                panic!("expected prepared YAML baseline for build-release line {line_no}")
            });
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release file-diff rows ready before same-content refresh",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && affected_lines
                    .iter()
                    .copied()
                    .all(|line| split_visible_ix_by_new_line(pane, line).is_some())
        },
        |pane| {
            let split_mismatches =
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines);
            let first_mismatch = split_mismatches.first().copied();
            let cache_row_ix = first_mismatch.and_then(|line_no| {
                pane.file_diff_cache_rows
                    .iter()
                    .position(|row| row.new_line == Some(line_no))
            });
            let provider_row_ix = first_mismatch.and_then(|line_no| {
                (0..pane.file_diff_split_row_len()).find(|&row_ix| {
                    pane.file_diff_split_row(row_ix)
                        .is_some_and(|row| row.new_line == Some(line_no))
                })
            });
            let actual = first_mismatch.and_then(|line_no| {
                split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
            });
            let cached_text = cache_row_ix.and_then(|row_ix| {
                let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
                let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
                pane.diff_text_segments_cache_get(key, epoch)
                    .map(|styled| styled.text.to_string())
            });
            let expected =
                first_mismatch.and_then(|line_no| baseline_new_by_line.get(&line_no).cloned());
            let doc_actual = pane
                .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .and_then(|document| {
                    first_mismatch.and_then(|line_no| {
                        prepared_document_snapshot_for_line(
                            theme,
                            new_text.as_str(),
                            new_line_starts.as_ref(),
                            document,
                            rows::DiffSyntaxLanguage::Yaml,
                            line_no,
                        )
                    })
                });
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={} split_mismatches={split_mismatches:?} first_mismatch={first_mismatch:?} cache_row_ix={cache_row_ix:?} provider_row_ix={provider_row_ix:?} cached_text={cached_text:?} actual={actual:?} doc_actual={doc_actual:?} expected={expected:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );

    let split_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        affected_lines
            .iter()
            .copied()
            .map(|line| {
                split_visible_ix_by_new_line(pane, line).unwrap_or_else(|| {
                    panic!("expected split visible row for build-release line {line}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    let (epoch_before, right_doc_ready_before, heuristic_mismatches) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines),
        )
    });
    if !right_doc_ready_before {
        assert!(
            !heuristic_mismatches.is_empty(),
            "expected at least one build-release YAML block-scalar line to differ while only heuristic styling is cached"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(500),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 2, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release file-diff rows ready after same-content refresh",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && affected_lines
                    .iter()
                    .copied()
                    .all(|line| split_visible_ix_by_new_line(pane, line).is_some())
        },
        |pane| {
            let split_mismatches =
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines);
            let first_mismatch = split_mismatches.first().copied();
            let actual = first_mismatch.and_then(|line_no| {
                split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
            });
            let expected =
                first_mismatch.and_then(|line_no| baseline_new_by_line.get(&line_no).cloned());
            let doc_actual = pane
                .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .and_then(|document| {
                    first_mismatch.and_then(|line_no| {
                        prepared_document_snapshot_for_line(
                            theme,
                            new_text.as_str(),
                            new_line_starts.as_ref(),
                            document,
                            rows::DiffSyntaxLanguage::Yaml,
                            line_no,
                        )
                    })
                });
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={} split_mismatches={split_mismatches:?} first_mismatch={first_mismatch:?} actual={actual:?} doc_actual={doc_actual:?} expected={expected:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should expose the build-release right document",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && (right_doc_ready_before
                    || pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight)
                        > epoch_before)
        },
        |pane| {
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should finish build-release right-doc chunk requests",
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
        },
        |pane| {
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            format!(
                "rev={} right_doc={right_doc:?} right_pending={:?} split_mismatches={:?}",
                pane.file_diff_cache_rev,
                right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.run_until_parked();

    for (&line_no, &visible_ix) in affected_lines.iter().zip(split_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let cached = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
        });
        let expected = baseline_new_by_line
            .get(&line_no)
            .unwrap_or_else(|| panic!("missing build-release baseline for line {line_no}"));
        assert_eq!(
            cached,
            Some(expected.clone()),
            "diagnostic: split-right cache should match the prepared baseline after painting line {line_no}"
        );
        let actual = paint_snapshot(&record);
        assert_eq!(
            actual, *expected,
            "same-content refresh should repaint split-right build-release YAML highlighting for line {line_no}"
        );

        let expects_row_bg = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (0..pane.file_diff_split_row_len()).any(|row_ix| {
                pane.file_diff_split_row(row_ix).is_some_and(|row| {
                    row.new_line == Some(line_no)
                        && matches!(
                            row.kind,
                            gitcomet_core::file_diff::FileDiffRowKind::Add
                                | gitcomet_core::file_diff::FileDiffRowKind::Modify
                        )
                })
            })
        });
        assert_eq!(
            record.row_bg.is_some(),
            expects_row_bg,
            "same-content refresh should preserve split-right diff background for line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        affected_lines
            .iter()
            .copied()
            .map(|line| {
                inline_visible_ix_by_new_line(pane, line).unwrap_or_else(|| {
                    panic!("expected inline visible row for build-release line {line}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should expose inline build-release rows",
        |pane| {
            pane.file_diff_cache_rev == 2
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    for (&line_no, &visible_ix) in affected_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let expected = baseline_new_by_line
            .get(&line_no)
            .unwrap_or_else(|| panic!("missing build-release baseline for line {line_no}"));
        let actual = paint_snapshot(&record);
        assert_eq!(
            actual, *expected,
            "same-content refresh should repaint inline build-release YAML highlighting for line {line_no}"
        );

        let expects_row_bg = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (0..pane.file_diff_inline_row_len()).any(|inline_ix| {
                pane.file_diff_inline_row(inline_ix).is_some_and(|line| {
                    line.new_line == Some(line_no)
                        && line.kind == gitcomet_core::domain::DiffLineKind::Add
                })
            })
        });
        assert_eq!(
            record.row_bg.is_some(),
            expects_row_bg,
            "same-content refresh should preserve inline diff background for line {line_no}"
        );
    }
}
