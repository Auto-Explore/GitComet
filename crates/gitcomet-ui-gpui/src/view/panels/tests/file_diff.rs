#![allow(dead_code)]
#![allow(clippy::type_complexity)]

use super::*;
use crate::view::panes::main::DiffChangeSide;
use crate::view::panes::main::diff_cache::PatchInlineVisibleMap;
use std::path::PathBuf;

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
    let target = gitcomet_core::domain::DiffTarget::commit(
        gitcomet_core::domain::CommitId("deadbeef".into()),
        path.clone(),
    );
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

fn build_collapsed_diff_scroll_sync_fixture_texts() -> (String, String, String) {
    let total_lines = 260usize;
    let changed_lines = (8..=248usize).step_by(12).collect::<Vec<_>>();
    let mut old_lines = (1..=total_lines)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
    let mut new_lines = old_lines.clone();

    for line in changed_lines.iter().copied() {
        if line == 8 {
            old_lines[line - 1] = format!("old value {line} {}", "left_payload_".repeat(160));
            new_lines[line - 1] = format!("new value {line} {}", "right_payload_".repeat(160));
        } else {
            old_lines[line - 1] = format!("old value {line}");
            new_lines[line - 1] = format!("new value {line}");
        }
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
    for line in changed_lines {
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

fn build_full_file_inline_horizontal_scroll_fixture_texts() -> (String, String, String) {
    let long_added = format!("added value {}", "inline_payload_".repeat(180));
    let old_text = "line 1\nline 2\nline 3\n".to_string();
    let new_text = format!("line 1\n{long_added}\nline 2\nline 3\n");
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 line 1
+{long_added}
 line 2
 line 3
"
    );
    (unified, old_text, new_text)
}

fn build_collapsed_diff_trailing_hscroll_fixture_texts() -> (String, String, String) {
    let old_lines = (1..=70usize)
        .map(|line| {
            if line == 1 {
                format!("old value 1 {}", "left_payload_".repeat(160))
            } else {
                format!("line {line}")
            }
        })
        .collect::<Vec<_>>();
    let new_lines = (1..=70usize)
        .map(|line| {
            if line == 1 {
                format!("new value 1 {}", "right_payload_".repeat(160))
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
@@ -1,4 +1,4 @@
-{}
+{}
 {}
 {}
 {}
",
        old_lines[0], new_lines[0], old_lines[1], old_lines[2], old_lines[3],
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
    debug_selector_bounds(cx, selector).center()
}

fn debug_selector_bounds(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> gpui::Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected `{selector}` bounds"))
}

fn collapsed_hunk_visible_ix_for_src_ix(
    pane: &crate::view::panes::main::MainPaneView,
    src_ix: usize,
) -> usize {
    pane.collapsed_diff_hunks
        .iter()
        .position(|hunk| hunk.src_ix == src_ix)
        .and_then(|hunk_ix| {
            pane.collapsed_diff_hunk_visible_indices
                .get(hunk_ix)
                .copied()
        })
        .map(|visible_ix| pane.diff_visual_ix_for_source_visible_ix(visible_ix))
        .unwrap_or_else(|| panic!("expected a collapsed hunk anchor for src_ix={src_ix}"))
}

fn collapsed_file_row_visible_ix(
    pane: &crate::view::panes::main::MainPaneView,
    target_row_ix: usize,
) -> usize {
    (0..pane.diff_visible_len())
        .find(|&visible_ix| {
            let Some(source_visible_ix) = pane.diff_source_visible_ix_for_visible_ix(visible_ix)
            else {
                return false;
            };
            matches!(
                pane.collapsed_visible_row(source_visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix })
                    if row_ix == target_row_ix
            )
        })
        .unwrap_or_else(|| panic!("expected a collapsed file row for row_ix={target_row_ix}"))
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

fn set_diff_row_selection_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    anchor: usize,
    range: (usize, usize),
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_selection_anchor = Some(anchor);
                pane.diff_selection_range = Some(range);
                pane.clear_diff_text_selection();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
}

/// Visual rows the pane reports as inside the focused change block.
fn focused_change_block_rows(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Vec<usize> {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (0..pane.diff_visible_len())
            .filter(|&visible_ix| pane.diff_focused_change_block_row(visible_ix).is_some())
            .collect()
    })
}

mod cache_and_blame;
mod fixtures;
mod scrolling;
mod syntax;
use fixtures::{BUILD_RELEASE_ARTIFACTS, DEPLOYMENT_CI};
use scrolling::push_file_patch_diff_state_with_rev;

/// A binary side is a placeholder outcome: the pane must classify it instead
/// of dumping the loader error, which names a temp file the user never chose.
#[gpui::test]
fn binary_source_backed_diff_is_classified_as_not_text(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(882);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_binary_source_backed",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create binary fixture");
    let path = PathBuf::from("assets/blob.bin");
    let old_source_path = source_dir.join("old.bin");
    let new_source_path = source_dir.join("new.bin");
    std::fs::write(&old_source_path, [0u8, 0xff, 0xfe, 1, 2]).expect("write old binary");
    std::fs::write(&new_source_path, [0u8, 0xff, 0xfe, 1, 2, 3, 4]).expect("write new binary");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(
                    target,
                    "Binary files a/assets/blob.bin and b/assets/blob.bin differ\n",
                ),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
                        old_source_path.clone(),
                        "old-binary",
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
                        new_source_path.clone(),
                        "new-binary",
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "the binary diff rebuild to settle",
        |pane| pane.file_diff_cache_rev == 1 && pane.file_diff_cache_inflight.is_none(),
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.file_diff_cache_error,
            Some(
                crate::view::panes::main::diff_cache::FileDiffCacheError::NotText {
                    old_bytes: Some(5),
                    new_bytes: Some(7),
                }
            )
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup binary fixture");
}

/// A pointer-only LFS file must be explained by the card, never shown as a
/// three-line pointer diff.
#[gpui::test]
fn missing_lfs_content_shows_the_large_file_card(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(883);
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_lfs_card", std::process::id()));
    let path = PathBuf::from("art/hero.psd");
    let side = |content| gitcomet_core::large_files::LargeFileSide {
        pointer: gitcomet_core::large_files::LargeFilePointer::Lfs(
            gitcomet_core::lfs::LfsPointer {
                oid: gitcomet_core::lfs::LfsOid([9; 32]),
                size: 4_200_000,
            },
        ),
        content,
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some("version https://git-lfs.github.com/spec/v1\n".to_string()),
                    Some("version https://git-lfs.github.com/spec/v1\n".to_string()),
                )
                .with_large_sides(
                    Some(side(
                        gitcomet_core::large_files::LargeFileContent::MissingLocally,
                    )),
                    Some(side(
                        gitcomet_core::large_files::LargeFileContent::MissingLocally,
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("large_file_card").is_some(),
        "the card must explain a pointer-only file"
    );
}

#[gpui::test]
fn lfs_preview_and_image_cards_do_not_require_a_text_diff(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::domain::{
        DiffPreviewTextFile, DiffPreviewTextSide, FileDiffImage, FileStatusKind,
    };
    use gitcomet_core::large_files::{LargeFileContent, LargeFilePointer, LargeFileSide};
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let dir = tempfile::tempdir().unwrap();
    let side = LargeFileSide {
        pointer: LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer {
            oid: gitcomet_core::lfs::LfsOid([9; 32]),
            size: 4_200_000,
        }),
        content: LargeFileContent::MissingLocally,
    };
    for (index, (name, status, preview_side)) in [
        ("added.txt", FileStatusKind::Added, DiffPreviewTextSide::New),
        (
            "deleted.txt",
            FileStatusKind::Deleted,
            DiffPreviewTextSide::Old,
        ),
        (
            "image.png",
            FileStatusKind::Modified,
            DiffPreviewTextSide::New,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let path = PathBuf::from(name);
        let repo_id = gitcomet_state::model::RepoId(890 + index as u64);
        let preview_path = dir.path().join("cached-pointer");
        std::fs::write(
            &preview_path,
            "version https://git-lfs.github.com/spec/v1\n",
        )
        .unwrap();
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, dir.path());
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    status,
                    gitcomet_core::domain::DiffArea::Staged,
                );
                if name.ends_with(".png") {
                    repo.diff_state.diff_file_image =
                        Loadable::Ready(Some(Arc::new(FileDiffImage {
                            path: path.clone(),
                            new_large: Some(side.clone()),
                            ..Default::default()
                        })));
                } else {
                    repo.diff_state.diff_preview_text_file =
                        Loadable::Ready(Some(Arc::new(DiffPreviewTextFile {
                            path: preview_path.clone(),
                            side: preview_side,
                            large_file: Some(side.clone()),
                        })));
                    repo.diff_state.diff_preview_text_file_rev = 1;
                }
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
        draw_and_drain_test_window(cx);
        assert!(
            cx.debug_bounds("large_file_card").is_some(),
            "{name} needs the metadata card"
        );
        assert!(
            cx.debug_bounds("large_file_card_download").is_some(),
            "{name} needs a download action"
        );
    }
}

#[gpui::test]
fn lfs_payload_diff_disables_pointer_patch_actions(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::large_files::{LargeFileContent, LargeFilePointer, LargeFileSide};
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let repo_id = gitcomet_state::model::RepoId(894);
            let mut repo = opening_repo_state(repo_id, &std::env::temp_dir());
            let path = PathBuf::from("data.txt");
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff::from_unified(
                repo.diff_state.diff_target.clone().unwrap(),
                "diff --git a/data.txt b/data.txt\nindex 1111111..2222222 100644\n--- a/data.txt\n+++ b/data.txt\n@@ -1,3 +1,3 @@\n version https://git-lfs.github.com/spec/v1\n-oid sha256:old\n+oid sha256:new\n size 12\n",
            )));
            let side = LargeFileSide {
                pointer: LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer {
                    oid: gitcomet_core::lfs::LfsOid([9; 32]),
                    size: 12,
                }),
                content: LargeFileContent::Available,
            };
            repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path,
                    Some("old\n".into()),
                    Some("new\n".into()),
                )
                .with_large_sides(Some(side.clone()), Some(side)),
            )));
            repo.diff_state.diff_file_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            assert_eq!(this.main_pane.read(cx).diff_stage_gutter_area(), None);
            this.popover_host.update(cx, |host, cx| {
                // Row four is also a valid pointer-patch hunk index. Even a
                // stale or programmatically opened menu must not act on it.
                let model = host.context_menu_model(&PopoverKind::DiffHunkMenu { repo_id, src_ix: 4 }, cx).unwrap();
                let entries: Vec<_> = model.items.iter().filter_map(|item| match item {
                    ContextMenuItem::Entry { disabled, .. } => Some(*disabled),
                    _ => None,
                }).collect();
                assert_eq!(entries, [true, true]);
            });
        });
    });
}

