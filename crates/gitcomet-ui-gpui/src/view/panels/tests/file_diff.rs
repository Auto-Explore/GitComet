#![allow(dead_code)]
#![allow(clippy::type_complexity)]

use super::*;
use crate::view::panes::main::DiffChangeSide;
use crate::view::panes::main::diff_cache::PatchInlineVisibleMap;
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
        to_commit_id: Some(gitcomet_core::domain::CommitId("bbbb".into())),
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
                    origin: gitcomet_state::model::ForeignDiffOrigin::Submodule,
                    submodule_repo_path: submodule_workdir.clone(),
                    parent_submodule_path: PathBuf::from("vendor/submodule"),
                    entries: vec![gitcomet_state::model::InlineSubmoduleDiffEntry {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        target: target.clone(),
                        section: gitcomet_state::model::InlineSubmoduleDiffSection::Range(
                            gitcomet_core::domain::SubmoduleDiffRangeKind::CommitHistory,
                        ),
                    }]
                    .into(),
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

#[gpui::test]
fn same_file_refresh_keeps_rows_instead_of_flashing_processing(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(70720);
    let path = PathBuf::from("src/lib.rs");

    let unified_before = concat!(
        "diff --git a/src/lib.rs b/src/lib.rs\n",
        "--- a/src/lib.rs\n",
        "+++ b/src/lib.rs\n",
        "@@ -1,3 +1,3 @@\n",
        " one\n",
        "-two\n",
        "+two_mod\n",
        " three\n",
    );
    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "keep_rows",
        path.clone(),
        1,
        unified_before.to_string(),
        "one\ntwo\nthree\n".to_string(),
        "one\ntwo_mod\nthree\n".to_string(),
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "the file diff rows to be built",
        |pane| {
            pane.file_diff_cache_content_signature.is_some()
                && pane.diff_visible_len() > 0
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "visible_len={} right_doc={:?}",
                pane.diff_visible_len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );
    let (syntax_generation_before, old_rows_document) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.file_diff_syntax_generation,
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .expect("old rows should have a prepared right document"),
        )
    });

    // Staging a line reloads the same file with different content. The rebuild
    // must not blank the pane: the previous rows stay up until the new ones land.
    let unified_after = concat!(
        "diff --git a/src/lib.rs b/src/lib.rs\n",
        "--- a/src/lib.rs\n",
        "+++ b/src/lib.rs\n",
        "@@ -1,3 +1,3 @@\n",
        " one\n",
        "-three\n",
        "+three_mod\n",
    );
    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "keep_rows",
        path,
        2,
        unified_after.to_string(),
        "one\ntwo_mod\nthree\n".to_string(),
        "one\ntwo_mod\nthree_mod\n".to_string(),
    );

    // Draw without draining, so the rebuild is still in flight.
    crate::view::test_support::redraw(cx);
    let (inflight, has_rows, syntax_generation_during) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.file_diff_cache_inflight.is_some(),
            pane.file_diff_cache_content_signature.is_some(),
            pane.file_diff_syntax_generation,
        )
    });
    assert!(inflight, "expected the same-file rebuild to be in flight");
    assert!(
        has_rows,
        "the previous rows must survive the rebuild, or the pane flashes a placeholder"
    );
    assert_eq!(
        syntax_generation_during, syntax_generation_before,
        "the visible rows must keep their generation until the replacement row swap"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let replacement_key = pane
                    .file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight)
                    .expect("in-flight replacement key");
                pane.prepared_syntax_documents
                    .insert(replacement_key, old_rows_document);
            });
        });
    });

    draw_and_drain_test_window(cx);
    let (has_content, syntax_generation_after, replacement_document) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.file_diff_cache_content_signature.is_some(),
            pane.file_diff_syntax_generation,
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
        )
    });
    assert!(
        has_content,
        "the rebuilt rows must be in place once the refresh lands"
    );
    assert_ne!(
        syntax_generation_after, syntax_generation_before,
        "installing replacement rows must advance their syntax generation"
    );
    assert_ne!(
        replacement_document,
        Some(old_rows_document),
        "a document prepared from kept rows under the incoming rev must be discarded at the row swap"
    );
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

/// Three change blocks; the first spans two lines so navigation can start mid-block.
fn build_full_diff_multi_block_fixture_texts() -> (String, String, String) {
    let old_text =
        "alpha\nold one\nold two\nmiddle one\nold three\nmiddle two\nold four\nomega\n".to_string();
    let new_text =
        "alpha\nnew one\nnew two\nmiddle one\nnew three\nmiddle two\nnew four\nomega\n".to_string();
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,8 +1,8 @@
 alpha
-old one
-old two
+new one
+new two
 middle one
-old three
+new three
 middle two
-old four
+new four
 omega
"
    .to_string();
    (unified, old_text, new_text)
}

/// The first line is changed, so the first change block starts at row 0.
fn build_full_diff_first_row_change_fixture_texts() -> (String, String, String) {
    let old_text = "old first\nkeep\nold last\n".to_string();
    let new_text = "new first\nkeep\nnew last\n".to_string();
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
-old first
+new first
 keep
-old last
+new last
"
    .to_string();
    (unified, old_text, new_text)
}

/// Two change blocks; the first one's row wraps, so its continuation rows sit
/// between the two navigation stops.
fn build_full_diff_word_wrap_navigation_fixture_texts() -> (String, String, String) {
    let old_one = format!("old first {}", "left_payload_".repeat(160));
    let new_one = format!("new first {}", "right_payload_".repeat(160));
    let old_two = "old second changed row".to_string();
    let new_two = "new second changed row".to_string();
    let old_text = format!("alpha\n{old_one}\nmiddle\n{old_two}\nomega\n");
    let new_text = format!("alpha\n{new_one}\nmiddle\n{new_two}\nomega\n");
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,5 @@
 alpha
-{old_one}
+{new_one}
 middle
-{old_two}
+{new_two}
 omega
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

fn build_collapsed_diff_word_wrap_navigation_fixture_texts() -> (String, String, String) {
    let total_lines = 100usize;
    let changes = [20usize, 60usize];
    let mut old_lines = (1..=total_lines)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
    let mut new_lines = old_lines.clone();

    old_lines[changes[0] - 1] = format!("old value 20 {}", "left_payload_".repeat(160));
    new_lines[changes[0] - 1] = format!("new value 20 {}", "right_payload_".repeat(160));
    old_lines[changes[1] - 1] = "old value 60".to_string();
    new_lines[changes[1] - 1] = "new value 60".to_string();

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
    for line in changes {
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

fn push_collapsed_diff_loading_fixture_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    file_ready: bool,
) -> gitcomet_core::domain::DiffTarget {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_collapsed_loading_root",
        std::process::id(),
        fixture_name
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
        path: Some(path.clone()),
    };
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);
    let file_diff = gitcomet_core::domain::FileDiffText::new(path, Some(old_text), Some(new_text));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = if file_ready {
                gitcomet_state::model::Loadable::Ready(Some(Arc::new(file_diff)))
            } else {
                gitcomet_state::model::Loadable::Loading
            };

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    target
}

fn assert_collapsed_diff_hunk_height(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    expected: gpui::Pixels,
) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected `{selector}` bounds"));
    let actual_height: f32 = bounds.size.height.into();
    let expected_height: f32 = expected.into();
    assert!(
        (actual_height - expected_height).abs() < 0.01,
        "expected `{selector}` height {expected_height}, got {actual_height}"
    );
}

fn assert_collapsed_diff_loading_does_not_render_patch_rows(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &'static str,
    diff_view: DiffViewMode,
) {
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_content_mode = DiffContentMode::Collapsed;
            pane.diff_view = diff_view;
            cx.notify();
        });
    });

    let target = push_collapsed_diff_loading_fixture_state(cx, view, repo_id, fixture_name, false);
    cx.run_until_parked();

    let paint_log = cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        let _ = window.draw(app);
        rows::diff_paint_log_for_tests()
    });
    assert!(
        paint_log.is_empty(),
        "collapsed loading should not render raw patch rows, got {paint_log:?}"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_content_mode, DiffContentMode::Collapsed);
        assert!(
            !pane.is_collapsed_diff_projection_active(),
            "loading file contents must not activate the collapsed projection"
        );
        assert!(
            pane.patch_diff_row_len() > 0,
            "patch rows should be cached but not rendered while collapsed file contents load"
        );
    });

    push_collapsed_diff_loading_fixture_state(cx, view, repo_id, fixture_name, true);
    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff loading fixture activates collapsed projection",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
                && !pane.collapsed_diff_hunk_visible_indices.is_empty()
        },
        |pane| {
            format!(
                "mode={:?} view={:?} collapsed_active={} inflight={:?} cache_target={:?} patch_rows={} file_rows={} collapsed_rows={} hunk_rows={:?}",
                pane.diff_content_mode,
                pane.diff_view,
                pane.is_collapsed_diff_projection_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target,
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
                pane.collapsed_diff_visible_rows.len(),
                pane.collapsed_diff_hunk_visible_indices,
            )
        },
    );
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            let hunk_visible_ix = pane.collapsed_diff_hunk_visible_indices[0];
            pane.scroll_diff_to_item_strict(hunk_visible_ix, gpui::ScrollStrategy::Top);
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    let expected = cx.update(|_window, app| {
        AppTheme::gitcomet_dark()
            .editor_row_height(crate::ui_scale::UiScale::current(app).percent())
    });
    match diff_view {
        DiffViewMode::Inline => {
            assert_collapsed_diff_hunk_height(cx, "collapsed_diff_inline_hunk_shell", expected);
        }
        DiffViewMode::Split => {
            assert_collapsed_diff_hunk_height(cx, "collapsed_diff_split_left_hunk_shell", expected);
            assert_collapsed_diff_hunk_height(
                cx,
                "collapsed_diff_split_right_hunk_shell",
                expected,
            );
        }
    }
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

