#![allow(dead_code)]
#![allow(clippy::type_complexity)]

use super::*;
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

fn build_full_diff_multi_line_change_fixture_texts() -> (String, String, String) {
    let old_text = "alpha\nold one\nold two\nold three\nomega\n".to_string();
    let new_text = "alpha\nnew one\nnew two\nnew three\nomega\n".to_string();
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,5 @@
 alpha
-old one
-old two
-old three
+new one
+new two
+new three
 omega
"
    .to_string();
    (unified, old_text, new_text)
}

fn build_full_diff_word_wrap_navigation_fixture_texts() -> (String, String, String) {
    let old_one = format!("old first {}", "left_payload_".repeat(160));
    let new_one = format!("new first {}", "right_payload_".repeat(160));
    let old_two = "old second changed row".to_string();
    let new_two = "new second changed row".to_string();
    let old_text = format!("alpha\n{old_one}\n{old_two}\nomega\n");
    let new_text = format!("alpha\n{new_one}\n{new_two}\nomega\n");
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,4 @@
 alpha
-{old_one}
-{old_two}
+{new_one}
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
        crate::view::panes::main::diff_row_height_for_ui_scale(
            crate::ui_scale::UiScale::current(app).percent(),
        )
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

    cx.update(|_window, app| {
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
    cx.update(|_window, app| {
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

    cx.update(|_window, app| {
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
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(full_wrapped_line.clone()),
        "split diff copy should not insert soft-wrap newlines"
    );
    assert!(!full_wrapped_line.contains('\n'));

    cx.update(|_window, app| {
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
    cx.update(|_window, app| {
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
    });
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
        let char_width = rows::diff_canvas_text_wrap_char_width(window, editor_font_family);
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
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
}

fn assert_full_diff_change_shortcuts_visit_each_changed_row(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_full_diff_multi_line_change_fixture_texts();
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
        "full diff has per-row navigation entries",
        |pane| {
            let entries = pane.diff_nav_entries();
            pane.diff_content_mode == DiffContentMode::Full
                && pane.diff_view == diff_view
                && entries.len() >= 2
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
    let entries = cx.update(|_window, app| view.read(app).main_pane.read(app).diff_nav_entries());
    assert_eq!(
        entries[1],
        entries[0].saturating_add(1),
        "fixture should expose adjacent changed rows in one contiguous change block"
    );
    assert!(
        entries.len() >= 3,
        "fixture should expose at least three changed rows to test continuing from a selection"
    );

    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(entries[0]),
            "first F3 should select the first changed row in Full diff {diff_view:?}"
        );
    });

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(entries[1]),
            "second F3 should advance within the same Full diff change block in {diff_view:?}"
        );
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(entries[0]),
            "F2 should move back one changed row in Full diff {diff_view:?}"
        );
    });

    set_diff_row_selection_for_test(cx, view, entries[0], (entries[0], entries[1]));
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(entries[2]),
            "F3 should continue after the selected Full diff row range in {diff_view:?}"
        );
        assert_eq!(pane.diff_selection_range, Some((entries[2], entries[2])));
    });

    set_diff_row_selection_for_test(cx, view, entries[2], (entries[1], entries[2]));
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(entries[0]),
            "F2 should continue before the selected Full diff row range in {diff_view:?}"
        );
        assert_eq!(pane.diff_selection_range, Some((entries[0], entries[0])));
    });

    let text_region = match diff_view {
        DiffViewMode::Inline => DiffTextRegion::Inline,
        DiffViewMode::Split => DiffTextRegion::SplitLeft,
    };
    set_diff_text_selection_for_test(cx, view, entries[0], entries[1], text_region);
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(entries[2]),
            "F3 should continue after the selected Full diff text range in {diff_view:?}"
        );
        assert_eq!(pane.diff_selection_range, Some((entries[2], entries[2])));
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    set_diff_text_selection_for_test(cx, view, entries[1], entries[2], text_region);
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(entries[0]),
            "F2 should continue before the selected Full diff text range in {diff_view:?}"
        );
        assert_eq!(pane.diff_selection_range, Some((entries[0], entries[0])));
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });
}

#[gpui::test]
fn full_diff_inline_change_shortcuts_visit_each_changed_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_change_shortcuts_visit_each_changed_row(
        cx,
        &view,
        gitcomet_state::model::RepoId(70601),
        "full_diff_inline_row_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_change_shortcuts_visit_each_changed_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_change_shortcuts_visit_each_changed_row(
        cx,
        &view,
        gitcomet_state::model::RepoId(70602),
        "full_diff_split_row_nav",
        DiffViewMode::Split,
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
            "first navigation entry should be the first visual row for its changed logical row"
        );
        assert_eq!(
            second_row.wrap_ix, 0,
            "second navigation entry should be the first visual row for its changed logical row"
        );
        assert!(
            second_row.source_visible_ix > first_row.source_visible_ix,
            "second navigation entry should advance to the next changed logical row"
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
            "first F3 should select the first changed visual row in wrapped Full diff {diff_view:?}"
        );
    });

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
            "F2 should move back to the previous changed visual row in wrapped Full diff {diff_view:?}"
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
                    .is_some_and(|provider| !provider.change_visible_indices().is_empty())
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
            .change_visible_indices()
            .into_iter()
            .next()
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

fn assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_hunk_anchors(
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
        let raw_first_anchor = pane.collapsed_diff_hunk_visible_indices[0];
        let raw_second_anchor = pane.collapsed_diff_hunk_visible_indices[1];
        let expected_first = pane.diff_visual_ix_for_source_visible_ix(raw_first_anchor);
        let expected_second = pane.diff_visual_ix_for_source_visible_ix(raw_second_anchor);
        let entries = pane.diff_nav_entries();
        assert_eq!(
            entries[0], expected_first,
            "first collapsed hunk nav entry should use the mapped visual anchor"
        );
        assert_eq!(
            entries[1], expected_second,
            "second collapsed hunk nav entry should use the mapped visual anchor"
        );
        assert_ne!(
            expected_second, raw_second_anchor,
            "fixture should expose the stale source-visible second hunk index regression"
        );
        let second_row = pane.diff_wrap_visible_rows[expected_second];
        assert_eq!(second_row.source_visible_ix, raw_second_anchor);
        assert_eq!(
            second_row.wrap_ix, 0,
            "collapsed hunk navigation should land on the first visual row for the hunk anchor"
        );
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .take(expected_second)
                .any(|row| row.wrap_ix > 0),
            "fixture should include wrapped visual rows before the second hunk anchor"
        );
        (entries[0], entries[1])
    });

    set_diff_row_selection_for_test(cx, view, first_entry, (first_entry, first_entry));
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(second_entry),
            "F3 should navigate to the mapped collapsed hunk visual anchor in {diff_view:?}"
        );
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(first_entry),
            "F2 should navigate back to the previous collapsed hunk visual anchor in {diff_view:?}"
        );
    });
}

