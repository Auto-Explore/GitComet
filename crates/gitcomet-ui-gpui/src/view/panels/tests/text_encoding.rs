//! Files that are not UTF-8 in the read-only preview and the editor: decoded
//! as attributes, content or the user's choice say, and written back in the
//! same bytes.

use super::*;
use gitcomet_core::text_format::{
    EncodingAttr, FormatSource, TextAttributes, TextEncoding, TextOverride,
};
use std::path::PathBuf;

fn unique_workdir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{label}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

const FILE: &str = "menu.txt";

/// A repo showing `FILE` from the working tree as a full-content view.
fn file_state(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    edit: bool,
    attributes: Loadable<Arc<TextAttributes>>,
    encoding: Option<TextEncoding>,
) -> Arc<AppState> {
    file_state_for_path(repo_id, workdir, FILE, edit, attributes, encoding)
}

fn file_state_for_path(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    path: &str,
    edit: bool,
    attributes: Loadable<Arc<TextAttributes>>,
    encoding: Option<TextEncoding>,
) -> Arc<AppState> {
    let mut repo = opening_repo_state(repo_id, workdir);
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from(path),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    repo.diff_state.content_preview = true;
    repo.diff_state.edit_mode = edit;
    // The reducer moves these revisions with the values; the pane only
    // re-reads when one moves.
    repo.diff_state.text_attributes_rev = match &attributes {
        Loadable::Ready(_) => 2,
        _ => 1,
    };
    repo.diff_state.text_attributes = attributes;
    repo.diff_state.text_override_rev = u64::from(encoding.is_some());
    repo.diff_state.text_override =
        encoding.map(|encoding| gitcomet_state::model::OpenFileTextOverride {
            path: PathBuf::from(path),
            value: TextOverride {
                encoding: Some(encoding),
                ..TextOverride::default()
            },
        });
    app_state_with_repo(repo, repo_id)
}

fn show(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    state: Arc<AppState>,
    edit: bool,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, state, cx);
            this.main_pane.update(cx, |pane, cx| {
                if edit {
                    pane.ensure_file_editor_loaded(cx);
                } else {
                    pane.ensure_selected_file_preview_loaded(cx);
                }
            });
        });
    });
    cx.run_until_parked();
}

fn open_window(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<super::super::GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    })
}

fn koi8() -> TextEncoding {
    TextEncoding::from_label("koi8-r").unwrap()
}

#[gpui::test]
fn read_only_line_ending_conversion_preserves_buffer_and_format(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::text_format::LineEnding;
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join(FILE), b"caf\xe9\n").unwrap();
    show(
        cx,
        &view,
        file_state(
            gitcomet_state::model::RepoId(9590),
            workdir.path(),
            true,
            Loadable::NotLoaded,
            Some(TextEncoding::UTF_8),
        ),
        true,
    );
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            let before = pane.file_editor_text_format;
            assert!(!before.unwrap().is_writable());
            let text = pane.file_editor_input.read(cx).text().to_owned();
            pane.convert_file_editor_line_endings(LineEnding::CrLf, cx);
            assert_eq!(pane.file_editor_text_format, before);
            assert_eq!(pane.file_editor_input.read(cx).text(), text);
            assert!(!pane.file_editor_is_dirty());
        });
    });
}

#[gpui::test]
fn display_attributes_keep_the_preview_decode_key(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::text_format::{SideKind, TabWidth, TabWidthSource};
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join(FILE), "café\n").unwrap();
    let state = file_state(
        gitcomet_state::model::RepoId(9591),
        workdir.path(),
        false,
        Loadable::Ready(Arc::default()),
        None,
    );
    show(cx, &view, state.clone(), false);
    let original = cx.update(|_, app| {
        view.read(app)
            .main_pane
            .read(app)
            .selected_text_decode_request(SideKind::Worktree)
            .unwrap()
            .1
    });
    let mut next = (*state).clone();
    next.repos[0].diff_state.text_attributes = Loadable::Ready(Arc::new(TextAttributes {
        diff_unset: true,
        tab_width: Some(TabWidth {
            columns: 8,
            source: TabWidthSource::Attribute,
        }),
        ..TextAttributes::default()
    }));
    next.repos[0].diff_state.text_attributes_rev += 1;
    show(cx, &view, Arc::new(next), false);
    cx.update(|_, app| {
        assert_eq!(
            original,
            view.read(app)
                .main_pane
                .read(app)
                .selected_text_decode_request(SideKind::Worktree)
                .unwrap()
                .1
        )
    });
}

#[gpui::test]
fn absolute_file_target_produces_repo_relative_attribute_pattern(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join(FILE), "café\n").unwrap();
    show(
        cx,
        &view,
        file_state_for_path(
            gitcomet_state::model::RepoId(9592),
            workdir.path(),
            workdir.path().join(FILE).to_str().unwrap(),
            true,
            Loadable::NotLoaded,
            None,
        ),
        true,
    );
    cx.update(|_, app| {
        let state = view
            .read(app)
            .main_pane
            .read(app)
            .text_encoding_menu_state()
            .unwrap();
        assert_eq!(state.path, Path::new(FILE));
    });
}