#[gpui::test]
fn lfs_collapsed_diff_keeps_payload_changes_and_expands_context(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::large_files::{LargeFileContent, LargeFilePointer, LargeFileSide};
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let dir = tempfile::tempdir().unwrap();
    let old: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    let new = old
        .replace("line 8\n", "changed 8\n")
        .replace("line 22\n", "changed 22\n");
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let repo_id = gitcomet_state::model::RepoId(895);
            let mut repo = opening_repo_state(repo_id, dir.path());
            let path = PathBuf::from("data.txt");
            set_test_file_status(&mut repo, path.clone(), gitcomet_core::domain::FileStatusKind::Modified, gitcomet_core::domain::DiffArea::Unstaged);
            let side = LargeFileSide {
                pointer: LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer {
                    oid: gitcomet_core::lfs::LfsOid([9; 32]), size: 240,
                }), content: LargeFileContent::Available,
            };
            repo.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff::from_unified(
                repo.diff_state.diff_target.clone().unwrap(),
                "@@ -1,3 +1,3 @@\n version https://git-lfs.github.com/spec/v1\n-oid sha256:old\n+oid sha256:new\n size 240\n",
            )));
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
                path, Some(old), Some(new)
            ).with_large_sides(Some(side.clone()), Some(side)))));
            repo.diff_state.diff_file_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    for mode in [DiffViewMode::Split, DiffViewMode::Inline] {
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                pane.diff_content_mode = DiffContentMode::Collapsed;
                pane.diff_view = mode;
                pane.reset_collapsed_diff_projection(true);
                cx.notify();
            });
        });
        wait_for_main_pane_condition(
            cx,
            &view,
            "LFS payload hunks are visible",
            |pane| {
                pane.is_collapsed_diff_projection_active()
                    && !pane.collapsed_diff_visible_rows.is_empty()
            },
            |pane| {
                format!(
                    "rows={:?}, inflight={:?}",
                    pane.collapsed_diff_visible_rows, pane.file_diff_cache_inflight
                )
            },
        );
        let visible_ix = cx.update(|_, app| {
            view.read(app)
                .main_pane
                .read(app)
                .collapsed_diff_hunk_visible_indices[0]
        });
        let regions: &[DiffTextRegion] = match mode {
            DiffViewMode::Inline => &[DiffTextRegion::Inline],
            DiffViewMode::Split => &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight],
        };
        for &region in regions {
            let click = wait_for_diff_text_click_position_for_offset_range(
                cx,
                &view,
                visible_ix,
                region,
                0..1,
                "LFS hunk header",
            );
            cx.simulate_mouse_down(click, MouseButton::Right, Modifiers::default());
            cx.simulate_mouse_up(click, MouseButton::Right, Modifiers::default());
            draw_and_drain_test_window(cx);
            cx.update(|_, app| {
                assert!(!matches!(
                    view.read(app)
                        .popover_host
                        .read(app)
                        .popover_kind_for_tests(),
                    Some(PopoverKind::DiffHunkMenu { .. })
                ));
            });
        }
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                assert_eq!(
                    pane.collapsed_diff_hunks.len(),
                    2,
                    "two payload hunks, not one pointer hunk"
                );
                let headers: Vec<_> = pane
                    .collapsed_diff_hunks
                    .iter()
                    .map(|hunk| {
                        pane.collapsed_diff_hunk_header_display(hunk.src_ix)
                            .unwrap()
                            .to_string()
                    })
                    .collect();
                assert_eq!(headers, ["-5,7 +5,7", "-19,7 +19,7"]);
                assert_eq!(pane.collapsed_change_blocks().len(), 2);
                assert_eq!(pane.diff_stage_gutter_area(), None);
                let first = pane.collapsed_diff_hunks[0].src_ix;
                pane.collapsed_diff_reveal_hunk_up(first, cx);
                assert_eq!(
                    pane.collapsed_diff_hunk_header_display(first)
                        .unwrap()
                        .as_ref(),
                    "-1,11 +1,11"
                );
            });
        });
    }
}