#[gpui::test]
fn collapsed_diff_word_wrap_inline_change_shortcuts_use_visual_hunk_anchors(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_hunk_anchors(
        cx,
        &view,
        gitcomet_state::model::RepoId(70605),
        "collapsed_diff_word_wrap_inline_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_split_change_shortcuts_use_visual_hunk_anchors(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_hunk_anchors(
        cx,
        &view,
        gitcomet_state::model::RepoId(70606),
        "collapsed_diff_word_wrap_split_nav",
        DiffViewMode::Split,
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

fn push_raw_patch_diff_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    unified: String,
) -> gitcomet_core::domain::DiffTarget {
    push_raw_patch_diff_state_with_rev(cx, view, repo_id, fixture_name, unified, 1, true)
}

#[gpui::test]
fn split_file_diff_multiline_search_preserves_blank_side_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(9140);
    let path = PathBuf::from("src/split_search.rs");
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "split_search_blank_side_rows",
        path,
        "\
diff --git a/src/split_search.rs b/src/split_search.rs
index 1111111..2222222 100644
--- a/src/split_search.rs
+++ b/src/split_search.rs
@@ -1,2 +1,3 @@
 foo
+inserted
 bar
"
        .to_string(),
        "foo\nbar\n".to_string(),
        "foo\ninserted\nbar\n".to_string(),
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "split search file diff fixture activates",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.file_diff_split_row_len() == 3
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target.clone(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.diff_view = DiffViewMode::Split;
            pane.diff_search_active = true;

            pane.diff_search_query = "foo\nbar".into();
            pane.diff_search_recompute_matches();
            assert!(
                pane.diff_search_matches.is_empty(),
                "split search must not collapse a visible blank left cell between foo and bar"
            );

            pane.diff_search_query = "foo\n\nbar".into();
            pane.diff_search_recompute_matches();
            assert_eq!(
                pane.diff_search_matches.len(),
                1,
                "split search should match the visible left stream including the blank row"
            );
            let match_row = pane.diff_search_matches[0];
            assert_eq!(
                pane.diff_text_line_for_region(match_row, DiffTextRegion::SplitLeft)
                    .as_ref(),
                "foo"
            );
        });
    });
}

#[gpui::test]
fn diff_search_f3_continues_from_previous_location_after_patch_refresh(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(9138);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_search_refresh_position",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = std::path::PathBuf::from("src/search_refresh.rs");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let push_patch = |cx: &mut gpui::VisualTestContext, diff_rev: u64, unified: &str| {
        let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), unified);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                repo.diff_state.diff_target = Some(target.clone());
                repo.diff_state.diff_state_rev = diff_rev;
                repo.diff_state.diff_rev = diff_rev;
                repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
    };

    let initial_unified = "\
diff --git a/src/search_refresh.rs b/src/search_refresh.rs
index 1111111..2222222 100644
--- a/src/search_refresh.rs
+++ b/src/search_refresh.rs
@@ -1,9 +1,9 @@
 context 0
-old first
+needle first
 context 1
-old second
+needle second
 context 2
-old current
+needle current
 context 3
-old next
+needle next
 context 4
";
    push_patch(cx, 1, initial_unified);
    wait_for_main_pane_condition(
        cx,
        &view,
        "initial patch diff for search refresh regression",
        |pane| pane.diff_cache_rev == 1 && pane.patch_diff_row_len() > 0,
        |pane| (pane.diff_cache_rev, pane.patch_diff_row_len()),
    );

    let previous_visible_ix = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_search_active = true;
            pane.diff_search_query = "needle".into();
            pane.diff_search_recompute_matches();
            assert_eq!(
                pane.diff_search_matches.len(),
                4,
                "initial fixture should expose four search matches"
            );
            pane.diff_search_match_ix = Some(2);
            pane.diff_search_matches[2]
        })
    });

    let refreshed_unified = "\
diff --git a/src/search_refresh.rs b/src/search_refresh.rs
index 1111111..3333333 100644
--- a/src/search_refresh.rs
+++ b/src/search_refresh.rs
@@ -1,8 +1,8 @@
 context 0
-old first
+needle first
 context 1
-old second
+needle second
 context 2
 context 3
-old next
+needle next
 context 4
";
    push_patch(cx, 2, refreshed_unified);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.ensure_diff_visible_indices();
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "refreshed patch diff preserves search cursor before F3",
        |pane| pane.diff_cache_rev == 2 && pane.diff_search_matches.len() == 3,
        |pane| {
            (
                pane.diff_cache_rev,
                pane.diff_visible_len(),
                pane.diff_search_matches.clone(),
                pane.diff_search_match_ix,
            )
        },
    );

    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let match_ix = pane
            .diff_search_match_ix
            .expect("F3 should leave an active search match");
        let active_visible_ix = pane.diff_search_matches[match_ix];
        let active_text = pane
            .diff_text_line_for_region(active_visible_ix, DiffTextRegion::Inline)
            .to_string();

        assert!(
            active_visible_ix > previous_visible_ix,
            "F3 should continue after the pre-refresh match row, got previous={previous_visible_ix}, active={active_visible_ix}, matches={:?}",
            pane.diff_search_matches
        );
        assert!(
            active_text.contains("needle next"),
            "F3 should land on the next later remaining match, got {active_text:?}"
        );
    });
}

#[gpui::test]
fn diff_search_refresh_scrolls_to_first_match_after_previous_zero_match_query(
    cx: &mut gpui::TestAppContext,
) {
    fn unified_with_replacement(replacement: &str) -> String {
        let mut unified = "\
diff --git a/src/search_refresh_scroll.rs b/src/search_refresh_scroll.rs
index 1111111..2222222 100644
--- a/src/search_refresh_scroll.rs
+++ b/src/search_refresh_scroll.rs
@@ -1,72 +1,72 @@
"
        .to_string();

        for ix in 0..72 {
            if ix == 60 {
                unified.push_str("-old focus line\n");
                unified.push('+');
                unified.push_str(replacement);
                unified.push('\n');
            } else {
                unified.push_str(&format!(" context {ix}\n"));
            }
        }

        unified
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(9139);

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    push_raw_patch_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        "search_refresh_zero_previous_match",
        unified_with_replacement("fresh focus line"),
        1,
        true,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "initial no-match patch diff for search refresh regression",
        |pane| pane.diff_cache_rev == 1 && pane.patch_diff_row_len() > 0,
        |pane| (pane.diff_cache_rev, pane.patch_diff_row_len()),
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_search_active = true;
            pane.diff_search_query = "needle".into();
            pane.diff_search_recompute_matches();
            assert!(
                pane.diff_search_matches.is_empty(),
                "initial fixture should have no matches for the active query"
            );
            assert_eq!(pane.diff_search_match_ix, None);
        });
    });

    push_raw_patch_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        "search_refresh_zero_previous_match",
        unified_with_replacement("needle focus line"),
        2,
        true,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "refreshed patch diff scrolls to first new search match",
        |pane| {
            let first_match = pane.diff_search_matches.first().copied();
            pane.diff_cache_rev == 2
                && pane.diff_search_matches.len() == 1
                && pane.diff_search_match_ix == Some(0)
                && pane.diff_selection_anchor == first_match
                && pane.diff_selection_range == first_match.map(|ix| (ix, ix))
                && pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0)
                && pane.diff_scroll.0.borrow().base_handle.offset().y < px(-1.0)
        },
        |pane| {
            (
                pane.diff_cache_rev,
                pane.diff_visible_len(),
                pane.diff_search_matches.clone(),
                pane.diff_search_match_ix,
                pane.diff_selection_anchor,
                pane.diff_selection_range,
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );
}

fn push_raw_patch_diff_state_with_rev(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    unified: String,
    diff_rev: u64,
    ready: bool,
) -> gitcomet_core::domain::DiffTarget {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_raw_patch_root",
        std::process::id(),
        fixture_name
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId("feedface".into()),
        path: None,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = diff_rev;
            repo.diff_state.diff_rev = diff_rev;
            repo.diff_state.diff = if ready {
                gitcomet_state::model::Loadable::Ready(Arc::new(diff))
            } else {
                gitcomet_state::model::Loadable::Loading
            };
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    target
}

fn activate_full_file_diff_horizontal_scroll_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = if diff_view == DiffViewMode::Inline {
        build_full_file_inline_horizontal_scroll_fixture_texts()
    } else {
        build_collapsed_diff_horizontal_scroll_fixture_texts()
    };
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

    set_diff_content_mode_for_test(cx, view, DiffContentMode::Full);
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
        "full file diff horizontal overflow becomes available",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && match diff_view {
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
                }
        },
        |pane| {
            format!(
                "file_diff_active={} target={:?} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
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
}

fn push_working_tree_full_file_horizontal_scroll_fixture_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    area: gitcomet_core::domain::DiffArea,
    diff_view: DiffViewMode,
    diff_rev: u64,
    diff_file_rev: u64,
    patch_ready: bool,
    file_ready: bool,
) -> gitcomet_core::domain::DiffTarget {
    let (unified, old_text, new_text) = if diff_view == DiffViewMode::Inline {
        build_full_file_inline_horizontal_scroll_fixture_texts()
    } else {
        build_collapsed_diff_horizontal_scroll_fixture_texts()
    };
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_unstaged_full_file_hscroll_root",
        std::process::id(),
        fixture_name
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);
    let file_diff =
        gitcomet_core::domain::FileDiffText::new(path.clone(), Some(old_text), Some(new_text));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                area,
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = diff_rev;
            repo.diff_state.diff_rev = diff_rev;
            repo.diff_state.diff = if patch_ready {
                gitcomet_state::model::Loadable::Ready(Arc::new(diff))
            } else {
                gitcomet_state::model::Loadable::Loading
            };
            repo.diff_state.diff_file_rev = diff_file_rev;
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