#[gpui::test]
fn focused_restore_keeps_decoded_stage_bytes_without_a_worktree_format(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
    use gitcomet_core::domain::{DiffArea, FileConflictKind, FileStatusKind};
    use gitcomet_core::text_format::SideKind;
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(9520);
    for (original, encoding) in [
        (b"caf\xe9\n".as_slice(), TextEncoding::WINDOWS_1252),
        (b"\xff\xfea\0\n\0".as_slice(), TextEncoding::UTF_16LE),
        (
            b"\x87\x90\n".as_slice(),
            TextEncoding::from_label("shift_jis").unwrap(),
        ),
    ] {
        let (base, _) = ConflictPayload::decode(
            Some(Arc::from(original)),
            None,
            SideKind::GitInternal,
            &TextAttributes::default(),
            Some(encoding),
        );
        let session = ConflictSession::new_with_current(
            FILE.into(),
            FileConflictKind::BothDeleted,
            base,
            ConflictPayload::Absent,
            ConflictPayload::Absent,
            ConflictPayload::Absent,
        );
        let file = gitcomet_state::model::ConflictFile::from_shared_conflict_session(
            Path::new(FILE),
            &session,
        );
        let bytes = conflict_side_output_bytes(&file, ThreeWayColumn::Base).unwrap();
        assert_eq!(bytes.as_ref(), original);
        let mut repo = opening_repo_state(repo_id, workdir.path());
        set_test_file_status_with_conflict(
            &mut repo,
            FILE,
            FileStatusKind::Conflicted,
            Some(FileConflictKind::BothDeleted),
            DiffArea::Unstaged,
        );
        repo.conflict_state.conflict_file_path = Some(FILE.into());
        repo.conflict_state.conflict_file = Loadable::Ready(Some(file));
        repo.conflict_state.conflict_session = Some(session);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                push_test_state(view, app_state_with_repo(repo, repo_id), cx)
            })
        });
        cx.run_until_parked();
        let _ = std::fs::remove_file(workdir.path().join(FILE));
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                assert!(pane.conflict_output_text_format().is_none());
                pane.focused_mergetool_write_side_and_exit(repo_id, Path::new(FILE), &bytes, cx);
            });
        });
        assert_eq!(std::fs::read(workdir.path().join(FILE)).unwrap(), original);
    }
}

#[gpui::test]
fn changing_encoding_preserves_unsaved_conflict_output(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(9511);
    let current = "<<<<<<< ours\ncafé\n=======\ntea\n>>>>>>> theirs\n";
    std::fs::write(
        workdir.path().join(FILE),
        b"<<<<<<< ours\ncaf\xe9\n=======\ntea\n>>>>>>> theirs\n",
    )
    .unwrap();
    let mut repo = conflict_compare_repo_state(
        repo_id,
        workdir.path(),
        Path::new(FILE),
        "base\n",
        "café\n",
        "tea\n",
        current,
    );
    set_test_conflict_status(&mut repo, FILE, gitcomet_core::domain::DiffArea::Unstaged);
    repo.diff_state.text_override = Some(gitcomet_state::model::OpenFileTextOverride {
        path: FILE.into(),
        value: TextOverride {
            encoding: Some(TextEncoding::WINDOWS_1252),
            ..TextOverride::default()
        },
    });
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, app_state_with_repo(repo, repo_id), cx)
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            assert_eq!(
                pane.conflict_resolver.path.as_deref(),
                Some(Path::new(FILE))
            );
            assert!(!pane.conflict_resolved_output_is_modified());
            pane.conflict_resolver_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "my resolution\n", cx);
            });
        });
    });
    cx.run_until_parked();
    let edited = cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.conflict_resolved_output_is_modified());
        pane.conflict_resolver_input.read(app).text().to_string()
    });
    // Picking an encoding for a modified resolution sets what Save writes;
    // the sources are not reread and the edits stay.
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.main_pane.update(cx, |pane, cx| {
                pane.set_text_encoding_override(Some(koi8()), cx)
            });
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert_eq!(pane.conflict_resolver_input.read(app).text(), edited);
        assert_eq!(
            pane.conflict_output_text_format()
                .map(|format| format.format.encoding),
            Some(koi8())
        );
        assert!(
            pane.text_encoding_menu_state()
                .is_some_and(|menu| menu.editor && menu.unsaved),
            "the menu offers Save with encoding for the resolution"
        );
        let state = root.store.snapshot();
        let repo = state.repos.iter().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(
            repo.diff_state
                .text_override_for(Path::new(FILE))
                .unwrap()
                .encoding,
            Some(TextEncoding::WINDOWS_1252)
        );
    });
    // Returning to automatic detection would reload the conflict sources,
    // so it must keep the edits.
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.main_pane
                .update(cx, |pane, cx| pane.set_text_encoding_override(None, cx));
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert_eq!(pane.conflict_resolver_input.read(app).text(), edited);
        assert!(pane.conflict_resolved_output_is_modified());
        let state = root.store.snapshot();
        let repo = state.repos.iter().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(
            repo.diff_state
                .text_override_for(Path::new(FILE))
                .unwrap()
                .encoding,
            Some(TextEncoding::WINDOWS_1252)
        );
        assert!(
            root.toast_host
                .read(app)
                .toasts_for_tests(app)
                .iter()
                .any(|(kind, message)| *kind == components::ToastKind::Warning
                    && message.contains("Save or discard your edits"))
        );
    });
    // Once the output has been saved, the same request can reload normally.
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.mark_conflict_resolved_output_saved(cx);
            pane.set_text_encoding_override(Some(koi8()), cx);
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        let state = view.read(app).store.snapshot();
        let repo = state.repos.iter().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(
            repo.diff_state
                .text_override_for(Path::new(FILE))
                .unwrap()
                .encoding,
            Some(koi8())
        );
    });
}