#[gpui::test]
fn collapsed_diff_inline_loading_does_not_render_patch_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_loading_does_not_render_patch_rows(
        cx,
        &view,
        gitcomet_state::model::RepoId(260),
        "collapsed_inline_loading",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_loading_does_not_render_patch_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_loading_does_not_render_patch_rows(
        cx,
        &view,
        gitcomet_state::model::RepoId(261),
        "collapsed_split_loading",
        DiffViewMode::Split,
    );
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

/// First changed collapsed file row at or after `from`, as a source-visible index.
fn collapsed_first_changed_source_visible_ix(
    pane: &crate::view::panes::main::MainPaneView,
    from: usize,
) -> usize {
    (from..pane.collapsed_diff_visible_rows.len())
        .find(|&source_visible_ix| {
            pane.collapsed_visible_row(source_visible_ix)
                .and_then(crate::view::panes::main::CollapsedDiffVisibleRow::row_ix)
                .is_some_and(|row_ix| pane.file_diff_row_is_change(row_ix))
        })
        .unwrap_or_else(|| panic!("expected a changed collapsed file row at or after {from}"))
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
        // Line 35 is the only change: row 34 in both layouts.
        assert_eq!(
            pane.diff_nav_entries(),
            vec![collapsed_file_row_visible_ix(pane, 34)],
            "diff navigation should stop on the changed row, not the hunk anchor, once the top expansion row disappears"
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
            "fully revealed collapsed hunks should anchor to the first file row instead of a synthetic header"
        );
        assert_eq!(
            pane.diff_nav_entries(),
            vec![collapsed_file_row_visible_ix(pane, 34)],
            "diff navigation should keep stopping on the changed row once the hunk is fully revealed"
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

fn assert_collapsed_diff_file_switch_resets_expanded_context(
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

    let (hunk_src_ix, hidden_up_before, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected initial collapsed fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });
    assert!(hidden_up_before >= 20 && hidden_down_before >= 20);

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_up(hunk_src_ix, cx);
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.collapsed_diff_reveals.is_empty(),
            "fixture should have persisted expanded collapsed-diff context before switching files"
        );
        assert!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix) < hidden_up_before,
            "upward reveal should reduce the hidden-up budget before switching files"
        );
        assert!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix) < hidden_down_before,
            "downward reveal should reduce the hidden-down budget before switching files"
        );
    });

    let next_path = PathBuf::from("src/other.rs");
    let (next_unified, next_old_text, next_new_text) =
        build_collapsed_diff_multi_hunk_fixture_texts(&[(
            60,
            "old other value 60",
            "new other value 60",
        )]);
    let next_path_for_patch = next_path.to_string_lossy().replace('\\', "/");
    let next_unified = next_unified.replace("src/lib.rs", &next_path_for_patch);
    let next_target = push_regular_diff_content_mode_state_with_rev(
        cx,
        view,
        repo_id,
        fixture_name,
        next_path.clone(),
        2,
        next_unified,
        next_old_text,
        next_new_text,
    );
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff projection switches to the second file",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && pane.file_diff_cache_target == Some(next_target.clone())
                && !pane.collapsed_diff_hunks.is_empty()
        },
        collapsed_diff_cache_rebuild_snapshot,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected second collapsed fixture to expose one hunk");
        assert_eq!(
            pane.collapsed_diff_projection_identity
                .as_ref()
                .map(|identity| &identity.diff_target),
            Some(&next_target),
            "collapsed projection identity should follow the newly selected file target"
        );
        assert!(
            pane.collapsed_diff_reveals.is_empty(),
            "expanded collapsed-diff context from the previous file must not leak into the next file"
        );
        assert!(
            hunk.base_row_start > 50,
            "expected the rebuilt collapsed hunk to map to the second file's line-60 change, got {hunk:?}"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
            hunk.base_row_start,
            "the second file should start with default hidden context above its hunk"
        );
    });
}

#[gpui::test]
fn collapsed_diff_inline_file_switch_resets_expanded_context(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_file_switch_resets_expanded_context(
        cx,
        &view,
        gitcomet_state::model::RepoId(289),
        "collapsed_inline_file_switch_resets_context",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_file_switch_resets_expanded_context(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_file_switch_resets_expanded_context(
        cx,
        &view,
        gitcomet_state::model::RepoId(290),
        "collapsed_split_file_switch_resets_context",
        DiffViewMode::Split,
    );
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
        "expected the removed counter to be rendered in the left (before) pane header"
    );
    assert!(
        cx.debug_bounds("diff_split_header_added_stat").is_some(),
        "expected the added counter to be rendered in the right (after) pane header"
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
fn diff_word_wrap_toggles_full_file_diff_wrapped_row_path(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = "new line with enough text to exercise the soft wrap render path softwrapneedle abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
 first line
-old line
+new line with enough text to exercise the soft wrap render path softwrapneedle abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
"
    .to_string();
    let old_text = "first line\nold line\n".to_string();
    let new_text = format!("first line\n{long_new_line}\n");
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(187),
        "word_wrap_full_file_diff",
        path,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(false, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "full file diff ready for word wrap toggle",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && pane.diff_visible_len() > 0
        },
        |pane| {
            format!(
                "cache_inflight={:?} file_active={} visible_len={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active(),
                pane.diff_visible_len(),
            )
        },
    );
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_word_wrap_scroll").is_none(),
        "word wrap off should render the file diff row list"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    cx.update(|_window, app| {
        assert!(crate::view::test_support::diff_word_wrap(view.read(app)));
        assert!(view.read(app).main_pane.read(app).diff_word_wrap);
    });
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_word_wrap_scroll").is_none(),
        "word wrap on should stay on the normal highlighted diff row renderer"
    );
    assert!(
        cx.debug_bounds("diff_hscrollbar").is_none(),
        "word wrap on should suppress the horizontal scrollbar"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_visible_len() > pane.file_diff_inline_row_len(),
            "word wrap should expand long logical rows into continuation visual rows"
        );
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .any(|row| row.wrap_ix > 0),
            "wrapped continuation rows should be tracked separately from logical rows"
        );
    });

    let (_continuation_ix, source_visible_ix, continuation_start, continuation_text) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            pane.diff_wrap_visible_rows
                .iter()
                .enumerate()
                .find_map(|(visible_ix, visual)| {
                    if visual.wrap_ix == 0 {
                        return None;
                    }
                    let row_ix = pane.diff_mapped_ix_for_visible_ix(visible_ix)?;
                    let row = pane.file_diff_inline_render_data(row_ix)?;
                    if !row.text.as_ref().contains("softwrapneedle") {
                        return None;
                    }
                    let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                    let full_text = pane.diff_text_full_line_for_region(
                        visual.source_visible_ix,
                        DiffTextRegion::Inline,
                    );
                    let start = full_text.as_ref().find(text.as_ref())?;
                    (!text.is_empty() && !row.text.as_ref().starts_with(text.as_ref())).then_some((
                        visible_ix,
                        visual.source_visible_ix,
                        start,
                        text.to_string(),
                    ))
                })
                .expect(
                    "expected a non-prefix wrapped continuation row for the long file-diff line",
                )
        });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: continuation_start,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: continuation_start + continuation_text.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(continuation_text.clone()),
        "copying a wrapped continuation row should copy the visible slice, not the start of the logical line"
    );

    let full_wrapped_line = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let full = pane
            .diff_text_full_line_for_region(source_visible_ix, DiffTextRegion::Inline)
            .to_string();
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .filter(|row| row.source_visible_ix == source_visible_ix)
                .count()
                > 1,
            "expected selected source row to be split across wrapped visual rows"
        );
        full
    });
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: full_wrapped_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(full_wrapped_line.clone()),
        "copying across wrapped visual rows should not insert soft-wrap newlines"
    );
    assert!(
        !full_wrapped_line.contains('\n'),
        "fixture line should be a single logical source line"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = "softwrapneedle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("softwrapneedle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    rows::clear_diff_paint_log_for_tests();
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "search should match only the wrapped visual row containing the query"
        );
        let match_ix = pane.diff_search_matches[0];
        assert!(
            pane.diff_text_line_for_region(match_ix, DiffTextRegion::Inline)
                .as_ref()
                .contains("softwrapneedle"),
            "the active search match should point at the wrapped slice that contains the query"
        );
    });

    let boundary_query_start = continuation_start.saturating_sub(8);
    let boundary_query_end = (continuation_start + 8).min(full_wrapped_line.len());
    assert!(
        boundary_query_start < continuation_start && continuation_start < boundary_query_end,
        "expected enough text around the soft-wrap boundary"
    );
    let soft_wrap_boundary_literal_query = format!(
        "{}{}",
        &full_wrapped_line[boundary_query_start..continuation_start],
        &full_wrapped_line[continuation_start..boundary_query_end]
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = soft_wrap_boundary_literal_query.clone().into();
                let query_for_input = soft_wrap_boundary_literal_query.clone();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query_for_input, cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "literal search should match across a soft-wrap boundary in the source row"
        );
        let match_ix = pane.diff_search_matches[0];
        assert_eq!(
            pane.diff_wrap_visible_rows
                .get(match_ix)
                .map(|row| row.source_visible_ix),
            Some(source_visible_ix),
            "wrapped boundary matches should map back to the source row"
        );
    });

    let soft_wrap_boundary_query = format!(
        "{}\n{}",
        &full_wrapped_line[boundary_query_start..continuation_start],
        &full_wrapped_line[continuation_start..boundary_query_end]
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = soft_wrap_boundary_query.clone().into();
                let query_for_input = soft_wrap_boundary_query.clone();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query_for_input, cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_matches.is_empty(),
            "search should not treat a soft-wrap boundary as a real newline"
        );
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "softwrapneedle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("softwrapneedle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });

    let paint_log = rows::diff_paint_log_for_tests();
    let highlighted_text = paint_log
        .iter()
        .flat_map(|record| {
            record
                .highlights
                .iter()
                .filter(|(_, _, background)| background.is_some())
                .filter_map(|(range, _, _)| record.text.as_ref().get(range.clone()))
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        highlighted_text.contains("softwrapneedle"),
        "wrapped row rendering should preserve search highlighting"
    );

    let key_before_resize = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
    });
    cx.simulate_resize(gpui::size(px(760.0), px(520.0)));
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_ne!(
            pane.diff_wrap_visible_cache_key, key_before_resize,
            "resizing should rebuild the wrap projection key"
        );
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "search matches should be recomputed in the resized wrapped-row index space"
        );
        let match_ix = pane.diff_search_matches[0];
        assert!(
            match_ix < pane.diff_visible_len()
                && pane
                    .diff_text_line_for_region(match_ix, DiffTextRegion::Inline)
                    .as_ref()
                    .contains("softwrapneedle"),
            "resized search match should still point at the visual row containing the query"
        );
        assert!(
            !pane.diff_scrollbar_markers_cache.is_empty(),
            "scrollbar markers should be recomputed after the wrap projection changes"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(false, cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_word_wrap_scroll").is_none(),
        "turning word wrap off should restore the file diff row list"
    );
}