fn activate_raw_patch_horizontal_scroll_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, _, _) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    let target = push_raw_patch_diff_state(cx, view, repo_id, fixture_name, unified);

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
        "raw patch diff horizontal overflow becomes available",
        |pane| {
            pane.rendered_diff_target() == Some(&target)
                && !pane.is_file_diff_view_active()
                && pane.patch_diff_row_len() > 0
                && match diff_view {
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
                }
        },
        |pane| {
            format!(
                "target={:?} file_diff_active={} patch_rows={} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.rendered_diff_target(),
                pane.is_file_diff_view_active(),
                pane.patch_diff_row_len(),
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
}

fn assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

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

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.invalidate_font_metrics(cx);
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                let mut state = handle.0.borrow_mut();
                state.last_item_size = None;
                let base_handle = state.base_handle.clone();
                drop(state);
                let offset = base_handle.offset();
                base_handle.set_offset(point(px(0.0), offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

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
        left_after_x.abs() < 0.01,
        "unmeasured render should not force saved left/inline horizontal scroll back after the handle moves to zero (before={left_before_x}, after={left_after_x})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_after_x.abs() < 0.01,
            "unmeasured render should not force saved split-right horizontal scroll back after the handle moves to zero (before={right_before_x}, after={right_after_x})"
        );
    }
}

fn assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

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

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                handle.0.borrow_mut().last_item_size = None;
            }
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

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
        "first unmeasured render should not zero left/inline horizontal scroll (before={left_before_x}, after={left_after_x})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            (right_after_x - right_before_x).abs() < 0.01,
            "first unmeasured render should not zero split-right horizontal scroll (before={right_before_x}, after={right_after_x})"
        );
    }
}

fn assert_diff_horizontal_scroll_to_start_persists(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

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

    let (left_scrolled_x, right_scrolled_x) = cx.update(|_window, app| {
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
        left_scrolled_x < 0.0,
        "test setup should scroll the left/inline diff horizontally, got {left_scrolled_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_scrolled_x < 0.0,
            "test setup should scroll the split-right diff horizontally, got {right_scrolled_x}"
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                let base_handle = handle.0.borrow().base_handle.clone();
                let offset = base_handle.offset();
                base_handle.set_offset(point(px(0.0), offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |_pane, cx| cx.notify());
    });
    draw_and_drain_test_window(cx);

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
        left_after_x.abs() < 0.01,
        "left/inline horizontal scroll should stay at start, got {left_after_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_after_x.abs() < 0.01,
            "split-right horizontal scroll should stay at start, got {right_after_x}"
        );
    }
}

fn assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    let (left_before_max, right_before_max) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_max: f32 = pane
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        let right_max: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        (left_max, right_max)
    });
    assert!(
        left_before_max > 0.0,
        "test setup should expose left/inline horizontal range, got {left_before_max}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_max > 0.0,
            "test setup should expose split-right horizontal range, got {right_before_max}"
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                handle.0.borrow_mut().last_item_size = None;
            }
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    let (left_after_max, right_after_max) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_max: f32 = pane
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        let right_max: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        (left_max, right_max)
    });

    assert!(
        (left_after_max - left_before_max).abs() < 1.0,
        "left/inline horizontal range should not flicker across unmeasured render (before={left_before_max}, after={left_after_max})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            (right_after_max - right_before_max).abs() < 1.0,
            "split-right horizontal range should not flicker across unmeasured render (before={right_before_max}, after={right_after_max})"
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct DiffHorizontalScrollbarGeometry {
    label: &'static str,
    scrollbar_bounds: gpui::Bounds<Pixels>,
    viewport_bounds: gpui::Bounds<Pixels>,
    offset_x: f32,
}

fn pixel_delta(a: Pixels, b: Pixels) -> f32 {
    let delta: f32 = (a - b).into();
    delta.abs()
}

fn capture_diff_horizontal_scrollbar_geometry(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) -> Vec<DiffHorizontalScrollbarGeometry> {
    match diff_view {
        DiffViewMode::Inline => {
            let scrollbar_bounds = debug_selector_bounds(cx, "diff_hscrollbar");
            let (viewport_bounds, offset_x) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let state = pane.diff_scroll.0.borrow();
                let offset_x: f32 = state.base_handle.offset().x.into();
                (state.base_handle.bounds(), offset_x)
            });
            vec![DiffHorizontalScrollbarGeometry {
                label: "inline",
                scrollbar_bounds,
                viewport_bounds,
                offset_x,
            }]
        }
        DiffViewMode::Split => {
            let left_scrollbar_bounds = debug_selector_bounds(cx, "diff_split_left_hscrollbar");
            let right_scrollbar_bounds = debug_selector_bounds(cx, "diff_split_right_hscrollbar");
            let ((left_viewport_bounds, left_offset_x), (right_viewport_bounds, right_offset_x)) =
                cx.update(|_window, app| {
                    let pane = view.read(app).main_pane.read(app);
                    let left_state = pane.diff_scroll.0.borrow();
                    let left_offset_x: f32 = left_state.base_handle.offset().x.into();
                    let left = (left_state.base_handle.bounds(), left_offset_x);
                    drop(left_state);
                    let right_state = pane.diff_split_right_scroll.0.borrow();
                    let right_offset_x: f32 = right_state.base_handle.offset().x.into();
                    let right = (right_state.base_handle.bounds(), right_offset_x);
                    (left, right)
                });
            vec![
                DiffHorizontalScrollbarGeometry {
                    label: "split left",
                    scrollbar_bounds: left_scrollbar_bounds,
                    viewport_bounds: left_viewport_bounds,
                    offset_x: left_offset_x,
                },
                DiffHorizontalScrollbarGeometry {
                    label: "split right",
                    scrollbar_bounds: right_scrollbar_bounds,
                    viewport_bounds: right_viewport_bounds,
                    offset_x: right_offset_x,
                },
            ]
        }
    }
}

fn assert_scrollbar_geometry_matches_viewport(geometry: &[DiffHorizontalScrollbarGeometry]) {
    for sample in geometry {
        assert!(
            pixel_delta(
                sample.scrollbar_bounds.size.width,
                sample.viewport_bounds.size.width
            ) < 1.0,
            "{} horizontal scrollbar width should match viewport width (scrollbar={:?}, viewport={:?})",
            sample.label,
            sample.scrollbar_bounds,
            sample.viewport_bounds
        );
        assert!(
            pixel_delta(
                sample.scrollbar_bounds.left(),
                sample.viewport_bounds.left()
            ) < 1.0,
            "{} horizontal scrollbar left edge should match viewport left edge (scrollbar={:?}, viewport={:?})",
            sample.label,
            sample.scrollbar_bounds,
            sample.viewport_bounds
        );
        assert!(
            pixel_delta(
                sample.scrollbar_bounds.right(),
                sample.viewport_bounds.right()
            ) < 1.0,
            "{} horizontal scrollbar right edge should match viewport right edge (scrollbar={:?}, viewport={:?})",
            sample.label,
            sample.scrollbar_bounds,
            sample.viewport_bounds
        );
    }
}