#[gpui::test]
fn conflict_save_uses_output_encoding_without_reopening_sources(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
    use gitcomet_core::text_format::{LineEndingStats, SideKind, SideTextFormat};
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(9594);
    let mut repo = conflict_compare_repo_state(
        repo_id,
        workdir.path(),
        Path::new(FILE),
        "café\n",
        "café local\n",
        "日本語\n",
        "<<<<<<< ours\ncafé local\n=======\n日本語\n>>>>>>> theirs\n",
    );
    set_test_conflict_status(&mut repo, FILE, gitcomet_core::domain::DiffArea::Unstaged);
    let mut session = ConflictSession::from_stage_inputs(
        FILE.into(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Text("café\n".into()),
        ConflictPayload::Text("café local\n".into()),
        ConflictPayload::Text("日本語\n".into()),
    );
    session.current_format = Some(
        gitcomet_core::text_format::decode_bytes(
            b"caf\xe9\n",
            SideKind::Worktree,
            &TextAttributes::default(),
            Some(TextEncoding::WINDOWS_1252),
        )
        .format,
    );
    session.output_format = Some(SideTextFormat::utf8(LineEndingStats::default()));
    repo.conflict_state.conflict_session = Some(session);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, app_state_with_repo(repo, repo_id), cx)
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            let bytes = pane
                .conflict_output_bytes_for_save("日本語\n".into(), cx)
                .unwrap();
            assert_eq!(bytes.as_bytes(), "日本語\n".as_bytes());
            pane.set_save_text_format(
                gitcomet_core::text_format::TextFormat {
                    encoding: TextEncoding::WINDOWS_1252,
                    bom: false,
                },
                cx,
            );
            let first = pane
                .conflict_output_bytes_for_save("café\n".into(), cx)
                .unwrap();
            pane.mark_conflict_resolved_output_saved(cx);
            assert!(pane.conflict_resolver.output_save_format.is_none());
            let second = pane
                .conflict_output_bytes_for_save("café\n".into(), cx)
                .unwrap();
            assert_eq!(
                second.as_bytes(),
                first.as_bytes(),
                "saving again must keep the encoding just written"
            );
        })
    });
}

#[gpui::test]
fn auto_save_close_waits_only_for_dispatched_writes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    for close_window in [false, true] {
        for (prefix, writable) in [("Ā", false), ("header ", true)] {
            let (view, cx) = open_window(cx);
            let workdir = tempfile::tempdir().unwrap();
            let original = b"caf\xe9\n";
            std::fs::write(workdir.path().join(FILE), original).unwrap();
            let repo_id = gitcomet_state::model::RepoId(9512);
            show(
                cx,
                &view,
                file_state(
                    repo_id,
                    workdir.path(),
                    true,
                    Loadable::NotLoaded,
                    Some(TextEncoding::WINDOWS_1252),
                ),
                true,
            );
            cx.update(|window, app| {
                view.update(app, |view, cx| {
                    view.main_pane.update(cx, |pane, cx| {
                        pane.set_auto_save_file_edits(true, cx);
                        pane.file_editor_input.update(cx, |input, cx| {
                            input.replace_utf8_range(0..0, prefix, cx);
                        });
                        pane.on_file_editor_edited(cx);
                        assert!(pane.file_editor_is_dirty());
                    });
                    assert!(if close_window {
                        view.request_close_window_or_warn(window.window_handle().window_id(), cx)
                    } else {
                        view.request_quit_unsaved_file_edits_prompt(cx)
                    });
                    assert_eq!(view.pending_unsaved_file_edits_flush.is_some(), writable);
                    assert_eq!(view.main_pane.read(cx).file_editor_is_dirty(), !writable);
                    if writable {
                        assert!(view.pending_unsaved_file_edits_prompt.is_none());
                        // The test checks scheduling; don't actually close its window.
                        view.pending_unsaved_file_edits_flush = None;
                    } else {
                        let prompt = view
                            .pending_unsaved_file_edits_prompt
                            .as_ref()
                            .expect("offer Save/Discard instead of retrying a failed encoding");
                        assert_eq!(prompt.files, [SharedString::from(FILE)]);
                        let pane = view.main_pane.read(cx);
                        assert_eq!(pane.file_editor_input.read(cx).text(), "Ācafé\n");
                    }
                });
            });
            assert_eq!(std::fs::read(workdir.path().join(FILE)).unwrap(), original);
        }
    }
}

#[gpui::test]
fn save_all_encoding_failure_cancels_quit_and_preserves_stashed_edits(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(9593);
    std::fs::write(workdir.path().join(FILE), b"caf\xe9\n").unwrap();
    std::fs::write(workdir.path().join("other.txt"), "other\n").unwrap();
    show(
        cx,
        &view,
        file_state(
            repo_id,
            workdir.path(),
            true,
            Loadable::NotLoaded,
            Some(TextEncoding::WINDOWS_1252),
        ),
        true,
    );
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.file_editor_input
                .update(cx, |input, cx| input.replace_utf8_range(0..0, "Ā", cx));
            pane.on_file_editor_edited(cx);
            pane.stash_current_file_editor_buffer(cx);
        })
    });
    show(
        cx,
        &view,
        file_state_for_path(
            repo_id,
            workdir.path(),
            "other.txt",
            true,
            Loadable::NotLoaded,
            None,
        ),
        true,
    );
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.resolve_unsaved_file_edits(crate::view::UnsavedFileEditsAction::QuitApp, true, cx);
            assert!(
                view.pending_unsaved_file_edits_flush.is_none(),
                "failed saves must not retry quit"
            );
            assert!(view.pending_unsaved_file_edits_prompt.is_none());
            let stash = &view.main_pane.read(cx).file_editor_stash;
            let edit = stash
                .get(&(repo_id, FILE.into()))
                .expect("retain recovery text");
            assert_eq!(edit.text.as_ref(), "Ācafé\n");
            assert!(edit.is_dirty());
        })
    });
}