#[gpui::test]
fn split_diff_word_wrap_copy_omits_soft_wrap_newlines(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(860.0), px(460.0)));

    let (unified, old_text, new_text) = build_full_diff_word_wrap_navigation_fixture_texts();
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(2192),
        "split_word_wrap_copy_real_content",
        PathBuf::from("src/lib.rs"),
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "split file diff ready for word-wrap copy",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && pane.diff_visible_len() > 0
        },
        |pane| {
            format!(
                "cache_inflight={:?} file_active={} visible_len={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active(),
                pane.diff_visible_len(),
            )
        },
    );
    draw_and_drain_test_window(cx);

    let (source_visible_ix, full_wrapped_line, next_source_visible_ix, next_source_line) = cx
        .update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (source_visible_ix, full_wrapped_line) = pane
                .diff_wrap_visible_rows
                .iter()
                .map(|row| row.source_visible_ix)
                .find_map(|source_visible_ix| {
                    let full = pane
                        .diff_text_full_line_for_region(
                            source_visible_ix,
                            DiffTextRegion::SplitRight,
                        )
                        .to_string();
                    if !full.contains("right_payload_") {
                        return None;
                    }
                    let wrap_row_count = pane
                        .diff_wrap_visible_rows
                        .iter()
                        .filter(|row| row.source_visible_ix == source_visible_ix)
                        .count();
                    (wrap_row_count > 1).then_some((source_visible_ix, full))
                })
                .expect("expected a wrapped split-right source row");
            let (next_source_visible_ix, next_source_line) = pane
                .diff_wrap_visible_rows
                .iter()
                .map(|row| row.source_visible_ix)
                .find_map(|ix| {
                    if ix <= source_visible_ix {
                        return None;
                    }
                    let text = pane
                        .diff_text_full_line_for_region(ix, DiffTextRegion::SplitRight)
                        .to_string();
                    (!text.is_empty()).then_some((ix, text))
                })
                .expect("expected a following real split-right source row");
            (
                source_visible_ix,
                full_wrapped_line,
                next_source_visible_ix,
                next_source_line,
            )
        });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: full_wrapped_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(full_wrapped_line.clone()),
        "split diff copy should not insert soft-wrap newlines"
    );
    assert!(!full_wrapped_line.contains('\n'));

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix: next_source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: next_source_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(format!("{full_wrapped_line}\n{next_source_line}")),
        "copying real source rows should still preserve real line breaks"
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_continuation_rows_use_source_visible_row(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let repo_id = gitcomet_state::model::RepoId(188);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_word_wrap_source_row",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let (continuation_ix, source_visible_ix, expected_text) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_wrap_visible_rows
            .iter()
            .enumerate()
            .find_map(|(visible_ix, visual)| {
                if visual.wrap_ix == 0 {
                    return None;
                }
                let Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix }) =
                    pane.collapsed_visible_row(visual.source_visible_ix)
                else {
                    return None;
                };
                let row = pane.file_diff_inline_render_data(row_ix)?;
                if !row.text.as_ref().contains("right_payload_") {
                    return None;
                }
                let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                text.as_ref().contains("right_payload_").then_some((
                    visible_ix,
                    visual.source_visible_ix,
                    text.to_string(),
                ))
            })
            .expect("expected a wrapped collapsed file row continuation")
    });
    assert_ne!(
        continuation_ix, source_visible_ix,
        "the regression needs a continuation row whose visual index differs from the source row"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(continuation_ix, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });
    rows::clear_diff_paint_log_for_tests();
    draw_and_drain_test_window(cx);
    let record = rows::diff_paint_log_for_tests()
        .into_iter()
        .find(|record| {
            record.visible_ix == continuation_ix && record.region == DiffTextRegion::Inline
        })
        .unwrap_or_else(|| {
            panic!("expected paint record for wrapped continuation {continuation_ix}")
        });
    assert_eq!(
        record.text.as_ref(),
        expected_text,
        "collapsed wrapped continuation rows should render the source row slice, not the next collapsed logical row"
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_copy_uses_continuation_slice(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let repo_id = gitcomet_state::model::RepoId(190);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_word_wrap_copy_slice",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let (continuation_ix, source_visible_ix, selection_start, expected_text) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            pane.diff_wrap_visible_rows
                .iter()
                .enumerate()
                .find_map(|(visible_ix, visual)| {
                    if visual.wrap_ix == 0 {
                        return None;
                    }
                    let Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow {
                        row_ix,
                    }) = pane.collapsed_visible_row(visual.source_visible_ix)
                    else {
                        return None;
                    };
                    let row = pane.file_diff_inline_render_data(row_ix)?;
                    if !row.text.as_ref().contains("right_payload_") {
                        return None;
                    }
                    let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                    if !text.as_ref().contains("right_payload_") {
                        return None;
                    }
                    let (_, range) = pane
                        .diff_text_visual_source_range_for_region(visible_ix, DiffTextRegion::Inline);
                    Some((
                        visible_ix,
                        visual.source_visible_ix,
                        range.start,
                        text.to_string(),
                    ))
                })
                .expect("expected a wrapped collapsed file row continuation for copy")
        });
    assert_ne!(
        continuation_ix, source_visible_ix,
        "the copy regression needs a visual continuation row"
    );

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: selection_start,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: selection_start + expected_text.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.sync_diff_focus_to_text_selection();
            cx.notify();
        });
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected_text),
        "Ctrl-C should copy the same wrapped slice that is selected and painted"
    );

    let full_wrapped_line = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let full = pane
            .diff_text_full_line_for_region(source_visible_ix, DiffTextRegion::Inline)
            .to_string();
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .filter(|row| row.source_visible_ix == source_visible_ix)
                .count()
                > 1,
            "expected selected collapsed row to be split across wrapped visual rows"
        );
        full
    });
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: full_wrapped_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(full_wrapped_line.clone()),
        "collapsed diff copy should not insert soft-wrap newlines"
    );
    assert!(
        !full_wrapped_line.contains('\n'),
        "fixture line should be a single logical source line"
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_selection_survives_resize(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let repo_id = gitcomet_state::model::RepoId(191);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_word_wrap_resize_selection",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let marker = "right_payload_";
    let (source_visible_ix, marker_start, key_before_resize) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let (source_visible_ix, marker_start) = pane
            .diff_wrap_visible_rows
            .iter()
            .enumerate()
            .find_map(|(visible_ix, visual)| {
                if visual.wrap_ix == 0 {
                    return None;
                }
                let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                let local = text.as_ref().find(marker)?;
                let (_, range) = pane
                    .diff_text_visual_source_range_for_region(visible_ix, DiffTextRegion::Inline);
                Some((visual.source_visible_ix, range.start + local))
            })
            .expect("expected marker on a wrapped collapsed continuation row");
        (
            source_visible_ix,
            marker_start,
            pane.diff_wrap_visible_cache_key,
        )
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: marker_start,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: marker_start + marker.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.sync_diff_focus_to_text_selection();
            cx.notify();
        });
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(marker.to_string())
    );

    cx.simulate_resize(gpui::size(px(650.0), px(420.0)));
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_ne!(
            pane.diff_wrap_visible_cache_key, key_before_resize,
            "resizing should rebuild wrapped visual rows"
        );
    });

    cx.simulate_keystrokes("ctrl-c");
    let copied_after_resize = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("expected copied selection after resize");
    assert_eq!(
        copied_after_resize.replace('\n', ""),
        marker,
        "the selection should remain anchored to the same source text after wrap rows rebuild"
    );
}

#[gpui::test]
fn diff_word_wrap_columns_follow_scaled_font_metrics(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = format!("scaled {}", "wrapmetric".repeat(32));
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old line
+{long_new_line}
"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(189),
        "word_wrap_scaled_font_metrics",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "scaled font metrics fixture ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "cache_inflight={:?} file_active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);
    let default_columns = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
            .expect("expected default wrap cache key")
            .inline_columns
    });

    cx.update(|_window, app| {
        crate::app::set_app_ui_scale_percent(app, 200);
    });
    draw_and_drain_test_window(cx);
    let zoomed_columns = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
            .expect("expected zoomed wrap cache key")
            .inline_columns
    });

    assert!(
        zoomed_columns < default_columns,
        "wrap columns should decrease when the active diff font scales up (default={default_columns}, zoomed={zoomed_columns})"
    );

    cx.update(|_window, app| {
        crate::app::set_app_ui_scale_percent(app, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
        let mut appearance = crate::appearance::current(app);
        appearance.editor_font_size_px = 26;
        app.set_global(appearance);
        view.update(app, |view, cx| view.notify_font_preferences_changed(cx));
    });
    draw_and_drain_test_window(cx);
    let larger_editor_columns = cx.update(|_, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
            .expect("expected updated wrap cache after changing editor size")
            .inline_columns
    });
    assert!(
        larger_editor_columns < default_columns,
        "changing only the editor font must remeasure wrapping and gutters"
    );
}

#[gpui::test]
async fn diff_word_wrap_column_count_consistency_with_available_width(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.0), px(600.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = format!("consistency {}", "x".repeat(200));
    let unified = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n\
         index 1111111..2222222 100644\n\
         --- a/src/lib.rs\n\
         +++ b/src/lib.rs\n\
         @@ -1 +1 @@\n\
         -old line\n\
         +{long_new_line}\n"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(199),
        "wrap_column_consistency",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "wrap column consistency ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "inflight={:?} active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);

    let (inline_columns, char_width, show_line_numbers) = cx.update(|window, app| {
        let editor_font_family = crate::font_preferences::current_editor_font_family(app);
        let pane = view.read(app).main_pane.read(app);
        let cache_key = pane
            .diff_wrap_visible_cache_key
            .expect("wrap cache key must be populated after rendering with wrap on");
        let inline_columns = cache_key.inline_columns;
        let char_width = rows::diff_canvas_text_wrap_char_width(
            window,
            editor_font_family,
            crate::appearance::Appearance::default().editor_font_size_px,
        );
        let show_line_numbers = pane.diff_show_line_numbers;
        (inline_columns, char_width, show_line_numbers)
    });

    // Compute expected text-area pixel width.
    let ui_scale_percent = 100u32;
    let content_width = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        crate::view::panes::main::pane_content_width_for_layout(
            pane.last_window_size.width,
            pane.layout_sidebar_render_width,
            pane.layout_details_render_width,
            pane.layout_sidebar_collapsed,
            pane.layout_details_collapsed,
        )
    });
    let scrollbar_gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
    let available_width = (content_width - scrollbar_gutter).max(px(0.0));
    let pad = rows::diff_canvas_row_horizontal_padding(ui_scale_percent);
    let inline_text_start = if show_line_numbers {
        rows::diff_canvas_inline_text_start(ui_scale_percent)
    } else {
        pad
    };
    let text_area_px = (available_width - inline_text_start - pad).max(px(0.0));

    let expected_columns = diff_wrap_column_for_width(text_area_px, char_width);

    assert!(
        inline_columns > 1,
        "wrap columns should be > 1 (got {inline_columns})"
    );

    let diff = inline_columns.abs_diff(expected_columns);
    let max_tol = (expected_columns / 10).max(2);
    assert!(
        diff <= max_tol,
        "inline_columns ({inline_columns}) differs from expected ({expected_columns}) \
         by {diff}, max acceptable {max_tol} \
         (text_area_px={text_area_px:?}, char_width={char_width:?})"
    );

    let occupied_px = char_width * inline_columns as f32;
    assert!(
        occupied_px <= text_area_px + char_width,
        "wrapped text width ({occupied_px:?}) should not exceed text area \
         ({text_area_px:?}) by more than one char width"
    );

    let unused = (text_area_px - occupied_px).max(px(0.0));
    assert!(
        unused <= char_width * 3.0,
        "unused space ({unused:?}) should be < 3 chars ({:?}) — \
         lines break too early if larger",
        char_width * 3.0
    );
}

fn diff_wrap_column_for_width(width: Pixels, char_width: Pixels) -> usize {
    let cw = f32::from(char_width.max(px(1.0)));
    ((f32::from(width.max(px(0.0))) / cw).floor() as usize).max(1)
}

/// Display columns a wrapped segment occupies, matching the tab expansion the
/// wrap algorithm uses (`DIFF_WRAP_TAB_EXPANDED_COLUMNS`).
fn wrap_display_columns(text: &str) -> usize {
    text.chars().map(|ch| if ch == '\t' { 4 } else { 1 }).sum()
}