/// An annexed file without local content offers Get, and "Where is it?"
/// results appear under the card once loaded for that content key.
#[gpui::test]
fn missing_annex_content_offers_get_and_lists_copies(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(884);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_annex_card",
        std::process::id()
    ));
    let path = PathBuf::from("data/scan.tif");
    let side = gitcomet_core::large_files::LargeFileSide {
        pointer: gitcomet_core::large_files::LargeFilePointer::Annex(
            gitcomet_core::annex::parse_key("SHA256E-s4200000--abc.tif").unwrap(),
        ),
        content: gitcomet_core::large_files::LargeFileContent::Unknown,
    };

    let push = |cx: &mut gpui::VisualTestContext, whereis: bool| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                repo.diff_state.diff_file_rev = 1;
                repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                    gitcomet_core::domain::FileDiffText::new(
                        path.clone(),
                        Some("/annex/objects/SHA256E-s4200000--abc.tif\n".to_string()),
                        Some("/annex/objects/SHA256E-s4200000--abc.tif\n".to_string()),
                    )
                    .with_large_sides(Some(side.clone()), Some(side.clone())),
                )));
                if whereis {
                    repo.annex_whereis.insert(
                        "SHA256E-s4200000--abc.tif".into(),
                        gitcomet_state::model::Loadable::Ready(Arc::new(
                            gitcomet_core::large_files::AnnexWhereis {
                                key: "SHA256E-s4200000--abc.tif".into(),
                                copies: vec![gitcomet_core::large_files::AnnexLocation {
                                    uuid: "u-backup".into(),
                                    description: "[backup]".into(),
                                    here: false,
                                }],
                                untrusted: Vec::new(),
                            },
                        )),
                    );
                    repo.annex_whereis_rev = 1;
                }
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
        draw_and_drain_test_window(cx);
    };

    push(cx, false);
    assert!(cx.debug_bounds("large_file_card").is_some());
    assert!(cx.debug_bounds("large_file_card_annex_get").is_some());
    assert!(cx.debug_bounds("large_file_card_annex_whereis").is_some());
    assert!(cx.debug_bounds("large_file_card_whereis").is_none());
    push(cx, true);
    assert!(
        cx.debug_bounds("large_file_card_whereis").is_some(),
        "loaded copies are listed under the card"
    );
}

mod folding;
mod layout;
mod navigation;
mod selection;
mod wrapping;
