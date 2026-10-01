//! Layout: content modes, whitespace, refresh stability, horizontal scroll.

use super::*;

fn push_inline_submodule_diff_content_mode_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
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
    let target = gitcomet_core::domain::DiffTarget::commit_range(
        gitcomet_core::domain::CommitId("aaaa".into()),
        Some(gitcomet_core::domain::CommitId("bbbb".into())),
        Some(path.clone()),
    );
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
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
                PathBuf::from("vendor/submodule"),
                gitcomet_core::domain::DiffArea::Unstaged,
            ));
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

#[gpui::test]
fn same_file_refresh_keeps_rows_instead_of_flashing_processing(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

#[gpui::test]
fn diff_content_mode_main_pane_persist_path_does_not_reenter_main_pane_updates(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn reveal_whitespace_chars_marks_file_diff_paint_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(186);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_content_mode_regular",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::commit(
        gitcomet_core::domain::CommitId("deadbeef".into()),
        path.clone(),
    );
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

#[gpui::test]
fn diff_content_mode_switches_inline_submodule_diff_between_patch_and_content(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

#[gpui::test]
fn collapsed_diff_inline_hunk_header_stays_pinned_during_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

fn assert_collapsed_diff_window_resize_preserves_horizontal_scroll(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