/// Greedy word wrap must emit *maximal* segments: a non-final segment may only
/// stop short of the column budget when the next word could not have fit.
///
/// This is the invariant that "lines break too early" violates. It is stated in
/// columns rather than pixels on purpose — `#[gpui::test]` runs on gpui's
/// `NoopTextSystem`, where every glyph advances an identical 0.6em regardless of
/// font, so pixel measurements taken in a test cannot distinguish fonts at all.
#[gpui::test]
async fn diff_word_wrap_segments_are_maximal_for_their_column_budget(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    // Short words only, so every break lands on whitespace. A segment that
    // stops early here is a genuine wrap bug — unlike a single long token,
    // which correctly gets pushed to the next row and leaves the previous one
    // partly empty.
    let long_new_line = "let value = compute(alpha, beta, gamma) + delta * epsilon - zeta / eta; "
        .repeat(6)
        .trim_end()
        .to_string();
    let unified = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n\
         index 1111111..2222222 100644\n\
         --- a/src/lib.rs\n\
         +++ b/src/lib.rs\n\
         @@ -1 +1 @@\n\
         -old line\n\
         +{long_new_line}\n"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(200),
        "wrap_segments_maximal",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "wrap segments ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "inflight={:?} active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let wrap_columns = pane
            .diff_wrap_visible_cache_key
            .expect("wrap cache key must be populated after rendering with wrap on")
            .inline_columns;
        assert!(
            wrap_columns > 8,
            "degenerate wrap budget ({wrap_columns} columns)"
        );

        let visible_len = pane.diff_visible_len();
        let mut checked = 0usize;
        for visible_ix in 0..visible_len {
            let rows = &pane.diff_wrap_visible_rows;
            let (Some(row), Some(next_row)) = (rows.get(visible_ix), rows.get(visible_ix + 1))
            else {
                continue;
            };
            // Only non-final segments of a wrapped source row are constrained;
            // the last segment is free to be short.
            if next_row.source_visible_ix != row.source_visible_ix {
                continue;
            }

            let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
            let next_text = pane.diff_text_line_for_region(visible_ix + 1, DiffTextRegion::Inline);
            if text.is_empty() || next_text.is_empty() {
                continue;
            }

            let used = wrap_display_columns(text.as_ref());
            // What appending the next row's first word to this row would have
            // cost, including any whitespace the row opens with (a row starts
            // with whitespace when the previous word ended exactly on the
            // column boundary).
            let next_word: String = {
                let next_text = next_text.as_ref();
                let leading_ws = next_text.len() - next_text.trim_start().len();
                let word = next_text
                    .trim_start()
                    .chars()
                    .take_while(|ch| !ch.is_whitespace());
                next_text[..leading_ws].chars().chain(word).collect()
            };
            let next_word_columns = wrap_display_columns(&next_word);

            assert!(
                used <= wrap_columns,
                "visible_ix={visible_ix}: segment overflows its budget \
                 ({used} columns of {wrap_columns}). text={text:?}"
            );
            assert!(
                used + next_word_columns > wrap_columns,
                "visible_ix={visible_ix}: line broke too early — {used} of \
                 {wrap_columns} columns used and the next word {next_word:?} \
                 ({next_word_columns} columns) would still have fit. \
                 text={text:?}"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "expected at least one source row that wraps to multiple visual rows"
        );
    });
}

/// Wrap columns must be measured in the font the rows are *painted* in.
///
/// `MainPane::diff_wrap_columns` runs while the diff pane is building its
/// element tree, before the rows container pushes
/// `.font_family(editor_font_family)` onto the window text style stack. The
/// ambient style there is a proportional UI font, and the wrap width sample is
/// `"WWWWWWWWWW"` — the widest glyph in a proportional face (IBM Plex Sans `W`
/// is 0.891em against Lilex's uniform 0.600em), which overestimated the column
/// width by ~1.5x and wrapped every line at roughly two thirds of the width it
/// actually had.
///
/// The assertion is on font *identity*, not measured width: gpui's test
/// `NoopTextSystem` maps every font descriptor to the same `FontId` and every
/// glyph to the same advance, so no width-based test can catch this.
#[gpui::test]
async fn diff_word_wrap_columns_are_measured_in_the_editor_font(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = format!("fonttest {}", "abc def ghi ".repeat(30));
    let unified = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n\
         index 1111111..2222222 100644\n\
         --- a/src/lib.rs\n\
         +++ b/src/lib.rs\n\
         @@ -1 +1 @@\n\
         -old line\n\
         +{long_new_line}\n"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(201),
        "wrap_measure_font",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "wrap measure font ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "inflight={:?} active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let editor_font_family = crate::font_preferences::current_editor_font_family(app);
        let main_pane = view.read(app).main_pane.clone();
        let measured = main_pane.update(app, |pane, cx| pane.diff_wrap_measure_font_family(cx));

        assert_eq!(
            measured.as_ref(),
            editor_font_family.as_str(),
            "wrap columns must be measured in the editor font the rows are painted in"
        );
        // The trap: outside the rows container the ambient text style is never
        // the editor font, so measuring against `window.text_style()` silently
        // measures the wrong face.
        assert_ne!(
            window.text_style().font_family.as_ref(),
            editor_font_family.as_str(),
            "ambient text style unexpectedly matches the editor font — this test \
             no longer guards anything"
        );
    });
}

#[gpui::test]
fn reveal_whitespace_chars_marks_file_diff_paint_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let path = PathBuf::from("src/lib.rs");
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-alpha
+a b\t
"
    .to_string();
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(188),
        "reveal_whitespace_file_diff",
        path,
        unified,
        "alpha\n".to_string(),
        "a b\t\n".to_string(),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.set_diff_word_wrap(false, cx);
            this.set_diff_reveal_whitespace_chars(true, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "file diff ready for whitespace reveal",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref().contains("a b\t")
                })
        },
        |pane| {
            format!(
                "cache_inflight={:?} file_active={} inline_rows={:?}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| format!("{:?}:{}", line.kind, line.text.as_ref()))
                    .collect::<Vec<_>>(),
            )
        },
    );

    let visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (0..pane.diff_visible_len())
            .find(|&visible_ix| {
                let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return false;
                };
                pane.file_diff_inline_row(inline_ix).is_some_and(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref().contains("a b\t")
                })
            })
            .expect("expected visible added row with whitespace")
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });
    cx.run_until_parked();

    let record = cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        let _ = window.draw(app);
        rows::diff_paint_log_for_tests()
            .into_iter()
            .find(|record| {
                record.visible_ix == visible_ix && record.region == DiffTextRegion::Inline
            })
            .expect("expected paint record for visible whitespace row")
    });
    assert_eq!(record.text.as_ref(), "a·b→↵");
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
        "regular file diff collapsed mode activates collapsed projection",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.patch_diff_split_row_len() > 0
                && pane.file_diff_split_row_len() > 0
        },
        |pane| {
            format!(
                "content_mode={:?} file_diff_active={} collapsed_active={} inflight={:?} cache_target={:?} patch_rows={} file_rows={}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.is_collapsed_diff_projection_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target,
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

fn set_diff_text_selection_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    start_visible_ix: usize,
    end_visible_ix: usize,
    region: DiffTextRegion,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: start_visible_ix,
                    region,
                    offset: 0,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: end_visible_ix,
                    region,
                    offset: 1,
                });
                pane.diff_selection_anchor = Some(end_visible_ix);
                pane.diff_selection_range = None;
                // Seeded selections must own the window like real ones, or the
                // arbitration reaps them on the next global write. Adopted last,
                // once the state it describes is fully seeded.
                pane.diff_text_selection_owner.adopt(window, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
}

/// Loads `fixture` as a Full-mode file diff in `diff_view` with nothing
/// selected, and returns its navigation stops once there are `expected_stops`.
fn activate_full_diff_nav_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
    (unified, old_text, new_text): (String, String, String),
    expected_stops: usize,
) -> Vec<usize> {
    let target = push_regular_diff_content_mode_state(
        cx,
        view,
        repo_id,
        fixture_name,
        PathBuf::from("src/lib.rs"),
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        view,
        "full diff fixture activates file diff view",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight.is_some(),
                pane.file_diff_cache_target.clone(),
                pane.diff_nav_entries(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_content_mode = DiffContentMode::Full;
                pane.diff_view = diff_view;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_autoscroll_pending = false;
                pane.clear_diff_text_selection();
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "full diff has one navigation stop per change block",
        |pane| {
            pane.diff_content_mode == DiffContentMode::Full
                && pane.diff_view == diff_view
                && pane.diff_nav_entries().len() == expected_stops
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_nav_entries(),
            )
        },
    );
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_nav_entries())
}

fn press_and_assert_anchor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    keystroke: &str,
    expected: usize,
    message: &str,
) {
    cx.simulate_keystrokes(keystroke);
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_selection_anchor, Some(expected), "{message}");
        // Anchor only: the landed row gets no selection wash.
        assert_eq!(pane.diff_selection_range, None, "{message}");
    });
}

fn assert_full_diff_change_shortcuts_visit_each_change_block(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    let (e0, e1, e2) = (entries[0], entries[1], entries[2]);
    assert!(
        e1 > e0 + 1,
        "fixture's first change block should span several rows in {diff_view:?}: {entries:?}"
    );

    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e0,
        &format!("first F3 should land on the first change block in Full diff {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e1,
        &format!("second F3 should skip the rest of the block in Full diff {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 should move back one change block in Full diff {diff_view:?}"),
    );

    // From the middle of a block, F2 goes to that block's start and F3 to the next block.
    set_diff_row_selection_for_test(cx, view, e0 + 1, (e0 + 1, e0 + 1));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 mid-block should go to the block's start in Full diff {diff_view:?}"),
    );
    set_diff_row_selection_for_test(cx, view, e0 + 1, (e0 + 1, e0 + 1));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e1,
        &format!("F3 mid-block should go to the next block in Full diff {diff_view:?}"),
    );

    set_diff_row_selection_for_test(cx, view, e0, (e0, e1));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e2,
        &format!("F3 should continue after the selected row range in Full diff {diff_view:?}"),
    );

    set_diff_row_selection_for_test(cx, view, e2, (e1, e2));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 should continue before the selected row range in Full diff {diff_view:?}"),
    );

    let text_region = match diff_view {
        DiffViewMode::Inline => DiffTextRegion::Inline,
        DiffViewMode::Split => DiffTextRegion::SplitLeft,
    };
    set_diff_text_selection_for_test(cx, view, e0, e0 + 1, text_region);
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e1,
        &format!("F3 should continue after the selected text range in Full diff {diff_view:?}"),
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    set_diff_text_selection_for_test(cx, view, e1, e2, text_region);
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 should continue before the selected text range in Full diff {diff_view:?}"),
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    // The last stop does not wrap around.
    set_diff_row_selection_for_test(cx, view, e2, (e2, e2));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e2,
        &format!("F3 on the last block should stay put in Full diff {diff_view:?}"),
    );
}

#[gpui::test]
fn full_diff_inline_change_shortcuts_visit_each_change_block(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_change_shortcuts_visit_each_change_block(
        cx,
        &view,
        gitcomet_state::model::RepoId(70601),
        "full_diff_inline_block_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_change_shortcuts_visit_each_change_block(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_change_shortcuts_visit_each_change_block(
        cx,
        &view,
        gitcomet_state::model::RepoId(70602),
        "full_diff_split_block_nav",
        DiffViewMode::Split,
    );
}

/// The real app binds F2/F3 globally *and* observes keystrokes for a diff
/// shortcut fallback; a press must still move exactly one block.
fn assert_full_diff_app_keys_move_one_block_per_press(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    cx.update(|_window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });

    focus_diff_panel(cx, view);
    for (keystroke, expected, message) in [
        ("f3", entries[0], "first F3"),
        ("f3", entries[1], "second F3"),
        ("f2", entries[0], "F2"),
        ("f7", entries[1], "F7"),
        ("shift-f7", entries[0], "Shift+F7"),
    ] {
        cx.simulate_keystrokes(keystroke);
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            assert_eq!(
                view.read(app).main_pane.read(app).diff_selection_anchor,
                Some(expected),
                "{message} should move exactly one change block in {diff_view:?}"
            );
        });
    }
}

#[gpui::test]
fn full_diff_inline_app_keys_move_one_block_per_press(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_app_keys_move_one_block_per_press(
        cx,
        &view,
        gitcomet_state::model::RepoId(70637),
        "full_diff_inline_app_keys_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_app_keys_move_one_block_per_press(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_app_keys_move_one_block_per_press(
        cx,
        &view,
        gitcomet_state::model::RepoId(70638),
        "full_diff_split_app_keys_nav",
        DiffViewMode::Split,
    );
}