fn assert_scrollbar_geometry_stays_stable(
    before: &[DiffHorizontalScrollbarGeometry],
    after: &[DiffHorizontalScrollbarGeometry],
) {
    assert_eq!(
        before.len(),
        after.len(),
        "geometry capture should keep the same number of scrollbars"
    );
    for (before, after) in before.iter().zip(after.iter()) {
        assert_eq!(before.label, after.label);
        assert!(
            pixel_delta(
                before.scrollbar_bounds.left(),
                after.scrollbar_bounds.left()
            ) < 1.0,
            "{} horizontal scrollbar left edge should not move across unmeasured render (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.right(),
                after.scrollbar_bounds.right()
            ) < 1.0,
            "{} horizontal scrollbar right edge should not move across unmeasured render (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.size.width,
                after.scrollbar_bounds.size.width
            ) < 1.0,
            "{} horizontal scrollbar width should not change across unmeasured render (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            (after.offset_x - before.offset_x).abs() < 0.01,
            "{} horizontal offset should not change across unmeasured render (before={}, after={})",
            before.label,
            before.offset_x,
            after.offset_x
        );
    }
}

fn assert_scrollbar_bounds_stay_stable(
    before: &[DiffHorizontalScrollbarGeometry],
    after: &[DiffHorizontalScrollbarGeometry],
) {
    assert_eq!(
        before.len(),
        after.len(),
        "geometry capture should keep the same number of scrollbars"
    );
    for (before, after) in before.iter().zip(after.iter()) {
        assert_eq!(before.label, after.label);
        assert!(
            pixel_delta(
                before.scrollbar_bounds.left(),
                after.scrollbar_bounds.left()
            ) < 1.0,
            "{} horizontal scrollbar left edge should not move (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.right(),
                after.scrollbar_bounds.right()
            ) < 1.0,
            "{} horizontal scrollbar right edge should not move (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.size.width,
                after.scrollbar_bounds.size.width
            ) < 1.0,
            "{} horizontal scrollbar width should not change (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
    }
}

fn assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
    sync_mode: DiffScrollSync,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, sync_mode);
    }

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "diff horizontal overflow is available before geometry capture",
        |pane| {
            let left_max = pane.diff_scroll.0.borrow().base_handle.max_offset();
            let left_overflows = left_max.x > px(0.0);
            if diff_view == DiffViewMode::Inline {
                return left_overflows;
            }
            left_overflows
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

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(240.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(360.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let before = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    assert_scrollbar_geometry_matches_viewport(&before);
    for sample in &before {
        assert!(
            sample.offset_x < 0.0,
            "{} test setup should start with a horizontal offset, got {}",
            sample.label,
            sample.offset_x
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.invalidate_font_metrics(cx);
            pane.diff_scroll.0.borrow_mut().last_item_size = None;
            pane.diff_split_right_scroll.0.borrow_mut().last_item_size = None;
        });
    });
    draw_and_drain_test_window(cx);

    let after = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    assert_scrollbar_geometry_matches_viewport(&after);
    assert_scrollbar_geometry_stays_stable(&before, &after);
}

fn assert_diff_horizontal_scrollbar_drag_keeps_range_stable(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "diff horizontal overflow is available before scrollbar drag",
        |pane| {
            pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                && (diff_view == DiffViewMode::Inline
                    || pane
                        .diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x
                        > px(0.0))
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

    let before = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    let (before_content_width, before_max_offset) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let column = crate::view::panes::main::DiffHorizontalScrollColumn::Primary;
        let content_width: f32 = pane.diff_horizontal_content_width_for_column(column).into();
        let max_offset: f32 = pane
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        (content_width, max_offset)
    });
    assert!(
        before_max_offset > 0.0,
        "test setup should expose horizontal range before drag, got {before_max_offset}"
    );

    let scrollbar_bounds = before[0].scrollbar_bounds;
    let start = point(
        scrollbar_bounds.left() + px(12.0),
        scrollbar_bounds.center().y,
    );
    let end = point(
        (start.x + px(80.0)).min(scrollbar_bounds.right() - px(12.0)),
        start.y,
    );
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    draw_and_drain_test_window(cx);
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    draw_and_drain_test_window(cx);

    let after = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    assert_scrollbar_bounds_stay_stable(&before, &after);

    let (after_content_width, after_max_offset, after_offset_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let column = crate::view::panes::main::DiffHorizontalScrollColumn::Primary;
        let content_width: f32 = pane.diff_horizontal_content_width_for_column(column).into();
        let state = pane.diff_scroll.0.borrow();
        let max_offset: f32 = state.base_handle.max_offset().x.into();
        let offset_x: f32 = state.base_handle.offset().x.into();
        (content_width, max_offset, offset_x)
    });

    assert!(
        (after_content_width - before_content_width).abs() < 1.0,
        "dragging the horizontal scrollbar should not change measured content width (before={before_content_width}, after={after_content_width})"
    );
    assert!(
        (after_max_offset - before_max_offset).abs() < 1.0,
        "dragging the horizontal scrollbar should not change horizontal range (before={before_max_offset}, after={after_max_offset})"
    );
    assert!(
        after_offset_x < 0.0,
        "dragging the horizontal scrollbar should move horizontally, got {after_offset_x}"
    );
}

#[gpui::test]
fn diff_vertical_scrollbar_gutter_stays_reserved_when_unmeasured(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(287),
        "diff_vertical_scrollbar_gutter_stays_reserved_when_unmeasured",
        DiffViewMode::Split,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
            pane.diff_scroll.0.borrow_mut().last_item_size = None;
            pane.diff_split_right_scroll.0.borrow_mut().last_item_size = None;

            let left_gutter = pane.diff_vertical_scrollbar_gutter_for_column(
                crate::view::panes::main::DiffHorizontalScrollColumn::Primary,
                pane.diff_scroll.clone(),
            );
            let right_gutter = pane.diff_vertical_scrollbar_gutter_for_column(
                crate::view::panes::main::DiffHorizontalScrollColumn::SplitRight,
                pane.diff_split_right_scroll.clone(),
            );

            assert_eq!(
                left_gutter, gutter,
                "unmeasured primary diff should keep reserved vertical gutter"
            );
            assert_eq!(
                right_gutter, gutter,
                "unmeasured split-right diff should keep reserved vertical gutter"
            );
        });
    });
}

fn assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    area: gitcomet_core::domain::DiffArea,
    diff_view: DiffViewMode,
    sync_mode: DiffScrollSync,
) {
    let target = push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        view,
        repo_id,
        fixture_name,
        area,
        diff_view,
        1,
        1,
        true,
        true,
    );
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, sync_mode);
    }
    set_diff_content_mode_for_test(cx, view, DiffContentMode::Full);
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
        "unstaged full-file diff horizontal overflow becomes available",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.file_diff_cache_inflight.is_none()
                && match diff_view {
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
                }
        },
        |pane| {
            format!(
                "active={} target={:?} inflight={:?} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
                pane.file_diff_cache_inflight,
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
            left_handle.set_offset(point(-left_max.x.min(px(360.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(540.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x, left_before_max, right_before_max, seq_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
                pane.file_diff_cache_seq,
            )
        });
    assert!(
        left_before_x < px(0.0),
        "test setup should scroll the left/inline diff horizontally, got {left_before_x:?}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_x < px(0.0),
            "test setup should scroll split-right horizontally, got {right_before_x:?}"
        );
    }

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        view,
        repo_id,
        fixture_name,
        area,
        diff_view,
        2,
        2,
        false,
        false,
    );
    draw_and_drain_test_window(cx);

    let assert_stable = |cx: &mut gpui::VisualTestContext, label: &str, expected_rev: u64| {
        let (left_x, right_x, left_max, right_max, seq, cache_rev, active) =
            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                (
                    pane.diff_scroll.0.borrow().base_handle.offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .offset()
                        .x,
                    pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x,
                    pane.file_diff_cache_seq,
                    pane.file_diff_cache_rev,
                    pane.is_file_diff_view_active(),
                )
            });
        assert!(active, "{label}: file diff view should remain active");
        assert_eq!(
            cache_rev, expected_rev,
            "{label}: same-target cache rev should track the active refresh"
        );
        assert_eq!(
            seq, seq_before,
            "{label}: same-content refresh should not rebuild the file-diff cache"
        );
        assert!(
            (left_x - left_before_x).abs() < px(0.01),
            "{label}: left/inline horizontal offset should stay stable (before={left_before_x:?}, after={left_x:?})"
        );
        assert!(
            (left_max - left_before_max).abs() < px(1.0),
            "{label}: left/inline horizontal range should stay stable (before={left_before_max:?}, after={left_max:?})"
        );
        if diff_view == DiffViewMode::Split {
            assert!(
                (right_x - right_before_x).abs() < px(0.01),
                "{label}: split-right horizontal offset should stay stable (before={right_before_x:?}, after={right_x:?})"
            );
            assert!(
                (right_max - right_before_max).abs() < px(1.0),
                "{label}: split-right horizontal range should stay stable (before={right_before_max:?}, after={right_max:?})"
            );
        }
    };
    assert_stable(cx, "same-target loading redraw", 2);

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        view,
        repo_id,
        fixture_name,
        area,
        diff_view,
        2,
        2,
        true,
        true,
    );
    draw_and_drain_test_window(cx);
    assert_stable(cx, "same-target ready redraw", 2);
}