#[gpui::test]
fn tab_width_changes_refresh_patch_search_with_unchanged_rows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(9513);
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId("feedface".into()),
        path: None,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(
        target.clone(),
        "diff --git a/a.txt b/a.txt\n@@ -0,0 +1 @@\n+\tneedle\n",
    );
    let mut repo = opening_repo_state(repo_id, workdir.path());
    repo.diff_state.diff_target = Some(target);
    repo.diff_state.diff_rev = 1;
    repo.diff_state.diff = Loadable::Ready(Arc::new(diff));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            push_test_state(view, app_state_with_repo(repo, repo_id), cx)
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_word_wrap = false;
            pane.default_tab_size = 2;
            pane.sync_display_tab_width(cx);
            pane.diff_search_active = true;
            pane.diff_search_query = "+        needle".into();
            pane.diff_search_recompute_matches();
            assert!(pane.diff_search_matches.is_empty());
            assert!(pane.diff_search_inline_patch_trigram_index.is_some());
            let projection = (
                pane.diff_visible_cache_len,
                pane.diff_visible_cache_projection_rev,
            );
            pane.default_tab_size = 8;
            pane.sync_display_tab_width(cx);
            assert_eq!(pane.diff_search_matches.len(), 1);
            assert_eq!(
                projection,
                (
                    pane.diff_visible_cache_len,
                    pane.diff_visible_cache_projection_rev
                )
            );
            assert_eq!(
                pane.diff_text_full_line_for_region(
                    pane.diff_search_matches[0],
                    DiffTextRegion::Inline
                )
                .as_ref(),
                "+        needle"
            );

            // Changing width while the search is closed must invalidate it too.
            pane.diff_search_active = false;
            pane.default_tab_size = 2;
            pane.sync_display_tab_width(cx);
            pane.diff_search_active = true;
            pane.diff_search_query = "+  needle".into();
            pane.diff_search_recompute_matches();
            assert_eq!(pane.diff_search_matches.len(), 1);
        });
    });
}

fn check_unsaved_buffer_roundtrip(cx: &mut gpui::TestAppContext, auto_save: bool) {
    for (other_bytes, other_encoding) in [
        (b"caf\xe9\n".as_slice(), TextEncoding::UTF_8),
        (
            b"\x87\x90\n".as_slice(),
            TextEncoding::from_label("shift_jis").unwrap(),
        ),
    ] {
        let workdir = tempfile::tempdir().unwrap();
        let original = b"caf\xe9\n";
        std::fs::write(workdir.path().join(FILE), original).unwrap();
        std::fs::write(workdir.path().join("read_only.txt"), other_bytes).unwrap();
        let repo_id = gitcomet_state::model::RepoId(9509);
        let state = |path, encoding| {
            file_state_for_path(
                repo_id,
                workdir.path(),
                path,
                true,
                Loadable::NotLoaded,
                Some(encoding),
            )
        };
        let (view, cx) = open_window(cx);
        show(cx, &view, state(FILE, TextEncoding::WINDOWS_1252), true);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.clone();
            pane.update(app, |pane, cx| {
                pane.set_auto_save_file_edits(auto_save, cx);
                pane.file_editor_input.update(cx, |input, cx| {
                    // This character cannot be saved in Windows-1252.
                    input.replace_utf8_range(0..0, "Ā", cx);
                    input.set_cursor_offset("Ā".len(), cx);
                });
                pane.on_file_editor_edited(cx);
                assert!(pane.file_editor_is_dirty());
            });
        });
        show(cx, &view, state("read_only.txt", other_encoding), true);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert!(pane.file_editor_input.read(app).is_read_only());
            assert_eq!(pane.unsaved_file_edit_keys(), vec![(repo_id, FILE.into())]);
        });
        assert_eq!(std::fs::read(workdir.path().join(FILE)).unwrap(), original);

        show(cx, &view, state(FILE, TextEncoding::WINDOWS_1252), true);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.clone();
            pane.update(app, |pane, cx| {
                let input = pane.file_editor_input.read(cx);
                assert_eq!(input.text(), "Ācafé\n");
                assert_eq!(input.cursor_offset(), "Ā".len());
                assert!(!input.is_read_only());
                assert!(pane.file_editor_is_dirty());
                assert_eq!(
                    pane.file_editor_text_format.unwrap().format.encoding,
                    TextEncoding::WINDOWS_1252
                );
                assert_eq!(pane.file_editor_first_dirty_line, Some(0));
                // The restored buffer accepts edits again, and removing the
                // unsavable character returns to the original clean contents.
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0.."Ā".len(), "", cx);
                });
                pane.on_file_editor_edited(cx);
                assert_eq!(pane.file_editor_input.read(cx).text(), "café\n");
                assert!(!pane.file_editor_is_dirty());
                assert!(pane.unsaved_file_edit_keys().is_empty());
            });
        });
    }
}

#[gpui::test]
async fn restored_edits_resume_encoding_refresh_after_attributes_arrive(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join(FILE), b"caf\xe9\n").unwrap();
    std::fs::write(workdir.path().join("other.txt"), "other\n").unwrap();
    let repo_id = gitcomet_state::model::RepoId(9510);
    let state =
        |attributes, encoding| file_state(repo_id, workdir.path(), true, attributes, encoding);
    show(cx, &view, state(Loadable::NotLoaded, None), true);
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.set_auto_save_file_edits(false, cx);
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "edit ", cx);
            });
            pane.on_file_editor_edited(cx);
        });
    });
    show(
        cx,
        &view,
        file_state_for_path(
            repo_id,
            workdir.path(),
            "other.txt",
            true,
            Loadable::NotLoaded,
            None,
        ),
        true,
    );
    show(cx, &view, state(Loadable::Loading, None), true);
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "edit café\n");
        assert!(pane.file_editor_decode_key.is_none());
    });
    let attributes = Arc::new(TextAttributes::default());
    show(
        cx,
        &view,
        state(Loadable::Ready(attributes.clone()), None),
        true,
    );
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            assert_eq!(pane.file_editor_input.read(cx).text(), "edit café\n");
            assert!(pane.file_editor_is_dirty());
            assert!(pane.file_editor_decode_key.is_some());
            pane.save_file_editor_buffer(cx);
            assert!(!pane.file_editor_is_dirty());
            // No backend repository writes here; report the write as landed.
            hold_editor_save_receipt(pane, repo_id, Path::new(FILE))
                .try_send(true)
                .unwrap();
        });
    });
    show(
        cx,
        &view,
        state(Loadable::Ready(attributes), Some(koi8())),
        true,
    );
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "cafИ\n");
        assert_eq!(
            pane.file_editor_text_format.unwrap().format.encoding,
            koi8()
        );
    });
}