fn assert_full_diff_f3_without_selection_reaches_block_at_first_row(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_first_row_change_fixture_texts(),
        2,
    );
    assert_eq!(
        entries[0], 0,
        "fixture's first change block should start at row 0"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_focus_visible_range(), None);
        assert_eq!(
            pane.diff_nav_next_target_ix(&entries),
            Some(0),
            "the Next button should be enabled for a block at row 0 with nothing selected"
        );
        assert_eq!(pane.diff_nav_prev_target_ix(&entries), None);
    });

    // F2 with nothing focused has no target and must not seed a focus that
    // would make the next F3 skip row 0.
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            None
        );
    });

    press_and_assert_anchor(
        cx,
        view,
        "f3",
        0,
        &format!("F3 with nothing selected should reach the block at row 0 in {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        entries[1],
        &format!("F3 should then reach the next block in {diff_view:?}"),
    );
}

#[gpui::test]
fn full_diff_inline_f3_without_selection_reaches_block_at_first_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_f3_without_selection_reaches_block_at_first_row(
        cx,
        &view,
        gitcomet_state::model::RepoId(70608),
        "full_diff_inline_first_row_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_f3_without_selection_reaches_block_at_first_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_f3_without_selection_reaches_block_at_first_row(
        cx,
        &view,
        gitcomet_state::model::RepoId(70609),
        "full_diff_split_first_row_nav",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_diff_ignore_whitespace_drops_whitespace_only_block_stop(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let old_text = "alpha\nold one\nmiddle\n  keep\nomega\n".to_string();
    let new_text = "alpha\nnew one\nmiddle\nkeep\nomega\n".to_string();
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,5 @@
 alpha
-old one
+new one
 middle
-  keep
+keep
 omega
"
    .to_string();
    let shown = activate_full_diff_nav_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(70610),
        "full_diff_ignore_whitespace_nav",
        DiffViewMode::Split,
        (unified, old_text, new_text),
        2,
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.set_diff_whitespace_mode(DiffWhitespaceMode::Ignore, cx);
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "ignore-whitespace drops the whitespace-only block stop",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.diff_nav_entries() == vec![shown[0]]
        },
        |pane| {
            (
                pane.diff_whitespace_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight.is_some(),
                pane.diff_nav_entries(),
            )
        },
    );
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

/// Focused change block marks painted on a fresh draw, by row then column.
fn painted_focused_change_block_marks(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Vec<rows::FocusedChangeBlockPaint> {
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |_pane, cx| cx.notify());
        rows::clear_focused_change_block_paint_log_for_tests();
        let _ = window.draw(app);
        let mut painted = rows::focused_change_block_paint_log_for_tests();
        painted.sort_by_key(|mark| (mark.visible_ix, mark.region as u8));
        painted.dedup();
        painted
    })
}

/// Asserts the focused block covers `expected` rows, each column painting the
/// bar with the outline closed on the first and last rows, and returns the
/// outline side each column painted, row by row.
fn assert_focused_change_block_bar(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    expected: std::ops::Range<usize>,
    diff_view: DiffViewMode,
    message: &str,
) -> Vec<Vec<Option<DiffChangeSide>>> {
    assert_eq!(
        focused_change_block_rows(cx, view),
        expected.clone().collect::<Vec<_>>(),
        "{message}: focused block rows in {diff_view:?}"
    );
    let regions: &[DiffTextRegion] = match diff_view {
        DiffViewMode::Inline => &[DiffTextRegion::Inline],
        DiffViewMode::Split => &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight],
    };
    let painted = painted_focused_change_block_marks(cx, view);
    let painted_cells = painted
        .iter()
        .map(|mark| (mark.visible_ix, mark.region, mark.top, mark.bottom))
        .collect::<Vec<_>>();
    let expected_cells = expected
        .clone()
        .flat_map(|visible_ix| {
            let top = visible_ix == expected.start;
            let bottom = visible_ix + 1 == expected.end;
            regions
                .iter()
                .map(move |&region| (visible_ix, region, top, bottom))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        painted_cells, expected_cells,
        "{message}: painted marks in {diff_view:?}"
    );
    regions
        .iter()
        .map(|&region| {
            painted
                .iter()
                .filter(|mark| mark.region == region)
                .map(|mark| mark.outline)
                .collect()
        })
        .collect()
}

fn assert_full_diff_focused_change_block_shows_accent_bar(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    let (e0, e1, e2) = (entries[0], entries[1], entries[2]);
    assert!(
        painted_focused_change_block_marks(cx, view).is_empty(),
        "no block is focused before navigating in {diff_view:?}"
    );

    // One unchanged row ("middle one", "middle two") separates the blocks.
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let outlines =
        assert_focused_change_block_bar(cx, view, e0..e1 - 1, diff_view, "F3 onto block one");
    // Every block modifies lines: inline alternates `-`/`+` rows, each in its
    // own colour; split outlines the old column red and the new one green.
    use DiffChangeSide::{Added, Removed};
    let expected_outlines = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Removed), Some(Added), Some(Removed), Some(Added)]],
        DiffViewMode::Split => vec![vec![Some(Removed); 2], vec![Some(Added); 2]],
    };
    assert_eq!(
        outlines, expected_outlines,
        "outline colours in {diff_view:?}"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_focused_change_block_bar(cx, view, e1..e2 - 1, diff_view, "F3 onto block two");

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_focused_change_block_bar(cx, view, e0..e1 - 1, diff_view, "F2 back to block one");

    // Moving the selection elsewhere takes the marks away.
    set_diff_row_selection_for_test(cx, view, 0, (0, 0));
    assert!(
        painted_focused_change_block_marks(cx, view).is_empty(),
        "the marks should follow the selection off the block in {diff_view:?}"
    );
}

#[gpui::test]
fn full_diff_inline_focused_change_block_shows_accent_bar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_focused_change_block_shows_accent_bar(
        cx,
        &view,
        gitcomet_state::model::RepoId(70634),
        "full_diff_inline_focus_bar",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_focused_change_block_shows_accent_bar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_focused_change_block_shows_accent_bar(
        cx,
        &view,
        gitcomet_state::model::RepoId(70635),
        "full_diff_split_focus_bar",
        DiffViewMode::Split,
    );
}

fn assert_pure_change_blocks_outline_only_their_side(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    // An inserted line, then a deleted one.
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,4 @@
 a
+NEW
 b
-c
 d
"
    .to_string();
    let fixture = (
        unified,
        "a\nb\nc\nd\n".to_string(),
        "a\nNEW\nb\nd\n".to_string(),
    );
    let entries =
        activate_full_diff_nav_fixture(cx, view, repo_id, fixture_name, diff_view, fixture, 2);

    use DiffChangeSide::{Added, Removed};
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let added = assert_focused_change_block_bar(
        cx,
        view,
        entries[0]..entries[0] + 1,
        diff_view,
        "the inserted line",
    );
    let expected_added = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Added)]],
        // Nothing was removed, so the old column's empty filler gets no outline.
        DiffViewMode::Split => vec![vec![None], vec![Some(Added)]],
    };
    assert_eq!(
        added, expected_added,
        "an addition outlines green in {diff_view:?}"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let removed = assert_focused_change_block_bar(
        cx,
        view,
        entries[1]..entries[1] + 1,
        diff_view,
        "the deleted line",
    );
    let expected_removed = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Removed)]],
        DiffViewMode::Split => vec![vec![Some(Removed)], vec![None]],
    };
    assert_eq!(
        removed, expected_removed,
        "a removal outlines red in {diff_view:?}"
    );
}

#[gpui::test]
fn full_diff_inline_pure_change_blocks_outline_only_their_side(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_pure_change_blocks_outline_only_their_side(
        cx,
        &view,
        gitcomet_state::model::RepoId(70639),
        "full_diff_inline_pure_outline",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_pure_change_blocks_outline_only_their_side(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_pure_change_blocks_outline_only_their_side(
        cx,
        &view,
        gitcomet_state::model::RepoId(70640),
        "full_diff_split_pure_outline",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_diff_whitespace_mode_change_does_not_restore_focused_block_on_row_click(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 alpha
-  keep
+keep
 omega
"
    .to_string();
    let entries = activate_full_diff_nav_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(70641),
        "full_diff_whitespace_focus_bar",
        DiffViewMode::Split,
        (
            unified,
            "alpha\n  keep\nomega\n".to_string(),
            "alpha\nkeep\nomega\n".to_string(),
        ),
        1,
    );
    let row = entries[0];
    focus_diff_panel(cx, &view);
    press_and_assert_anchor(cx, &view, "f3", row, "F3 onto the whitespace-only block");
    assert_focused_change_block_bar(
        cx,
        &view,
        row..row + 1,
        DiffViewMode::Split,
        "the whitespace-only block before ignoring whitespace",
    );
    let visible_len = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            let visible_len = pane.diff_visible_len();
            pane.set_diff_whitespace_mode(DiffWhitespaceMode::Ignore, cx);
            visible_len
        })
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "the whitespace-only block becomes context",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.diff_visible_len() == visible_len
                && pane.diff_nav_entries().is_empty()
        },
        |pane| {
            (
                pane.file_diff_cache_inflight.is_some(),
                pane.diff_visible_len(),
                pane.diff_nav_entries(),
            )
        },
    );

    // Split mode keeps the same row indices and row count. Clicking the old
    // anchor must not revive the block captured before the mode switch.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            assert_eq!(pane.diff_selection_anchor, None);
            let row_ix = pane.diff_mapped_ix_for_visible_ix(row).unwrap();
            assert!(!pane.file_diff_row_is_change(row_ix));
            pane.handle_patch_row_click(row, DiffClickKind::Line, false);
            assert_eq!(pane.diff_selection_anchor, Some(row));
            cx.notify();
        });
    });
    assert!(
        painted_focused_change_block_marks(cx, &view).is_empty(),
        "clicking the former whitespace-only block must not paint its old outline"
    );
    assert!(focused_change_block_rows(cx, &view).is_empty());
}

#[gpui::test]
fn focused_change_block_bar_hides_after_the_layout_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let entries = activate_full_diff_nav_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(70636),
        "full_diff_focus_bar_relayout",
        DiffViewMode::Inline,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_eq!(
        focused_change_block_rows(cx, &view).first().copied(),
        Some(entries[0])
    );

    // Inline and split number their rows differently; the captured rows no
    // longer describe the block, even though the anchor index is unchanged.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(entries[0]),
            "fixture should keep the anchor so only the layout check can hide the bar"
        );
    });
    assert!(
        focused_change_block_rows(cx, &view).is_empty(),
        "a relayout should hide the bar rather than mark whatever rows now sit there"
    );
}