#[gpui::test]
fn full_file_diff_inline_unstaged_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(912),
        "full_file_inline_unstaged_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn full_file_diff_split_unstaged_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(913),
        "full_file_split_unstaged_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn full_file_diff_split_staged_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(914),
        "full_file_split_staged_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Staged,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn full_file_diff_split_vertical_sync_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(915),
        "full_file_split_vertical_sync_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

fn assert_raw_patch_diff_horizontal_scroll_stable_across_same_target_loading(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
    sync_mode: DiffScrollSync,
) {
    let (unified, _, _) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    let target = push_raw_patch_diff_state_with_rev(
        cx,
        view,
        repo_id,
        fixture_name,
        unified.clone(),
        1,
        true,
    );
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, sync_mode);
    }
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
        "raw patch horizontal overflow becomes available before same-target loading",
        |pane| {
            pane.rendered_diff_target() == Some(&target)
                && !pane.is_file_diff_view_active()
                && pane.diff_cache_rev == 1
                && pane.patch_diff_row_len() > 0
                && match diff_view {
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
                }
        },
        |pane| {
            format!(
                "target={:?} cache_rev={} rows={} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.rendered_diff_target(),
                pane.diff_cache_rev,
                pane.patch_diff_row_len(),
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
            left_handle.set_offset(point(-left_max.x.min(px(360.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(540.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x, left_before_max, right_before_max) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
            )
        });
    assert!(left_before_x < px(0.0));
    if diff_view == DiffViewMode::Split {
        assert!(right_before_x < px(0.0));
    }

    let assert_stable = |cx: &mut gpui::VisualTestContext, label: &str| {
        let (left_x, right_x, left_max, right_max, cache_rev, rows) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
                pane.diff_cache_rev,
                pane.patch_diff_row_len(),
            )
        });
        assert_eq!(cache_rev, 2, "{label}: raw patch cache rev should advance");
        assert!(rows > 0, "{label}: raw patch rows should remain cached");
        assert!(
            (left_x - left_before_x).abs() < px(0.01),
            "{label}: left/inline offset should remain stable"
        );
        assert!(
            (left_max - left_before_max).abs() < px(1.0),
            "{label}: left/inline range should remain stable"
        );
        if diff_view == DiffViewMode::Split {
            assert!(
                (right_x - right_before_x).abs() < px(0.01),
                "{label}: split-right offset should remain stable"
            );
            assert!(
                (right_max - right_before_max).abs() < px(1.0),
                "{label}: split-right range should remain stable"
            );
        }
    };

    push_raw_patch_diff_state_with_rev(cx, view, repo_id, fixture_name, unified.clone(), 2, false);
    draw_and_drain_test_window(cx);
    assert_stable(cx, "raw patch same-target loading redraw");

    push_raw_patch_diff_state_with_rev(cx, view, repo_id, fixture_name, unified, 2, true);
    draw_and_drain_test_window(cx);
    assert_stable(cx, "raw patch same-target ready redraw");
}

#[gpui::test]
fn raw_patch_inline_same_target_loading_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_raw_patch_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(916),
        "raw_patch_inline_same_target_loading_hscroll",
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn raw_patch_split_vertical_sync_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_raw_patch_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(917),
        "raw_patch_split_vertical_sync_same_target_loading_hscroll",
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

#[gpui::test]
fn collapsed_diff_split_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(918);
    let fixture_name = "collapsed_split_same_target_loading_hscroll";
    let target = push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        &view,
        repo_id,
        fixture_name,
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        1,
        1,
        true,
        true,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Split;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split horizontal overflow becomes available before loading refresh",
        |pane| {
            pane.rendered_diff_target() == Some(&target)
                && pane.is_collapsed_diff_projection_active()
                && pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
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
                "collapsed_active={} diff_rev={} file_rev={} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_cache_rev,
                pane.file_diff_cache_rev,
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
            left_handle.set_offset(point(-left_max.x.min(px(360.0)), left_offset.y));

            let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
            let right_offset = right_handle.offset();
            let right_max = right_handle.max_offset();
            right_handle.set_offset(point(-right_max.x.min(px(540.0)), right_offset.y));
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x, left_before_max, right_before_max) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
            )
        });
    assert!(left_before_x < px(0.0));
    assert!(right_before_x < px(0.0));

    let assert_stable = |cx: &mut gpui::VisualTestContext, label: &str| {
        let (left_x, right_x, left_max, right_max, diff_rev, file_rev, active) =
            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                (
                    pane.diff_scroll.0.borrow().base_handle.offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .offset()
                        .x,
                    pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x,
                    pane.diff_cache_rev,
                    pane.file_diff_cache_rev,
                    pane.is_collapsed_diff_projection_active(),
                )
            });
        assert!(
            active,
            "{label}: collapsed projection should remain cache-active"
        );
        assert_eq!(diff_rev, 2, "{label}: patch cache rev should advance");
        assert_eq!(file_rev, 2, "{label}: file cache rev should advance");
        assert!(
            (left_x - left_before_x).abs() < px(0.01),
            "{label}: split-left offset should remain stable"
        );
        assert!(
            (right_x - right_before_x).abs() < px(0.01),
            "{label}: split-right offset should remain stable"
        );
        assert!(
            (left_max - left_before_max).abs() < px(1.0),
            "{label}: split-left range should remain stable"
        );
        assert!(
            (right_max - right_before_max).abs() < px(1.0),
            "{label}: split-right range should remain stable"
        );
    };

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        &view,
        repo_id,
        fixture_name,
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        2,
        2,
        false,
        false,
    );
    draw_and_drain_test_window(cx);
    assert_stable(cx, "collapsed same-target loading redraw");

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        &view,
        repo_id,
        fixture_name,
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        2,
        2,
        true,
        true,
    );
    draw_and_drain_test_window(cx);
    assert_stable(cx, "collapsed same-target ready redraw");
}

#[gpui::test]
fn raw_patch_inline_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(902),
        "raw_patch_inline_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn raw_patch_split_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(903),
        "raw_patch_split_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn raw_patch_split_vertical_sync_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(904),
        "raw_patch_split_vertical_sync_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

#[gpui::test]
fn raw_patch_inline_horizontal_scrollbar_drag_keeps_range_stable(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(909),
        "raw_patch_inline_hscrollbar_drag_range_stable",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scrollbar_drag_keeps_range_stable(cx, &view, DiffViewMode::Inline);
}

#[gpui::test]
fn collapsed_diff_split_horizontal_scrollbar_drag_keeps_range_stable(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(910),
        "collapsed_split_hscrollbar_drag_range_stable",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_drag_keeps_range_stable(cx, &view, DiffViewMode::Split);
}