#[gpui::test]
async fn failed_encoding_autosave_preserves_edits_across_navigation(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    check_unsaved_buffer_roundtrip(cx, true);
}

#[gpui::test]
async fn stashed_edits_restore_writability_after_visiting_read_only_files(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    check_unsaved_buffer_roundtrip(cx, false);
}

#[gpui::test]
async fn a_latin1_file_previews_decoded_and_the_strip_names_its_encoding(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_preview");
    std::fs::write(workdir.join(FILE), b"Caf\xe9 cr\xe8me\r\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9501);
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, false, Loadable::NotLoaded, None),
        false,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.worktree_preview_text.as_ref(), "Café crème\r\n");
        let format = pane.worktree_preview_text_format.expect("format recorded");
        assert_eq!(format.format.encoding, TextEncoding::WINDOWS_1252);
        let status = pane.text_format_status().expect("strip shown");
        assert_eq!(status.encoding_label.as_ref(), "Windows-1252");
        assert_eq!(status.line_ending_label.as_deref(), Some("CRLF"));
        assert!(!status.editable);
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn the_preview_waits_for_attributes_and_follows_an_encoding_attribute(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_attribute");
    // "Привет" in KOI8-R.
    std::fs::write(workdir.join(FILE), b"\xf0\xd2\xc9\xd7\xc5\xd4\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9502);
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, false, Loadable::Loading, None),
        false,
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.worktree_preview_text.is_empty(),
            "nothing is decoded before the attributes are known"
        );
    });

    let attributes = TextAttributes {
        encoding: Some(EncodingAttr::from_label("koi8-r")),
        ..TextAttributes::default()
    };
    show(
        cx,
        &view,
        file_state(
            repo_id,
            &workdir,
            false,
            Loadable::Ready(Arc::new(attributes)),
            None,
        ),
        false,
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.worktree_preview_text.as_ref(), "Привет\n");
        assert_eq!(
            pane.worktree_preview_text_format
                .map(|format| format.source),
            Some(FormatSource::EncodingAttribute)
        );
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn choosing_an_encoding_rereads_the_preview(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_override");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9503);
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, false, Loadable::NotLoaded, None),
        false,
    );
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, false, Loadable::NotLoaded, Some(koi8())),
        false,
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.worktree_preview_text.as_ref(), "cafИ\n");
        assert_eq!(
            pane.worktree_preview_text_format
                .map(|format| format.source),
            Some(FormatSource::Override)
        );
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn the_editor_edits_a_latin1_file_and_refuses_to_save_what_it_cannot_encode(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_editor");
    let original = b"caf\xe9\n".to_vec();
    std::fs::write(workdir.join(FILE), &original).expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9504);
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, true, Loadable::NotLoaded, None),
        true,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "café\n");
        assert!(!pane.file_editor_input.read(app).is_read_only());
        assert_eq!(
            pane.file_editor_text_format
                .map(|format| format.format.encoding),
            Some(TextEncoding::WINDOWS_1252)
        );
        assert!(
            pane.text_format_status()
                .is_some_and(|status| status.editable)
        );
    });

    // U+0100 has no windows-1252 byte.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "Ā", cx);
                });
                pane.save_file_editor_buffer(cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_is_dirty(),
            "an unwritable character keeps the edit unsaved"
        );
    });
    assert_eq!(std::fs::read(workdir.join(FILE)).unwrap(), original);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn text_that_does_not_decode_opens_read_only(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_malformed");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9505);
    // Forcing UTF-8 on Latin-1 bytes replaces them with U+FFFD.
    show(
        cx,
        &view,
        file_state(
            repo_id,
            &workdir,
            true,
            Loadable::NotLoaded,
            Some(TextEncoding::UTF_8),
        ),
        true,
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "caf\u{fffd}\n");
        assert!(
            pane.file_editor_input.read(app).is_read_only(),
            "saving would destroy the bytes that did not decode"
        );
        assert!(
            pane.text_format_status()
                .is_some_and(|status| status.encoding_warning)
        );
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn a_new_encoding_choice_rereads_a_clean_editor(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_reopen");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9506);
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, true, Loadable::NotLoaded, None),
        true,
    );
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, true, Loadable::NotLoaded, Some(koi8())),
        true,
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "cafИ\n");
        assert!(!pane.file_editor_is_dirty());
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn save_with_encoding_converts_on_the_next_save(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_save_with");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9507);
    show(
        cx,
        &view,
        file_state(repo_id, &workdir, true, Loadable::NotLoaded, None),
        true,
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert!(!pane.file_editor_is_dirty());
                pane.set_file_editor_save_format(gitcomet_core::text_format::TextFormat::UTF_8, cx);
            });
        });
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_editor_is_dirty(),
            "a conversion is unsaved until it is written"
        );
        assert_eq!(
            pane.file_editor_text_format.map(|format| format.format),
            Some(gitcomet_core::text_format::TextFormat::UTF_8)
        );
        assert_eq!(
            pane.text_format_status()
                .map(|status| status.encoding_label),
            Some("UTF-8".into())
        );
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane
                .update(cx, |pane, cx| pane.save_file_editor_buffer(cx));
        });
    });
    cx.update(|_window, app| {
        assert!(!view.read(app).main_pane.read(app).file_editor_is_dirty());
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn the_editor_rereads_when_the_files_attributes_change(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_attributes_change");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9508);
    let with_attributes = |attributes: TextAttributes, rev: u64| {
        let mut state = file_state(
            repo_id,
            &workdir,
            true,
            Loadable::Ready(Arc::new(attributes)),
            None,
        );
        Arc::make_mut(&mut state).repos[0]
            .diff_state
            .text_attributes_rev = rev;
        state
    };
    let koi8_rule = || TextAttributes {
        encoding: Some(EncodingAttr::from_label("koi8-r")),
        ..TextAttributes::default()
    };
    show(
        cx,
        &view,
        with_attributes(TextAttributes::default(), 2),
        true,
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "café\n");
    });

    // A rule written for the file (or the previous file's attributes being
    // replaced by its own) changes how the clean buffer reads.
    show(cx, &view, with_attributes(koi8_rule(), 3), true);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "cafИ\n");
        assert_eq!(
            pane.file_editor_text_format.map(|format| format.source),
            Some(FormatSource::EncodingAttribute)
        );
    });

    // Unsaved edits are never thrown away for it; the buffer keeps the
    // encoding it was read in.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "Ж", cx);
                });
                pane.on_file_editor_edited(cx);
            });
        });
    });
    show(
        cx,
        &view,
        with_attributes(TextAttributes::default(), 4),
        true,
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "ЖcafИ\n");
        assert!(pane.file_editor_is_dirty());
        assert_eq!(
            pane.file_editor_text_format
                .map(|format| format.format.encoding),
            Some(koi8())
        );
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