fn assert_full_diff_word_wrap_change_shortcuts_skip_continuations(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    cx.simulate_resize(gpui::size(px(760.0), px(420.0)));
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_full_diff_word_wrap_navigation_fixture_texts();
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
        "full diff word-wrap fixture activates file diff view",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight.is_some(),
                pane.file_diff_cache_target.clone(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = diff_view;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_autoscroll_pending = false;
                pane.clear_diff_text_selection();
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "full diff word-wrap navigation entries are visual rows",
        |pane| {
            let entries = pane.diff_nav_entries();
            pane.diff_content_mode == DiffContentMode::Full
                && pane.diff_view == diff_view
                && pane.diff_word_wrap
                && pane.diff_wrap_visible_cache_key.is_some()
                && pane
                    .diff_wrap_visible_rows
                    .iter()
                    .any(|row| row.wrap_ix > 0)
                && entries.len() >= 2
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_wrap_visible_cache_key,
                pane.diff_nav_entries(),
            )
        },
    );

    let (first_entry, second_entry) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let entries = pane.diff_nav_entries();
        let first_entry = entries[0];
        let second_entry = entries[1];
        let first_row = pane.diff_wrap_visible_rows[first_entry];
        let second_row = pane.diff_wrap_visible_rows[second_entry];
        assert_eq!(
            first_row.wrap_ix, 0,
            "first navigation entry should be the first visual row of its change block"
        );
        assert_eq!(
            second_row.wrap_ix, 0,
            "second navigation entry should be the first visual row of its change block"
        );
        assert!(
            second_row.source_visible_ix > first_row.source_visible_ix,
            "second navigation entry should advance to the next change block"
        );
        let has_wrapped_continuation_between_entries = pane
            .diff_wrap_visible_rows
            .iter()
            .enumerate()
            .any(|(visible_ix, row)| {
                visible_ix > first_entry
                    && visible_ix < second_entry
                    && row.source_visible_ix == first_row.source_visible_ix
                    && row.wrap_ix > 0
            });
        assert!(
            has_wrapped_continuation_between_entries,
            "fixture should put wrapped continuation rows between the first two navigation entries; entries={entries:?}, first_row={first_row:?}, second_row={second_row:?}"
        );
        (first_entry, second_entry)
    });

    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(first_entry),
            "first F3 should select the first change block in wrapped Full diff {diff_view:?}"
        );
    });
    let expected_bar_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let rows = &pane.diff_wrap_visible_rows;
        // One unchanged row ("middle") separates the two blocks.
        let block = rows[first_entry].source_visible_ix..rows[second_entry].source_visible_ix - 1;
        let expected = (0..rows.len())
            .filter(|&ix| block.contains(&rows[ix].source_visible_ix))
            .collect::<Vec<_>>();
        assert!(
            expected.iter().any(|&ix| rows[ix].wrap_ix > 0),
            "fixture's first block should wrap in {diff_view:?}"
        );
        expected
    });
    assert_eq!(
        focused_change_block_rows(cx, view),
        expected_bar_rows,
        "the focus bar should cover the block's wrapped continuation rows in {diff_view:?}"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(second_entry),
            "second F3 should skip wrap continuations in wrapped Full diff {diff_view:?}"
        );
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(first_entry),
            "F2 should move back to the previous change block in wrapped Full diff {diff_view:?}"
        );
    });
}

#[gpui::test]
fn full_diff_word_wrap_inline_change_shortcuts_skip_continuation_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_word_wrap_change_shortcuts_skip_continuations(
        cx,
        &view,
        gitcomet_state::model::RepoId(70603),
        "full_diff_word_wrap_inline_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_word_wrap_inline_change_shortcuts_map_provider_rows_through_visible_map(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.simulate_resize(gpui::size(px(760.0), px(420.0)));
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_full_diff_word_wrap_navigation_fixture_texts();
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(70607),
        "full_diff_word_wrap_inline_visible_map_nav",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "full diff word-wrap visible-map fixture activates file diff view",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight.is_some(),
                pane.file_diff_cache_target.clone(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_autoscroll_pending = false;
                pane.clear_diff_text_selection();
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "full diff word-wrap visible-map fixture has wrapped inline rows",
        |pane| {
            pane.diff_content_mode == DiffContentMode::Full
                && pane.diff_view == DiffViewMode::Inline
                && pane.diff_word_wrap
                && pane.diff_wrap_visible_cache_key.is_some()
                && pane
                    .diff_wrap_visible_rows
                    .iter()
                    .any(|row| row.wrap_ix > 0)
                && pane
                    .file_diff_inline_row_provider
                    .as_ref()
                    .is_some_and(|provider| !provider.change_blocks().is_empty())
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_wrap_visible_cache_key,
                pane.diff_nav_entries(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let inline_len = pane.file_diff_inline_row_len();
                assert!(inline_len > 1, "fixture should expose file inline rows");
                let mut hidden = vec![false; inline_len];
                hidden[0] = true;
                pane.diff_visible_inline_map =
                    Some(PatchInlineVisibleMap::from_hidden_flags(hidden.as_slice()));
                pane.diff_visible_indices.clear();
                pane.diff_wrap_visible_rows.clear();
                pane.diff_wrap_visible_cache_key = None;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.clear_diff_text_selection();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let provider = pane
            .file_diff_inline_row_provider
            .as_ref()
            .expect("fixture should use the paged inline file provider");
        let first_changed_provider_ix = provider
            .change_blocks()
            .first()
            .map(|block| block.start)
            .expect("fixture should contain a changed inline row");
        let visible_map = pane
            .diff_visible_inline_map
            .as_ref()
            .expect("test should install a non-identity inline visible map");
        let first_changed_source_visible_ix = visible_map
            .visible_ix_for_src_ix(first_changed_provider_ix)
            .expect("changed provider row should remain visible");
        assert_ne!(
            first_changed_provider_ix, first_changed_source_visible_ix,
            "fixture should exercise a non-identity provider-to-visible mapping"
        );

        let entries = pane.diff_nav_entries();
        let expected_first_entry =
            pane.diff_visual_ix_for_source_visible_ix(first_changed_source_visible_ix);
        assert_eq!(
            entries.first().copied(),
            Some(expected_first_entry),
            "wrapped Full inline diff navigation should convert provider rows through the visible map"
        );
        assert_eq!(
            pane.diff_wrap_visible_rows
                .get(expected_first_entry)
                .map(|row| row.source_visible_ix),
            Some(first_changed_source_visible_ix),
            "navigation should target the first wrapped visual row for the mapped source-visible row"
        );
    });
}

#[gpui::test]
fn full_diff_word_wrap_split_change_shortcuts_skip_continuation_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_word_wrap_change_shortcuts_skip_continuations(
        cx,
        &view,
        gitcomet_state::model::RepoId(70604),
        "full_diff_word_wrap_split_nav",
        DiffViewMode::Split,
    );
}

fn assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_block_starts(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    cx.simulate_resize(gpui::size(px(760.0), px(420.0)));
    let (unified, old_text, new_text) = build_collapsed_diff_word_wrap_navigation_fixture_texts();
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

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.clear_diff_text_selection();
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff word-wrap navigation entries are visual rows",
        |pane| {
            pane.diff_view == diff_view
                && pane.diff_word_wrap
                && pane.is_collapsed_diff_projection_active()
                && pane.diff_wrap_visible_cache_key.is_some()
                && pane.collapsed_diff_hunk_visible_indices.len() >= 2
                && pane.diff_nav_entries().len() >= 2
        },
        |pane| {
            (
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_wrap_visible_cache_key,
                pane.collapsed_diff_hunk_visible_indices.clone(),
                pane.diff_nav_entries(),
            )
        },
    );

    let (first_entry, second_entry) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        // Each hunk holds one change; its stop is the first changed row, not
        // the hunk's header or leading context.
        let raw_first =
            collapsed_first_changed_source_visible_ix(pane, pane.collapsed_diff_hunk_visible_indices[0]);
        let raw_second =
            collapsed_first_changed_source_visible_ix(pane, pane.collapsed_diff_hunk_visible_indices[1]);
        let expected_first = pane.diff_visual_ix_for_source_visible_ix(raw_first);
        let expected_second = pane.diff_visual_ix_for_source_visible_ix(raw_second);
        assert_eq!(
            pane.diff_nav_entries(),
            vec![expected_first, expected_second],
            "collapsed nav entries should be the mapped visual rows of each block's first changed row"
        );
        assert_ne!(
            expected_second, raw_second,
            "fixture should expose the stale source-visible second block index regression"
        );
        let second_row = pane.diff_wrap_visible_rows[expected_second];
        assert_eq!(second_row.source_visible_ix, raw_second);
        assert_eq!(
            second_row.wrap_ix, 0,
            "collapsed navigation should land on the first visual row of the block's first row"
        );
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .take(expected_second)
                .any(|row| row.wrap_ix > 0),
            "fixture should include wrapped visual rows before the second block"
        );
        (expected_first, expected_second)
    });

    set_diff_row_selection_for_test(cx, view, first_entry, (first_entry, first_entry));
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(second_entry),
            "F3 should navigate to the next collapsed change block's visual row in {diff_view:?}"
        );
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(first_entry),
            "F2 should navigate back to the previous collapsed change block's visual row in {diff_view:?}"
        );
    });
}

#[gpui::test]
fn collapsed_diff_word_wrap_inline_change_shortcuts_use_visual_block_starts(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_block_starts(
        cx,
        &view,
        gitcomet_state::model::RepoId(70605),
        "collapsed_diff_word_wrap_inline_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_split_change_shortcuts_use_visual_block_starts(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_block_starts(
        cx,
        &view,
        gitcomet_state::model::RepoId(70606),
        "collapsed_diff_word_wrap_split_nav",
        DiffViewMode::Split,
    );
}

/// One git hunk (lines 17-27) holding two change runs, at lines 20 and 24.
fn build_collapsed_diff_two_runs_in_one_hunk_fixture_texts() -> (String, String, String) {
    let changes = [20usize, 24];
    let line_text = |line: usize, side: &str| {
        if changes.contains(&line) {
            format!("{side} value {line}")
        } else {
            format!("line {line}")
        }
    };
    let old_lines = (1..=40)
        .map(|line| line_text(line, "old"))
        .collect::<Vec<_>>();
    let new_lines = (1..=40)
        .map(|line| line_text(line, "new"))
        .collect::<Vec<_>>();
    let mut unified = String::from(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -17,11 +17,11 @@
",
    );
    for line in 17..=27 {
        if changes.contains(&line) {
            unified.push_str(&format!(
                "-{}\n+{}\n",
                old_lines[line - 1],
                new_lines[line - 1]
            ));
        } else {
            unified.push_str(&format!(" {}\n", old_lines[line - 1]));
        }
    }
    (
        unified,
        format!("{}\n", old_lines.join("\n")),
        format!("{}\n", new_lines.join("\n")),
    )
}

fn assert_collapsed_diff_hunk_with_two_change_runs_has_two_stops(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_two_runs_in_one_hunk_fixture_texts();
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

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed hunk with two change runs has two stops",
        |pane| {
            pane.diff_view == diff_view
                && pane.collapsed_diff_hunk_visible_indices.len() == 1
                && pane.diff_nav_entries().len() == 2
        },
        |pane| {
            (
                pane.diff_view,
                pane.collapsed_diff_hunk_visible_indices.clone(),
                pane.diff_nav_entries(),
            )
        },
    );

    // Lines 20 and 24: split rows 19 and 23; inline adds a `+` row after 20.
    let (first_row, second_row) = match diff_view {
        DiffViewMode::Inline => (19, 24),
        DiffViewMode::Split => (19, 23),
    };
    let entries = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let entries = pane.diff_nav_entries();
        assert_eq!(
            entries,
            vec![
                collapsed_file_row_visible_ix(pane, first_row),
                collapsed_file_row_visible_ix(pane, second_row),
            ],
            "each change run in the hunk should be its own stop, on its first changed row"
        );
        entries
    });

    set_diff_row_selection_for_test(cx, view, entries[0], (entries[0], entries[0]));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        entries[1],
        &format!("F3 should stop on the second run inside the same hunk in {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        entries[0],
        &format!("F2 should return to the first run in {diff_view:?}"),
    );
    // Line 20 is one split row, or a `-`/`+` pair inline.
    let run_len = match diff_view {
        DiffViewMode::Inline => 2,
        DiffViewMode::Split => 1,
    };
    let outlines = assert_focused_change_block_bar(
        cx,
        view,
        entries[0]..entries[0] + run_len,
        diff_view,
        "the marks cover only the focused run, not the whole hunk",
    );
    use DiffChangeSide::{Added, Removed};
    let expected_outlines = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Removed), Some(Added)]],
        DiffViewMode::Split => vec![vec![Some(Removed)], vec![Some(Added)]],
    };
    assert_eq!(
        outlines, expected_outlines,
        "outline colours in {diff_view:?}"
    );
}