#[gpui::test]
fn collapsed_diff_inline_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(905),
        "collapsed_inline_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn collapsed_diff_split_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(906),
        "collapsed_split_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn collapsed_diff_split_vertical_sync_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(907),
        "collapsed_split_vertical_sync_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

#[gpui::test]
fn collapsed_diff_split_scroll_sync_setting_controls_each_axis(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(908),
        "collapsed_split_scroll_sync_setting_controls_each_axis",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split diff exposes horizontal and vertical scroll ranges",
        |pane| {
            let left_max = uniform_list_max_offset(&pane.diff_scroll);
            let right_max = uniform_list_max_offset(&pane.diff_split_right_scroll);
            left_max.width > px(40.0)
                && right_max.width > px(40.0)
                && left_max.height > px(120.0)
                && right_max.height > px(120.0)
        },
        |pane| {
            format!(
                "left_offset={:?} right_offset={:?} left_max={:?} right_max={:?}",
                uniform_list_offset(&pane.diff_scroll),
                uniform_list_offset(&pane.diff_split_right_scroll),
                uniform_list_max_offset(&pane.diff_scroll),
                uniform_list_max_offset(&pane.diff_split_right_scroll),
            )
        },
    );

    for mode in ALL_DIFF_SCROLL_SYNC_MODES {
        set_diff_scroll_sync_for_test(cx, &view, mode);

        for axis in ScrollSyncAxis::ALL {
            let scrolled = axis.offset(px(40.0));
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    reset_uniform_list_offsets(&[&pane.diff_scroll, &pane.diff_split_right_scroll]);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    set_uniform_list_offset(&pane.diff_scroll, scrolled);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let expected_right = if axis.includes(mode) {
                    axis.component(scrolled)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_scroll)),
                    axis.component(scrolled),
                    "collapsed split left should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_split_right_scroll)),
                    expected_right,
                    "collapsed split right should {} {} scrolling from the left column in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });

            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    reset_uniform_list_offsets(&[&pane.diff_scroll, &pane.diff_split_right_scroll]);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    set_uniform_list_offset(&pane.diff_split_right_scroll, scrolled);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let expected_left = if axis.includes(mode) {
                    axis.component(scrolled)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_split_right_scroll)),
                    axis.component(scrolled),
                    "collapsed split right should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_scroll)),
                    expected_left,
                    "collapsed split left should {} {} scrolling from the right column in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });
        }
    }
}

#[gpui::test]
fn full_file_diff_inline_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(271),
        "full_file_inline_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_file_diff_split_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(272),
        "full_file_split_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn raw_patch_inline_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(273),
        "raw_patch_inline_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn raw_patch_split_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(274),
        "raw_patch_split_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_file_diff_inline_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(275),
        "full_file_inline_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_file_diff_split_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(276),
        "full_file_split_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn raw_patch_inline_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(277),
        "raw_patch_inline_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn raw_patch_split_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(278),
        "raw_patch_split_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_file_diff_inline_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(279),
        "full_file_inline_scroll_to_start_persists",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Inline);
}

#[gpui::test]
fn full_file_diff_split_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(280),
        "full_file_split_scroll_to_start_persists",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Split);
}

#[gpui::test]
fn raw_patch_inline_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(281),
        "raw_patch_inline_scroll_to_start_persists",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Inline);
}

#[gpui::test]
fn raw_patch_split_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(282),
        "raw_patch_split_scroll_to_start_persists",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Split);
}

#[gpui::test]
fn full_file_diff_inline_horizontal_range_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(283),
        "full_file_inline_horizontal_range_stable_unmeasured",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_file_diff_split_horizontal_range_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(284),
        "full_file_split_horizontal_range_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn raw_patch_inline_horizontal_range_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(285),
        "raw_patch_inline_horizontal_range_stable_unmeasured",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn raw_patch_split_horizontal_range_stable_across_unmeasured_render(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(286),
        "raw_patch_split_horizontal_range_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

fn assert_collapsed_diff_trailing_down_button_clickable_above_hscrollbar(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_trailing_hscroll_fixture_texts();
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
        "collapsed trailing hunk horizontal overflow becomes available",
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

    let (hunk_src_ix, trailing_visible_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let trailing_visible_ix = pane
            .diff_visible_len()
            .checked_sub(1)
            .expect("expected at least one collapsed diff row");
        match pane.collapsed_visible_row(trailing_visible_ix) {
            Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                src_ix,
                expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                display_src_ix: None,
                hidden_rows,
            }) => (src_ix, trailing_visible_ix, hidden_rows),
            row => panic!("expected trailing collapsed down hunk header, got {row:?}"),
        }
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.scroll_diff_to_item_strict(trailing_visible_ix, gpui::ScrollStrategy::Bottom);
        });
    });
    draw_and_drain_test_window(cx);

    let down_selector = match diff_view {
        DiffViewMode::Inline => "collapsed_diff_inline_hunk_down",
        DiffViewMode::Split => "collapsed_diff_split_left_hunk_down",
    };
    let down_bounds = cx
        .debug_bounds(down_selector)
        .unwrap_or_else(|| panic!("expected `{down_selector}` bounds"));
    let scrollbar_top = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_scroll.0.borrow().base_handle.bounds().bottom()
            - components::Scrollbar::gutter(components::ScrollbarAxis::Horizontal)
    });
    assert!(
        down_bounds.center().y < scrollbar_top,
        "collapsed trailing down button center should be above the horizontal scrollbar (button={down_bounds:?}, scrollbar_top={scrollbar_top:?})"
    );

    simulate_counted_click(cx, down_bounds.center(), 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix) < hidden_down_before,
            "clicking the trailing collapsed hunk down button should reveal context"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
    });
}