/// A Latin-1 file open in the editor with a character windows-1252 has no
/// byte for, so nothing can be written until the encoding changes.
fn open_unencodable_edit(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
) {
    show(
        cx,
        view,
        file_state(repo_id, workdir, true, Loadable::NotLoaded, None),
        true,
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "Ā", cx);
                });
                pane.on_file_editor_edited(cx);
            });
        });
    });
    cx.run_until_parked();
}

fn sync_and_read_editor_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> (bool, bool, bool) {
    crate::view::test_support::drain_store_worker(view, cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.is_file_editor_active(),
            pane.file_editor_is_dirty(),
            pane.file_editor_shows_save_controls(),
        )
    })
}

#[gpui::test]
async fn save_button_stays_in_editor_when_the_text_cannot_be_encoded(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_save_stays");
    let original = b"caf\xe9\n".to_vec();
    std::fs::write(workdir.join(FILE), &original).expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9512);
    open_unencodable_edit(cx, &view, repo_id, &workdir);

    cx.update(|window, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.save_file_editor_buffer_and_exit(window, cx);
        });
    });
    let (active, dirty, _) = sync_and_read_editor_state(cx, &view);
    assert!(
        active,
        "a save that wrote nothing must not leave the editor"
    );
    assert!(dirty);
    assert_eq!(std::fs::read(workdir.join(FILE)).unwrap(), original);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn escape_with_auto_save_stays_and_offers_discard_when_encoding_fails(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_autosave_escape");
    let original = b"caf\xe9\n".to_vec();
    std::fs::write(workdir.join(FILE), &original).expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9513);
    open_unencodable_edit(cx, &view, repo_id, &workdir);
    cx.update(|_window, app| {
        view.read(app).main_pane.clone().update(app, |pane, _| {
            pane.auto_save_file_edits = true;
            assert!(
                !pane.file_editor_shows_save_controls(),
                "auto-save hides Save/Discard while it can write"
            );
        });
    });

    cx.update(|window, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.toggle_file_editor(window, cx);
        });
    });
    let (active, dirty, save_controls) = sync_and_read_editor_state(cx, &view);
    assert!(
        active,
        "leaving through a failed auto-save keeps the editor"
    );
    assert!(dirty);
    assert!(
        save_controls,
        "Discard must be reachable while auto-save cannot write"
    );
    assert_eq!(std::fs::read(workdir.join(FILE)).unwrap(), original);

    // Auto-save off keeps today's way out: the edits are kept and the editor closes.
    cx.update(|window, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.auto_save_file_edits = false;
            pane.toggle_file_editor(window, cx);
        });
    });
    let (active, _, _) = sync_and_read_editor_state(cx, &view);
    assert!(!active);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn picking_an_encoding_with_unsaved_edits_sets_the_save_format(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_pick_while_dirty");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9514);
    open_unencodable_edit(cx, &view, repo_id, &workdir);

    cx.update(|_window, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.set_text_encoding_override(Some(TextEncoding::UTF_8), cx);
            assert_eq!(
                pane.file_editor_text_format.map(|format| format.format),
                Some(gitcomet_core::text_format::TextFormat::UTF_8),
                "the pick is what Save writes"
            );
            assert!(pane.file_editor_is_dirty());
            assert_eq!(pane.file_editor_input.read(cx).text(), "Ācafé\n");
            assert!(pane.save_file_editor_buffer(cx), "UTF-8 can write Ā");
        });
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn converting_away_from_a_reopen_override_reads_back_in_the_new_encoding(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_convert_override");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9515);
    show(
        cx,
        &view,
        file_state(
            repo_id,
            &workdir,
            true,
            Loadable::NotLoaded,
            Some(TextEncoding::ISO_8859_1),
        ),
        true,
    );
    cx.update(|_window, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            assert_eq!(pane.file_editor_input.read(cx).text(), "café\n");
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "ŝ", cx);
            });
            pane.on_file_editor_edited(cx);
            assert!(!pane.save_file_editor_buffer(cx), "ŝ has no Latin-1 byte");
            pane.set_text_encoding_override(Some(TextEncoding::UTF_8), cx);
            assert!(pane.save_file_editor_buffer(cx));
        });
    });
    // The test backend opens no repo, so land the write the store would make.
    std::fs::write(workdir.join(FILE), "ŝcafé\n").expect("simulate the write");
    sync_and_read_editor_state(cx, &view);
    cx.update(|_window, app| {
        let root = view.read(app);
        let state = root.store.snapshot();
        let repo = state.repos.iter().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(
            repo.diff_state
                .text_override_for(Path::new(FILE))
                .and_then(|value| value.encoding),
            None,
            "UTF-8 bytes read as UTF-8 without the old Latin-1 choice"
        );
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.file_editor_decode_key,
            pane.selected_text_decode_request(gitcomet_core::text_format::SideKind::Worktree)
                .map(|(_, key)| key),
            "the saved buffer already holds this text; re-reading it races the write"
        );
    });

    // Reading the new bytes back must not garble them.
    cx.update(|_window, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.reload_file_editor_from_disk(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.file_editor_input.read(app).text(), "ŝcafé\n");
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn the_error_dialog_goes_to_the_character_and_saves_as_utf8(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = unique_workdir("encoding_error_actions");
    std::fs::write(workdir.join(FILE), b"caf\xe9\n").expect("write fixture");
    let repo_id = gitcomet_state::model::RepoId(9516);
    open_unencodable_edit(cx, &view, repo_id, &workdir);
    cx.update(|_window, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            assert!(!pane.save_file_editor_buffer(cx));
        });
    });
    cx.run_until_parked();
    let (toast_id, actions) = cx.update(|_window, app| {
        let notices = view.read(app).toast_host.read(app).error_notices();
        let [(id, notice)] = notices.as_slice() else {
            panic!("one error: {notices:?}");
        };
        assert_eq!(notice.repo_id, Some(repo_id));
        assert!(notice.message.contains("cannot be written in Windows-1252"));
        (*id, notice.actions.clone())
    });
    assert_eq!(
        actions,
        vec![
            crate::view::ErrorAction::SaveEditorAs {
                repo_id,
                path: FILE.into(),
                format: gitcomet_core::text_format::TextFormat::UTF_8,
            },
            crate::view::ErrorAction::RevealInEditor {
                repo_id,
                path: FILE.into(),
                line: 1,
                column: 1,
                ch: 'Ā',
            },
        ]
    );

    let open_dialog = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.open_popover_centered(PopoverKind::ErrorDetails { toast_id }, window, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };
    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} must be drawn"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };

    open_dialog(cx);
    click(cx, "error_details_action_1");
    cx.update(|_window, app| {
        let root = view.read(app);
        assert_eq!(root.popover_host.read(app).popover_kind_for_tests(), None);
        let input = root.main_pane.read(app).file_editor_input.read(app);
        assert_eq!(
            input.selected_range(),
            0.."Ā".len(),
            "Go to selects the character"
        );
        assert_eq!(root.toast_host.read(app).error_notices().len(), 1);
    });

    open_dialog(cx);
    click(cx, "error_details_action_0");
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert!(
            !pane.file_editor_is_dirty(),
            "Save as UTF-8 wrote the buffer"
        );
        assert_eq!(
            pane.file_editor_text_format.map(|format| format.format),
            Some(gitcomet_core::text_format::TextFormat::UTF_8)
        );
        assert!(root.toast_host.read(app).error_notices().is_empty());
        assert_eq!(root.popover_host.read(app).popover_kind_for_tests(), None);
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn review_editor_returning_to_lossy_encoding_stays_read_only(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::text_format::TextFormat;
    let _guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let original = b"\x87\x90\n";
    std::fs::write(workdir.path().join(FILE), original).unwrap();
    let encoding = TextEncoding::from_label("shift_jis").unwrap();
    show(
        cx,
        &view,
        file_state(
            RepoId(9690),
            workdir.path(),
            true,
            Loadable::NotLoaded,
            Some(encoding),
        ),
        true,
    );
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            assert!(pane.file_editor_text_format.unwrap().lossy);
            pane.set_file_editor_save_format(TextFormat::UTF_8, cx);
            assert!(!pane.file_editor_input.read(cx).is_read_only());
            pane.set_file_editor_save_format(
                TextFormat {
                    encoding,
                    bom: false,
                },
                cx,
            );
            assert!(
                pane.file_editor_input.read(cx).is_read_only(),
                "the original bytes still do not round trip"
            );
            assert!(!pane.save_file_editor_buffer(cx));
        });
    });
    assert_eq!(std::fs::read(workdir.path().join(FILE)).unwrap(), original);
}