#[gpui::test]
fn collapsed_diff_inline_hunk_with_two_change_runs_has_two_stops(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_hunk_with_two_change_runs_has_two_stops(
        cx,
        &view,
        gitcomet_state::model::RepoId(70611),
        "collapsed_inline_two_runs_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_hunk_with_two_change_runs_has_two_stops(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_hunk_with_two_change_runs_has_two_stops(
        cx,
        &view,
        gitcomet_state::model::RepoId(70630),
        "collapsed_split_two_runs_nav",
        DiffViewMode::Split,
    );
}

fn assert_patch_diff_change_stops_land_on_first_changed_rows(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    // Two runs in one hunk of a.rs, then a last-line edit without trailing
    // newlines in b.rs, whose markers sit between `-` and `+`.
    let unified = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,7 +1,7 @@
 one
-two
+TWO
 three
 four
 five
-six
+SIX
 seven
diff --git a/src/b.rs b/src/b.rs
index 3333333..4444444 100644
--- a/src/b.rs
+++ b/src/b.rs
@@ -1 +1 @@
-last
\\ No newline at end of file
+LAST
\\ No newline at end of file
"
    .to_string();
    let target =
        push_raw_patch_diff_state_with_rev(cx, view, repo_id, fixture_name, unified, 1, true);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "whole-commit patch diff has one stop per change block",
        |pane| {
            pane.rendered_diff_target() == Some(&target)
                && pane.diff_view == diff_view
                && !pane.is_file_diff_view_active()
                && pane.patch_diff_row_len() > 0
                && pane.diff_nav_entries().len() == 3
        },
        |pane| {
            (
                pane.rendered_diff_target().cloned(),
                pane.diff_view,
                pane.patch_diff_row_len(),
                pane.diff_nav_entries(),
            )
        },
    );

    let region = match diff_view {
        DiffViewMode::Inline => DiffTextRegion::Inline,
        DiffViewMode::Split => DiffTextRegion::SplitLeft,
    };
    let entries = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let entries = pane.diff_nav_entries();
        let texts = entries
            .iter()
            .map(|&ix| pane.diff_text_line_for_region(ix, region).to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            vec!["two", "six", "last"],
            "patch stops should land on each block's first changed line, never on `@@` or \
             `diff --git` rows, and a no-newline edit is one stop in {diff_view:?}"
        );
        entries
    });

    set_diff_row_selection_for_test(cx, view, entries[0], (entries[0], entries[0]));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        entries[1],
        &format!("F3 should reach the second run of the hunk in patch {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        entries[2],
        &format!("F3 should cross into the next file in patch {diff_view:?}"),
    );
    // `-last`, its marker, `+LAST` and its marker; each marker takes the
    // colour of the line it belongs to.
    let outlines = assert_focused_change_block_bar(
        cx,
        view,
        entries[2]..entries[2] + 4,
        diff_view,
        "the no-newline edit's marks should cover its markers",
    );
    use DiffChangeSide::{Added, Removed};
    let expected_outlines = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Removed), Some(Removed), Some(Added), Some(Added)]],
        DiffViewMode::Split => vec![vec![Some(Removed); 4], vec![Some(Added); 4]],
    };
    assert_eq!(
        outlines, expected_outlines,
        "outline colours in {diff_view:?}"
    );
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        entries[1],
        &format!("F2 should step back one block in patch {diff_view:?}"),
    );
}

#[gpui::test]
fn patch_diff_inline_change_stops_land_on_first_changed_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_patch_diff_change_stops_land_on_first_changed_rows(
        cx,
        &view,
        gitcomet_state::model::RepoId(70631),
        "patch_inline_block_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn patch_diff_split_change_stops_land_on_first_changed_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_patch_diff_change_stops_land_on_first_changed_rows(
        cx,
        &view,
        gitcomet_state::model::RepoId(70632),
        "patch_split_block_nav",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn collapsed_diff_first_open_keeps_hunk_header_visible_after_scrolled_file(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(620.0)));

    let repo_id = gitcomet_state::model::RepoId(70633);
    let changes = [
        (20, "old value 20", "new value 20"),
        (40, "old value 40", "new value 40"),
        (60, "old value 60", "new value 60"),
        (80, "old value 80", "new value 80"),
    ];
    let (unified, old_text, new_text) = build_collapsed_diff_multi_hunk_fixture_texts(&changes);
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_stale_offset",
        DiffViewMode::Inline,
        unified.clone(),
        old_text.clone(),
        new_text.clone(),
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let last = pane.diff_visible_len().saturating_sub(1);
            pane.scroll_diff_to_item_strict(last, gpui::ScrollStrategy::Top);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        diff_scroll_offset_y(cx, &view) < 0.0,
        "fixture should leave the first file scrolled down"
    );

    // Same content under another path: a new target that inherits the offset.
    let other_path = PathBuf::from("src/other.rs");
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_stale_offset",
        other_path,
        unified.replace("src/lib.rs", "src/other.rs"),
        old_text,
        new_text,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "second collapsed file opens and autoscrolls",
        |pane| {
            pane.file_diff_cache_target == Some(target.clone())
                && pane.is_collapsed_diff_projection_active()
                && !pane.diff_autoscroll_pending
        },
        |pane| {
            (
                pane.file_diff_cache_target.clone(),
                pane.is_collapsed_diff_projection_active(),
                pane.diff_autoscroll_pending,
            )
        },
    );
    draw_and_drain_test_window(cx);

    assert_eq!(
        diff_scroll_offset_y(cx, &view),
        0.0,
        "opening a collapsed diff should keep its first hunk header in view, not scroll to the \
         first changed line"
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
        "inline submodule collapsed mode activates collapsed projection",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && pane.is_collapsed_diff_projection_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.patch_diff_split_row_len() > 0
                && pane.file_diff_split_row_len() > 0
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} file_diff_active={} collapsed_active={} inflight={:?} cache_target={:?} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.is_collapsed_diff_projection_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target,
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
        "inline submodule collapsed toolbar click activates collapsed projection",
        |pane| {
            pane.is_inline_submodule_diff_active()
                && pane.diff_content_mode == DiffContentMode::Collapsed
                && pane.is_collapsed_diff_projection_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.patch_diff_split_row_len() > 0
                && pane.file_diff_split_row_len() > 0
        },
        |pane| {
            format!(
                "inline_active={} content_mode={:?} file_diff_active={} collapsed_active={} inflight={:?} cache_target={:?} patch_rows={} file_rows={}",
                pane.is_inline_submodule_diff_active(),
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.is_collapsed_diff_projection_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target,
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

/// Clicking a delimiter in the split file diff lights it and its partner.
///
/// The projection has to route through the *side's* real document: a diff
/// interleaves two file versions, so a raw row index says nothing on its own.
#[gpui::test]
fn split_file_diff_click_lights_the_matching_json_braces(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(931);
    let path = PathBuf::from("config.json");
    let old_text = "{\n  \"items\": [1, 2],\n  \"name\": \"old\"\n}\n".to_string();
    let new_text = "{\n  \"items\": [1, 2],\n  \"name\": \"new\"\n}\n".to_string();
    let unified = concat!(
        "@@ -1,4 +1,4 @@\n",
        " {\n",
        "   \"items\": [1, 2],\n",
        "-  \"name\": \"old\"\n",
        "+  \"name\": \"new\"\n",
        " }\n",
    )
    .to_string();

    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "split_pair_json",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "split file diff with prepared syntax on the new side",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.diff_view == DiffViewMode::Split
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "file_diff_active={} view={:?} doc={}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
            )
        },
    );

    // Row 1 on the right side is `  "items": [1, 2],` -- brackets at 11 and 16.
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        1,
        DiffTextRegion::SplitRight,
        11..12,
        "split diff pair bracket hitbox",
    );
    simulate_counted_click(cx, click, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("clicking `[` in the split diff should light its pair");
        assert_eq!(pair.kind, rows::SyntaxPairKind::Bracket);
        assert_eq!(
            pair.spans
                .iter()
                .map(|span| (span.source_visible_ix, span.region, span.range.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, DiffTextRegion::SplitRight, 11..12),
                (1, DiffTextRegion::SplitRight, 16..17),
            ],
            "both ends land on the right-hand side, never across the split"
        );
        assert_eq!(
            pane.diff_text_local_pair_ranges(1, DiffTextRegion::SplitRight)
                .into_vec(),
            vec![11..12, 16..17]
        );
        assert!(
            pane.diff_text_local_pair_ranges(1, DiffTextRegion::SplitLeft)
                .is_empty(),
            "the left side renders the old document and must not be washed"
        );
    });

    // And it actually reaches the paint pass -- the state being right is not the
    // same as a quad being drawn on the row.
    cx.update(|_window, _app| rows::clear_diff_paint_log_for_tests());
    draw_and_drain_test_window(cx);
    let painted = rows::diff_paint_log_for_tests()
        .into_iter()
        .find(|record| record.visible_ix == 1 && record.region == DiffTextRegion::SplitRight)
        .expect("row 1 of the right column should have painted");
    assert_eq!(
        painted.pair_quads,
        vec![11..12, 16..17],
        "the pair quad must be painted on the row, not merely computed"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_text_occurrences
                    .entry((1, DiffTextRegion::SplitRight))
                    .or_default()
                    .push(3..8);
            });
        });
    });
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_pair_match_for_tests().is_none(),
            "changing the row projection must discard pair spans keyed by the old rows"
        );
        assert!(
            pane.diff_text_occurrences_for_tests().is_empty(),
            "changing the row projection must discard occurrence spans keyed by the old rows"
        );
    });
}

/// The inline diff routes every row through the side its text came from, so a
/// context row pairs against the new document and a removed row against the old.
#[gpui::test]
fn inline_file_diff_click_lights_the_matching_json_braces(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(932);
    let path = PathBuf::from("config.json");
    let old_text = "{\n  \"items\": [1, 2],\n  \"name\": \"old\"\n}\n".to_string();
    let new_text = "{\n  \"items\": [1, 2],\n  \"name\": \"new\"\n}\n".to_string();
    let unified = concat!(
        "@@ -1,4 +1,4 @@\n",
        " {\n",
        "   \"items\": [1, 2],\n",
        "-  \"name\": \"old\"\n",
        "+  \"name\": \"new\"\n",
        " }\n",
    )
    .to_string();

    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "inline_pair_json",
        path,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline file diff with prepared syntax on the new side",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.diff_view == DiffViewMode::Inline
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "file_diff_active={} view={:?} doc={}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
            )
        },
    );

    // Inline row 1 is the context line `  "items": [1, 2],`.
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        1,
        DiffTextRegion::Inline,
        11..12,
        "inline diff pair bracket hitbox",
    );
    simulate_counted_click(cx, click, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("clicking `[` in the inline diff should light its pair");
        assert_eq!(pair.kind, rows::SyntaxPairKind::Bracket);
        assert_eq!(
            pane.diff_text_local_pair_ranges(1, DiffTextRegion::Inline)
                .into_vec(),
            vec![11..12, 16..17]
        );
    });

    let previous_signature = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.file_diff_cache_content_signature
            .expect("the first file-diff generation should be installed")
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_text_occurrences
                    .entry((1, DiffTextRegion::Inline))
                    .or_default()
                    .push(3..8);
            });
        });
    });

    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "inline_pair_json",
        PathBuf::from("config.json"),
        2,
        concat!(
            "@@ -1,4 +1,4 @@\n",
            " {\n",
            "   \"items\": (1, 2),\n",
            "-  \"name\": \"old\"\n",
            "+  \"name\": \"newer\"\n",
            " }\n",
        )
        .to_string(),
        "{\n  \"items\": (1, 2),\n  \"name\": \"old\"\n}\n".to_string(),
        "{\n  \"items\": (1, 2),\n  \"name\": \"newer\"\n}\n".to_string(),
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "same-target file-diff highlight generation refresh",
        |pane| {
            pane.file_diff_cache_rev == 2
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_content_signature != Some(previous_signature)
        },
        |pane| {
            format!(
                "rev={} inflight={:?} signature={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_content_signature,
            )
        },
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_pair_match_for_tests().is_none(),
            "an accepted source generation must discard its predecessor's pair spans"
        );
        assert!(
            pane.diff_text_occurrences_for_tests().is_empty(),
            "an accepted source generation must discard its predecessor's occurrences"
        );
    });
}