#[gpui::test]
fn collapsed_diff_inline_trailing_down_button_stays_above_hscrollbar(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_trailing_down_button_clickable_above_hscrollbar(
        cx,
        &view,
        gitcomet_state::model::RepoId(262),
        "collapsed_inline_trailing_hscroll",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_trailing_down_button_stays_above_hscrollbar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_trailing_down_button_clickable_above_hscrollbar(
        cx,
        &view,
        gitcomet_state::model::RepoId(263),
        "collapsed_split_trailing_hscroll",
        DiffViewMode::Split,
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
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

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
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);
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
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Header,
                        text: "diff --git a/page.njk b/page.njk".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Hunk,
                        text: "@@ -1,1 +1,1 @@".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Context,
                        text: " <nav class=\"menu\">Home</nav>".into(),
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

        assert_eq!(
            pane.diff_language_for_src_ix.get(5).copied().flatten(),
            Some(rows::DiffSyntaxLanguage::Jinja),
            "the patch should use the same path-based language detection as file content"
        );
        let jinja_styled = pane
            .diff_text_segments_cache
            .get(5)
            .and_then(|v| v.as_ref().map(|entry| &entry.styled))
            .expect("expected Nunjucks context line to be styled and cached");
        assert!(
            !jinja_styled.highlights.is_empty(),
            "the patch should apply Jinja's injected HTML highlighting"
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
            .diff_text_offset_for_position(visible_ix, DiffTextRegion::Inline, click)
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
            pane.diff_selection_anchor,
            Some(visible_ix),
            "double click on diff text should update the diff focus location"
        );
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
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
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
fn file_diff_cache_rebuilds_when_patch_arrives_after_same_file_refresh(
    cx: &mut gpui::TestAppContext,
) {
    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
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

    fn split_visible_ix_by_old_line(pane: &MainPaneView, old_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.old_line == Some(old_line))
        })
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

    fn inline_visible_ix_by_line_kind(
        pane: &MainPaneView,
        old_line: Option<u32>,
        new_line: Option<u32>,
        kind: gitcomet_core::domain::DiffLineKind,
    ) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix).is_some_and(|line| {
                line.kind == kind && line.old_line == old_line && line.new_line == new_line
            })
        })
    }

    fn wait_for_file_diff_seq_after(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_path: &std::path::Path,
        expected_rev: u64,
        previous_seq: u64,
    ) {
        wait_for_main_pane_condition(
            cx,
            view,
            label,
            |pane| {
                pane.file_diff_cache_rev == expected_rev
                    && pane.file_diff_cache_seq > previous_seq
                    && pane.file_diff_cache_inflight.is_none()
                    && pane.file_diff_cache_path.as_deref() == Some(expected_path)
                    && pane.is_file_diff_view_active()
            },
            |pane| {
                format!(
                    "seq={} previous_seq={} inflight={:?} cache_rev={} path={:?} active={} content_signature={:?}",
                    pane.file_diff_cache_seq,
                    previous_seq,
                    pane.file_diff_cache_inflight,
                    pane.file_diff_cache_rev,
                    pane.file_diff_cache_path,
                    pane.is_file_diff_view_active(),
                    pane.file_diff_cache_content_signature,
                )
            },
        );
    }

    fn assert_file_diff_backgrounds(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
    ) {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_view = DiffViewMode::Split;
                    pane.clear_diff_text_style_caches();
                    pane.ensure_diff_visible_indices();
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let (removed_ix, modified_ix, added_ix) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                split_visible_ix_by_old_line(pane, 2)
                    .expect("expected split visible row for removed old line 2"),
                split_visible_ix_by_new_line(pane, 2)
                    .expect("expected split visible row for modified new line 2"),
                split_visible_ix_by_new_line(pane, 4)
                    .expect("expected split visible row for added new line 4"),
            )
        });
        assert!(
            draw_paint_record_for_visible_ix(cx, view, removed_ix, DiffTextRegion::SplitLeft)
                .row_bg
                .is_some(),
            "{label} should paint split-left removal background after refresh",
        );
        assert!(
            draw_paint_record_for_visible_ix(cx, view, modified_ix, DiffTextRegion::SplitRight)
                .row_bg
                .is_some(),
            "{label} should paint split-right modification background after refresh",
        );
        assert!(
            draw_paint_record_for_visible_ix(cx, view, added_ix, DiffTextRegion::SplitRight)
                .row_bg
                .is_some(),
            "{label} should paint split-right addition background after refresh",
        );

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_view = DiffViewMode::Inline;
                    pane.clear_diff_text_style_caches();
                    pane.ensure_diff_visible_indices();
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let (removed_inline_ix, added_inline_ix) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                inline_visible_ix_by_line_kind(
                    pane,
                    Some(2),
                    None,
                    gitcomet_core::domain::DiffLineKind::Remove,
                )
                .expect("expected inline remove row for old line 2"),
                inline_visible_ix_by_line_kind(
                    pane,
                    None,
                    Some(4),
                    gitcomet_core::domain::DiffLineKind::Add,
                )
                .expect("expected inline add row for new line 4"),
            )
        });
        assert!(
            draw_paint_record_for_visible_ix(cx, view, removed_inline_ix, DiffTextRegion::Inline)
                .row_bg
                .is_some(),
            "{label} should paint inline removal background after refresh",
        );
        assert!(
            draw_paint_record_for_visible_ix(cx, view, added_inline_ix, DiffTextRegion::Inline)
                .row_bg
                .is_some(),
            "{label} should paint inline addition background after refresh",
        );
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(291);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_same_file_patch_ready_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/refresh_highlights.rs");
    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let old_text = "fn main() {\n    let value = 1;\n    let stable = 10;\n}\n";
    let new_text = "fn main() {\n    let value = 2;\n    let stable = 10;\n    let added = value + stable;\n}\n";
    let unified = "\
diff --git a/src/refresh_highlights.rs b/src/refresh_highlights.rs
index 1111111..2222222 100644
--- a/src/refresh_highlights.rs
+++ b/src/refresh_highlights.rs
@@ -1,4 +1,5 @@
 fn main() {
-    let value = 1;
+    let value = 2;
     let stable = 10;
+    let added = value + stable;
 }
";
    let patch_diff = Arc::new(gitcomet_core::domain::Diff::from_unified(
        target.clone(),
        unified,
    ));
    let file_diff = Arc::new(gitcomet_core::domain::FileDiffText::new(
        path.clone(),
        Some(old_text.to_string()),
        Some(new_text.to_string()),
    ));
    let expected_path = workdir.join(&path);

    let push_state = |cx: &mut gpui::VisualTestContext,
                      diff_rev: u64,
                      diff_file_rev: u64,
                      patch_ready: bool,
                      file_ready: bool| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                repo.diff_state.diff_target = Some(target.clone());
                repo.diff_state.diff_rev = diff_rev;
                repo.diff_state.diff = if patch_ready {
                    gitcomet_state::model::Loadable::Ready(Arc::clone(&patch_diff))
                } else {
                    gitcomet_state::model::Loadable::Loading
                };
                repo.diff_state.diff_file_rev = diff_file_rev;
                repo.diff_state.diff_file = if file_ready {
                    gitcomet_state::model::Loadable::Ready(Some(Arc::clone(&file_diff)))
                } else {
                    gitcomet_state::model::Loadable::Loading
                };

                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
    };

    push_state(cx, 1, 1, true, true);
    wait_for_file_diff_seq_after(
        cx,
        &view,
        "initial patch-backed file-diff cache build",
        expected_path.as_path(),
        1,
        0,
    );
    assert_file_diff_backgrounds(cx, &view, "initial patch-backed render");

    for (cycle_ix, (previous_patch_rev, next_file_rev, next_patch_rev)) in
        [(1, 2, 2), (2, 3, 3)].into_iter().enumerate()
    {
        let seq_before_refresh =
            cx.update(|_window, app| view.read(app).main_pane.read(app).file_diff_cache_seq);

        push_state(cx, previous_patch_rev, next_file_rev - 1, false, false);
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_diff_cache_seq, seq_before_refresh,
                "cycle {cycle_ix}: same-target loading should keep the existing cache alive"
            );
        });

        push_state(cx, previous_patch_rev, next_file_rev, false, true);
        wait_for_file_diff_seq_after(
            cx,
            &view,
            "file-ready same-target refresh builds temporary file-only cache",
            expected_path.as_path(),
            next_file_rev,
            seq_before_refresh,
        );
        let file_only_seq =
            cx.update(|_window, app| view.read(app).main_pane.read(app).file_diff_cache_seq);

        push_state(cx, next_patch_rev, next_file_rev, true, true);
        wait_for_file_diff_seq_after(
            cx,
            &view,
            "patch-ready same-target refresh rebuilds patch-backed cache",
            expected_path.as_path(),
            next_file_rev,
            file_only_seq,
        );
        assert_file_diff_backgrounds(cx, &view, &format!("cycle {cycle_ix} patch-backed render"));
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