#[gpui::test]
fn review_conflict_returning_to_lossy_encoding_cannot_save(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::text_format::{SideKind, TextFormat, decode_bytes};
    let _guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let repo_id = RepoId(9691);
    let mut repo = conflict_compare_repo_state(
        repo_id,
        workdir.path(),
        Path::new(FILE),
        "base\n",
        "ours\n",
        "theirs\n",
        "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n",
    );
    set_test_conflict_status(&mut repo, FILE, gitcomet_core::domain::DiffArea::Unstaged);
    let encoding = TextEncoding::from_label("shift_jis").unwrap();
    let format = decode_bytes(
        b"\x87\x90\n",
        SideKind::Worktree,
        &TextAttributes::default(),
        Some(encoding),
    )
    .format;
    let mut session = gitcomet_core::conflict_session::ConflictSession::from_stage_inputs(
        FILE.into(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        gitcomet_core::conflict_session::ConflictPayload::Text("base\n".into()),
        gitcomet_core::conflict_session::ConflictPayload::Text("ours\n".into()),
        gitcomet_core::conflict_session::ConflictPayload::Text("theirs\n".into()),
    );
    session.current_format = Some(format);
    repo.conflict_state.conflict_session = Some(session);
    show(cx, &view, app_state_with_repo(repo, repo_id), false);
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            assert!(pane.conflict_output_text_format().unwrap().lossy);
            pane.set_save_text_format(TextFormat::UTF_8, cx);
            pane.set_save_text_format(format.format, cx);
            assert!(
                pane.conflict_output_bytes_for_save("≒\n".into(), cx)
                    .is_none()
            );
        })
    });
}