/// Collapsed mode renders the same file-diff rows as Full, so a click in it must
/// pair too. Gating on `is_file_diff_view_active()` (which demands
/// `DiffContentMode::Full`) silently excluded this whole mode.
#[gpui::test]
fn collapsed_file_diff_click_lights_the_matching_json_braces(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(933);
    let path = PathBuf::from("config.json");
    let mut old_lines: Vec<String> = (0..30).map(|i| format!("  \"pad{i}\": {i},")).collect();
    let mut new_lines = old_lines.clone();
    old_lines.insert(0, "{".to_string());
    new_lines.insert(0, "{".to_string());
    old_lines.push("  \"items\": [1, 2],".to_string());
    new_lines.push("  \"items\": [1, 2],".to_string());
    old_lines.push("  \"name\": \"old\"".to_string());
    new_lines.push("  \"name\": \"new\"".to_string());
    old_lines.push("}".to_string());
    new_lines.push("}".to_string());
    let old_text = format!("{}\n", old_lines.join("\n"));
    let new_text = format!("{}\n", new_lines.join("\n"));
    let unified = format!(
        "@@ -31,3 +31,3 @@\n   \"items\": [1, 2],\n-  \"name\": \"old\"\n+  \"name\": \"new\"\n }}\n"
    );

    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_pair_json",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed pair fixture builds its file diff first",
        |pane| {
            pane.is_file_diff_view_active() && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| format!("file_diff_active={}", pane.is_file_diff_view_active()),
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed projection active with prepared syntax",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && !pane.is_file_diff_view_active()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "collapsed={} file_diff_active={} doc={}",
                pane.is_collapsed_diff_projection_active(),
                pane.is_file_diff_view_active(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
            )
        },
    );

    // Find the visible row showing the `"items"` context line and click its `[`.
    let (row_ix, col) = cx
        .update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (0..pane.diff_visible_len()).find_map(|ix| {
                let text = pane.diff_text_line_for_region(ix, DiffTextRegion::SplitRight);
                text.find('[').map(|col| (ix, col))
            })
        })
        .expect("a visible row should show the `items` line");

    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        row_ix,
        DiffTextRegion::SplitRight,
        col..col + 1,
        "collapsed diff pair bracket hitbox",
    );
    simulate_counted_click(cx, click, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("collapsed mode must pair too -- it renders the same file-diff rows");
        assert_eq!(pair.kind, rows::SyntaxPairKind::Bracket);
        assert_eq!(
            pane.diff_text_local_pair_ranges(row_ix, DiffTextRegion::SplitRight)
                .into_vec(),
            vec![col..col + 1, col + 5..col + 6]
        );
    });
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
            pane.diff_nav_entries().len(),
            2,
            "merging sections keeps one navigation stop per change block (lines 20 and 34)"
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
            pane.diff_nav_entries().len(),
            2,
            "projection rebuilds should keep one navigation stop per change block"
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

fn assert_collapsed_diff_reveal_click_preserves_horizontal_scroll(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
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
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff horizontal overflow becomes available before reveal",
        |pane| match diff_view {
            DiffViewMode::Inline => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
            }
            DiffViewMode::Split => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    && pane
                        .diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x
                        > px(0.0)
            }
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

    let (hunk_src_ix, hidden_up_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        let hidden_up = pane.collapsed_diff_hidden_up_rows(hunk.src_ix);
        assert!(
            hidden_up > 0,
            "fixture should expose hidden rows above the collapsed hunk"
        );
        (hunk.src_ix, hidden_up)
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(540.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(920.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });
    assert!(
        left_before_x < 0.0,
        "test setup should scroll the left/inline diff horizontally, got {left_before_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_x < 0.0,
            "test setup should scroll the split-right diff horizontally, got {right_before_x}"
        );
    }

    let reveal_selector = match diff_view {
        DiffViewMode::Inline => "collapsed_diff_inline_hunk_up",
        DiffViewMode::Split => "collapsed_diff_split_left_hunk_up",
    };
    let reveal_click = debug_selector_center(cx, reveal_selector);
    simulate_counted_click(cx, reveal_click, 1);
    draw_and_drain_test_window(cx);

    let (hidden_up_after, left_after_x, right_after_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            left_x,
            right_x,
        )
    });

    assert!(
        hidden_up_after < hidden_up_before,
        "clicking the collapsed reveal button should expand hidden context"
    );
    assert!(
        (left_after_x - left_before_x).abs() < 0.01,
        "collapsed reveal should preserve left/inline horizontal scroll (before={left_before_x}, after={left_after_x})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            (right_after_x - right_before_x).abs() < 0.01,
            "collapsed reveal should preserve split-right horizontal scroll (before={right_before_x}, after={right_after_x})"
        );
    }
}

#[gpui::test]
fn collapsed_diff_inline_reveal_click_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_reveal_click_preserves_horizontal_scroll(
        cx,
        &view,
        gitcomet_state::model::RepoId(264),
        "collapsed_inline_reveal_preserves_hscroll",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_reveal_click_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_reveal_click_preserves_horizontal_scroll(
        cx,
        &view,
        gitcomet_state::model::RepoId(265),
        "collapsed_split_reveal_preserves_hscroll",
        DiffViewMode::Split,
    );
}

fn assert_collapsed_diff_window_resize_preserves_horizontal_scroll(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
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
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff horizontal overflow becomes available before resize",
        |pane| match diff_view {
            DiffViewMode::Inline => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
            }
            DiffViewMode::Split => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    && pane
                        .diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x
                        > px(0.0)
            }
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

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(540.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(920.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });
    assert!(
        left_before_x < 0.0,
        "test setup should scroll the left/inline diff horizontally, got {left_before_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_x < 0.0,
            "test setup should scroll the split-right diff horizontally, got {right_before_x}"
        );
    }

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff horizontal overflow remains available after resize",
        |pane| match diff_view {
            DiffViewMode::Inline => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
            }
            DiffViewMode::Split => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    && pane
                        .diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x
                        > px(0.0)
            }
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

    let (left_after_x, right_after_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });

    assert!(
        (left_after_x - left_before_x).abs() < 0.01,
        "window resize should preserve left/inline horizontal scroll (before={left_before_x}, after={left_after_x})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            (right_after_x - right_before_x).abs() < 0.01,
            "window resize should preserve split-right horizontal scroll (before={right_before_x}, after={right_after_x})"
        );
    }
}

#[gpui::test]
fn collapsed_diff_inline_window_resize_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_window_resize_preserves_horizontal_scroll(
        cx,
        &view,
        gitcomet_state::model::RepoId(266),
        "collapsed_inline_resize_preserves_hscroll",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_window_resize_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_window_resize_preserves_horizontal_scroll(
        cx,
        &view,
        gitcomet_state::model::RepoId(267),
        "collapsed_split_resize_preserves_hscroll",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn collapsed_diff_inline_resize_back_restores_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(268),
        "collapsed_inline_resize_back_restores_hscroll",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed inline diff horizontal overflow becomes available before wide resize",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let offset = handle.offset();
            let max = handle.max_offset();
            handle.set_offset(point(-max.x.min(px(540.0)), offset.y));
        });
    });
    draw_and_drain_test_window(cx);

    let before_x = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let offset_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        assert!(
            offset_x < 0.0,
            "test setup should scroll inline diff horizontally, got {offset_x}"
        );
        offset_x
    });

    cx.simulate_resize(gpui::size(px(5000.0), px(600.0)));
    draw_and_drain_test_window(cx);
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed inline diff horizontal overflow returns after narrow resize",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    let after_x: f32 = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_scroll.0.borrow().base_handle.offset().x.into()
    });
    assert!(
        (after_x - before_x).abs() < 0.01,
        "horizontal scroll should be restored when overflow returns (before={before_x}, after={after_x})"
    );
}

#[gpui::test]
fn collapsed_diff_inline_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(269),
        "collapsed_inline_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed inline diff horizontal overflow becomes available before unmeasured render",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let offset = handle.offset();
            let max = handle.max_offset();
            handle.set_offset(point(-max.x.min(px(540.0)), offset.y));
        });
    });
    draw_and_drain_test_window(cx);

    let before_x = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let offset_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        assert!(
            offset_x < 0.0,
            "test setup should scroll inline diff horizontally, got {offset_x}"
        );
        offset_x
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.invalidate_font_metrics(cx);
            let mut state = pane.diff_scroll.0.borrow_mut();
            state.last_item_size = None;
            let handle = state.base_handle.clone();
            drop(state);
            let offset = handle.offset();
            handle.set_offset(point(px(0.0), offset.y));
        });
    });
    draw_and_drain_test_window(cx);

    let after_x: f32 = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_scroll.0.borrow().base_handle.offset().x.into()
    });
    assert!(
        after_x.abs() < 0.01,
        "unmeasured render should not force a saved horizontal offset back after the handle moves to zero (before={before_x}, after={after_x})"
    );
}

#[gpui::test]
fn collapsed_diff_inline_unscrolled_unmeasured_render_keeps_horizontal_scroll_range(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(270),
        "collapsed_inline_unscrolled_unmeasured_render_keeps_hscroll",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed inline diff durable horizontal width becomes available",
        |pane| pane.diff_horizontal_content_width() > px(900.0),
        |pane| {
            format!(
                "content_width={:?} offset={:?} max_offset={:?}",
                pane.diff_horizontal_content_width(),
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_scroll.0.borrow().base_handle.offset().x,
            px(0.0),
            "test setup should keep the inline diff at the left edge"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.invalidate_font_metrics(cx);
            pane.diff_scroll.0.borrow_mut().last_item_size = None;
        });
    });
    draw_and_drain_test_window(cx);

    let max_hint = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_horizontal_scroll_max_offset_for_viewport(
            crate::view::panes::main::DiffHorizontalScrollColumn::Primary,
            px(900.0),
        )
    });
    assert!(
        max_hint > px(0.0),
        "unscrolled unmeasured render should keep a durable horizontal range, got {max_hint:?}"
    );

    let hscrollbar_bounds = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_scroll.0.borrow().base_handle.bounds()
    });
    simulate_counted_click(
        cx,
        point(
            hscrollbar_bounds.right() - px(24.0),
            hscrollbar_bounds.bottom() - px(2.0),
        ),
        1,
    );
    draw_and_drain_test_window(cx);

    let after_click_x: f32 = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_scroll.0.borrow().base_handle.offset().x.into()
    });
    assert!(
        after_click_x < 0.0,
        "horizontal scrollbar should remain interactive after unmeasured render, got {after_click_x}"
    );
}

mod cache_and_blame;
mod scrolling;
mod syntax;
use scrolling::push_raw_patch_diff_state_with_rev;
use syntax::{fixture_git_diff, fixture_git_show};
