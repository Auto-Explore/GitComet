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