#[gpui::test]
fn review_conflict_encoding_choice_is_cleared_with_conflict_lifecycle(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    let repo_id = RepoId(9692);
    let mut repo = conflict_compare_repo_state(
        repo_id,
        workdir.path(),
        Path::new(FILE),
        "base\n",
        "ours\n",
        "theirs\n",
        "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n",
    );
    set_test_conflict_status(&mut repo, FILE, gitcomet_core::domain::DiffArea::Unstaged);
    let state = app_state_with_repo(repo, repo_id);
    show(cx, &view, state.clone(), false);
    let original = cx.update(|_, app| {
        view.read(app)
            .main_pane
            .read(app)
            .conflict_output_text_format()
    });
    for transition in ["save", "close", "resolve"] {
        show(cx, &view, Arc::new(AppState::test_default()), false);
        show(cx, &view, state.clone(), false);
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                pane.set_save_text_format(
                    gitcomet_core::text_format::TextFormat {
                        encoding: koi8(),
                        bom: false,
                    },
                    cx,
                );
            })
        });
        match transition {
            "save" => cx.update(|_, app| {
                view.read(app)
                    .main_pane
                    .clone()
                    .update(app, |pane, cx| pane.mark_conflict_resolved_output_saved(cx))
            }),
            "close" => show(cx, &view, Arc::new(AppState::test_default()), false),
            "resolve" => {
                let mut next = (*state).clone();
                next.repos[0].diff_state.diff_target = None;
                show(cx, &view, Arc::new(next), false);
            }
            _ => unreachable!(),
        }
        show(cx, &view, state.clone(), false);
        cx.update(|_, app| {
            let pane = view.read(app).main_pane.read(app);
            assert!(pane.conflict_resolver.output_save_format.is_none());
            if transition == "save" {
                assert_eq!(
                    pane.conflict_output_text_format().unwrap().format.encoding,
                    koi8()
                );
            } else {
                assert_eq!(pane.conflict_output_text_format(), original);
            }
        });
    }
}

#[gpui::test]
fn review_detect_encoding_with_edits_is_a_warning(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join(FILE), "text\n").unwrap();
    show(
        cx,
        &view,
        file_state(
            RepoId(9693),
            workdir.path(),
            true,
            Loadable::NotLoaded,
            None,
        ),
        true,
    );
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "edit ", cx);
            });
            pane.on_file_editor_edited(cx);
            pane.set_text_encoding_override(None, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        let host = view.read(app).toast_host.read(app);
        assert!(
            host.error_notices().is_empty(),
            "a refused UI action must not create a sticky error"
        );
        assert!(
            host.toasts_for_tests(app)
                .iter()
                .any(|(kind, message)| *kind == components::ToastKind::Warning
                    && message.contains("Save or discard"))
        );
    });
}

#[gpui::test]
fn review_converted_save_keeps_decode_key_while_attributes_load(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::text_format::TextFormat;
    let _guard = lock_visual_test();
    let (view, cx) = open_window(cx);
    let workdir = tempfile::tempdir().unwrap();
    std::fs::write(workdir.path().join(FILE), b"caf\xe9\n").unwrap();
    let repo_id = RepoId(9695);
    for attribute_encoding in [None, Some(EncodingAttr::from_label("windows-1252"))] {
        let saved_override = attribute_encoding.as_ref().map(|_| TextEncoding::UTF_8);
        let attributes = Arc::new(TextAttributes {
            encoding: attribute_encoding,
            ..TextAttributes::default()
        });
        let state = file_state(
            repo_id,
            workdir.path(),
            true,
            Loadable::Ready(attributes.clone()),
            Some(TextEncoding::WINDOWS_1252),
        );
        show(cx, &view, state, true);
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "saved ", cx);
                });
                pane.on_file_editor_edited(cx);
                pane.set_file_editor_save_format(TextFormat::UTF_8, cx);
            })
        });
        show(
            cx,
            &view,
            file_state(
                repo_id,
                workdir.path(),
                true,
                Loadable::Loading,
                Some(TextEncoding::WINDOWS_1252),
            ),
            true,
        );
        let write = cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                let expected = pane
                    .file_editor_decode_key
                    .unwrap()
                    .with_encoding(saved_override);
                assert!(pane.save_file_editor_buffer(cx));
                assert_eq!(
                    pane.file_editor_decode_key,
                    Some(expected),
                    "the optimistic save must keep the read identity until the write lands"
                );
                hold_editor_save_receipt(pane, repo_id, Path::new(FILE))
            })
        });
        // The store has accepted the write, but disk still contains the old
        // bytes. Attribute completion must not read them over the saved buffer.
        show(
            cx,
            &view,
            file_state(
                repo_id,
                workdir.path(),
                true,
                Loadable::Ready(attributes.clone()),
                saved_override,
            ),
            true,
        );
        cx.update(|_, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(pane.file_editor_input.read(app).text(), "saved café\n");
            assert_eq!(
                pane.file_editor_text_format.unwrap().format,
                TextFormat::UTF_8
            );
        });
        // The write lands, so the next pass starts from a saved buffer.
        write.try_send(true).unwrap();
    }
}