/// An untracked SVG is preview-only, so no patch is loaded for it — but an SVG
/// never reaches the text-file preview path, so the diff pane's Code view is
/// the only place its source is ever shown. It has to render the file text in
/// either diff mode, and the Image/Code toggle has to stay reachable.
#[gpui::test]
fn untracked_svg_keeps_the_code_view_and_toggle_in_collapsed_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(151);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_untracked_svg_code_view",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("assets/diagram.svg");
    let source = String::from_utf8(image_diff_svg_fixture(64, 64, "#22cc66"))
        .expect("svg fixture should be utf-8");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            // Preview-only: the state layer loads no patch for an untracked file.
            repo.diff_state.diff = gitcomet_state::model::Loadable::NotLoaded;
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(path.clone(), None, Some(source.clone())),
            )));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);

            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_content_mode = DiffContentMode::Collapsed;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Svg, RenderedPreviewMode::Source);
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.is_file_preview_active(),
            "an SVG is classified as an image, so it must not take the text-file preview path"
        );
        // Nothing to collapse without a patch, so the pane falls back to Full.
        assert_eq!(pane.effective_diff_content_mode(), DiffContentMode::Full);
        assert!(pane.wants_file_diff_view(false));
        assert!(!pane.is_collapsed_diff_projection_active());
        assert_eq!(
            crate::view::main_diff_rendered_preview_toggle_kind(
                pane.wants_file_diff_view(false),
                pane.wants_collapsed_diff_view(false),
                false,
                crate::view::diff_target_rendered_preview_kind(Some(&target)),
            ),
            Some(RenderedPreviewKind::Svg),
            "the Image/Code toggle must stay available while Collapsed is selected"
        );
        assert!(
            pane.file_diff_inline_row_len() > 0,
            "the Code view should have the SVG source to render"
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
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
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
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
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
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
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
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
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

/// Opens an unstaged text diff so the diff toolbar (Inline/Split + Blame)
/// renders, and puts the pane in `mode`.
fn push_unstaged_text_diff_for_blame_toggle(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    mode: DiffViewMode,
) {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{fixture_name}",
        std::process::id()
    ));
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
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
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff =
                gitcomet_state::model::Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
                    target: target.clone(),
                    lines: vec![
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Context,
                            text: "fn main() {".into(),
                        },
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Add,
                            text: "    let x = 1;".into(),
                        },
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Context,
                            text: "}".into(),
                        },
                    ],
                }));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    // Go through the root setter so the root and the pane agree, exactly as the
    // toolbar buttons and the session restore do.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_view_mode(mode, cx);
        });
    });
    draw_and_drain_test_window(cx);
}

fn click_blame_toggle(cx: &mut gpui::VisualTestContext) {
    let bounds = cx
        .debug_bounds("diff_annotate")
        .expect("the diff toolbar should render the blame toggle");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
}

/// Regression: enabling blame used to force Split → Inline (and restore it on
/// toggle-off). Blame is an annotation column, not a view mode — the split left
/// column renders it just as the inline view does — so the selected mode must
/// survive the toggle in both directions.
#[gpui::test]
fn blame_toggle_keeps_split_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(281);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_toggle_split",
        DiffViewMode::Split,
    );

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert!(pane.annotate_enabled, "clicking blame should enable it");
        assert_eq!(
            pane.diff_view,
            DiffViewMode::Split,
            "enabling blame must not switch the diff view to Inline"
        );
        assert_eq!(root.diff_view_mode, DiffViewMode::Split);
    });

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert!(!pane.annotate_enabled);
        assert_eq!(
            pane.diff_view,
            DiffViewMode::Split,
            "disabling blame must not change the diff view either"
        );
        assert_eq!(root.diff_view_mode, DiffViewMode::Split);
    });
}

#[gpui::test]
fn blame_toggle_keeps_inline_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(282);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_toggle_inline",
        DiffViewMode::Inline,
    );

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert!(pane.annotate_enabled);
        assert_eq!(pane.diff_view, DiffViewMode::Inline);
        assert_eq!(root.diff_view_mode, DiffViewMode::Inline);
    });
}

/// The annotation column narrows the left split column, so the shared split
/// wrap width must shrink when blame is on — the guarantee that made forcing
/// Inline unnecessary in the first place.
#[gpui::test]
fn split_annotate_reserves_the_annotation_column(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(283);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_split_columns",
        DiffViewMode::Split,
    );

    let (_, split_without_blame) = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| pane.diff_wrap_columns(window, cx))
    });

    click_blame_toggle(cx);

    let (_, split_with_blame) = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            assert!(
                pane.annotation_active(),
                "an unstaged working-tree diff supports blame, so the column is active"
            );
            pane.diff_wrap_columns(window, cx)
        })
    });

    assert!(
        split_with_blame < split_without_blame,
        "the annotation column must narrow the split wrap width \
         (with blame: {split_with_blame}, without: {split_without_blame})"
    );
}

/// The command palette and the Settings window route mode changes through
/// `GitCometView::set_diff_view_mode` rather than the toolbar buttons, so the
/// styled-segment cache clear has to live in the pane setter: inline keys those
/// segments by `row_ix` while split keys them by `row_ix * 2` / `row_ix * 2 + 1`
/// against the same epochs, so a stale entry can paint the wrong row.
#[gpui::test]
fn toggle_diff_view_command_clears_styled_segment_caches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(284);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "toggle_diff_view_cache",
        DiffViewMode::Inline,
    );

    // Seed the inline key space directly: which rows a draw happens to cache
    // depends on syntax availability and streaming heuristics, and the contract
    // under test is only that a mode change drops whatever is cached.
    let cached = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            for key in 0..3 {
                pane.diff_text_segments_cache_set(
                    key,
                    0,
                    crate::view::diff_text_model::CachedDiffStyledText {
                        text: "let x = 1;".into(),
                        highlights: Arc::from(Vec::new()),
                        highlights_hash: 0,
                        text_hash: 0,
                    },
                );
            }
            pane.diff_text_pair_match = Some(DiffTextPairMatch {
                kind: rows::SyntaxPairKind::Bracket,
                spans: Vec::new(),
            });
            pane.diff_text_occurrences
                .entry((0, DiffTextRegion::Inline))
                .or_default()
                .push(0..3);
            pane.diff_text_segments_cache.iter().flatten().count()
        })
    });
    assert_eq!(cached, 3);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.execute_command("toggle-diff-view", None, cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert_eq!(pane.diff_view, DiffViewMode::Split);
        assert_eq!(
            pane.diff_text_segments_cache.iter().flatten().count(),
            0,
            "switching modes outside the toolbar must still clear the aliasing cache"
        );
        assert!(pane.diff_text_pair_match_for_tests().is_none());
        assert!(pane.diff_text_occurrences_for_tests().is_empty());
    });
}

/// A match far along a long line has to be scrolled to sideways as well as
/// down. Without it the row comes into view with the hit still off the right
/// edge, which reads as "search found it but did not go there".
fn assert_diff_search_scrolls_sideways(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
) {
    let mut unified = concat!(
        "diff --git a/wide.txt b/wide.txt\n",
        "--- a/wide.txt\n",
        "+++ b/wide.txt\n",
        "@@ -1,12 +1,12 @@\n",
    )
    .to_string();
    for ix in 0..12 {
        if ix == 6 {
            // The needle sits well past any plausible viewport width.
            unified.push_str(&format!(" {}needle tail\n", "pad ".repeat(200)));
        } else {
            unified.push_str(&format!(" context {ix}\n"));
        }
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    push_raw_patch_diff_state_with_rev(cx, &view, repo_id, fixture_name, unified, 1, true);
    wait_for_main_pane_condition(
        cx,
        &view,
        "wide patch diff ready for horizontal search reveal",
        |pane| pane.diff_cache_rev == 1 && pane.patch_diff_row_len() > 0,
        |pane| (pane.diff_cache_rev, pane.patch_diff_row_len()),
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_search_active = true;
            pane.diff_search_query = "needle tail".into();
            pane.diff_search_recompute_matches_and_scroll_to_first();
            cx.notify();
        });
    });
    // Three passes: the vertical jump lands, the row paints its hitbox, and the
    // sideways reveal reads that hitbox on the frame after.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "expected the long line to be the only match, got {:?}",
            pane.diff_search_matches
        );
        let handle = pane.diff_scroll.0.borrow().base_handle.clone();
        assert!(
            handle.max_offset().x > px(0.0),
            "fixture must overflow horizontally for this to mean anything; max={:?}",
            handle.max_offset()
        );
        assert!(
            handle.offset().x < px(0.0),
            "expected the diff to scroll right to the match, x stayed at {:?} (mode={:?})",
            handle.offset(),
            pane.diff_view,
        );

        assert_eq!(
            pane.diff_search_horizontal_reveal, None,
            "the reveal should be claimed once, not re-applied every frame"
        );
    });
}

#[gpui::test]
fn diff_search_scrolls_sideways_to_a_match_far_along_a_long_line(cx: &mut gpui::TestAppContext) {
    assert_diff_search_scrolls_sideways(
        cx,
        gitcomet_state::model::RepoId(9141),
        "search_horizontal_reveal",
    );
}
