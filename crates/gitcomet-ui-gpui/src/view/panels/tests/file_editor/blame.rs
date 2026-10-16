use super::*;

#[gpui::test]
async fn blame_is_available_and_rendered_while_editing(cx: &mut gpui::TestAppContext) {
    // The editor used to leave Blame inert: the toggle was reachable but the
    // buffer had no annotation column, so turning it on did nothing visible.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(950);
    let workdir = unique_workdir("file_editor_blame");
    let file_rel = std::path::PathBuf::from("main.rs");
    let contents = "fn main() {}\nfn other() {}\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    open_annotated_editor(cx, &view, repo_id, &workdir, &file_rel);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(pane.is_file_editor_active());
        assert!(
            pane.annotation_active(),
            "an edited working-tree file is blameable — nothing about editing changes that"
        );
        assert!(
            pane.blame_render_ctx_for_test().is_some(),
            "the editor must resolve a blame context so its gutter has something to draw"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn annotate_column_is_resizable_while_editing(cx: &mut gpui::TestAppContext) {
    // The editor drew the annotation column but mounted no drag handle, so the
    // column was stuck at whatever width the diff view had last left it at.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(952);
    let workdir = unique_workdir("file_editor_annotate_resize");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\nfn other() {}\n")
        .expect("write fixture");

    open_annotated_editor(cx, &view, repo_id, &workdir, &file_rel);
    draw_and_drain_test_window(cx);

    let offset = editor_annotate_handle_offset(cx);
    let column_width = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        pane.annotate_column_width_px(crate::ui_scale::DEFAULT_UI_SCALE_PERCENT)
    });
    assert!(
        (f32::from(offset) - f32::from(column_width)).abs() <= 1.0,
        "the handle must straddle the annotation column's right edge, got {offset:?} for a \
         {column_width:?} column"
    );

    drag_annotate_handle(cx, 60.0);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(
            pane.annotate_column_width, 360.0,
            "dragging 60px right at 100% ui scale must widen the column by 60"
        );
        assert!(
            pane.annotate_resize.is_none(),
            "the release must end the drag"
        );
    });

    let widened_offset = editor_annotate_handle_offset(cx);
    assert!(
        f32::from(widened_offset) - f32::from(offset) > 55.0,
        "the handle must follow the column it resized, moved {:?}",
        widened_offset - offset
    );

    // Past the maximum the clamp takes over rather than the column running on.
    drag_annotate_handle(cx, 1_000.0);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).diff_renderer.read(app).annotate_column_width,
            crate::view::rows::DIFF_ANNOTATION_MAX_WIDTH_PX,
            "the drag must clamp at the maximum column width"
        );
    });

    // ...and the minimum on the way back.
    drag_annotate_handle(cx, -1_000.0);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).diff_renderer.read(app).annotate_column_width,
            crate::view::rows::DIFF_ANNOTATION_MIN_WIDTH_PX,
            "the drag must clamp at the minimum column width"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn annotate_resize_handle_waits_for_the_column_it_resizes(cx: &mut gpui::TestAppContext) {
    // The editor reserves the annotation column only once blame resolves, so
    // the handle has to follow the column rather than the toggle: gated on
    // `annotation_active()` it would sit at x=0 over a column nobody drew.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(953);
    let workdir = unique_workdir("file_editor_annotate_no_blame");
    let file_rel = std::path::PathBuf::from("main.rs");
    std::fs::write(workdir.join(&file_rel), "fn main() {}\nfn other() {}\n")
        .expect("write fixture");

    // Annotate on, but no blame ever loads — `TestBackend` opens no repository.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer.update(cx, |pane, cx| {
                pane.set_annotate_enabled(true, cx);
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.annotation_active(),
            "the toggle is on and the target is blameable"
        );
        assert!(
            pane.blame_render_ctx_for_test().is_none(),
            "no blame resolved, so the editor draws no annotation column"
        );
    });
    assert!(
        cx.debug_bounds("annotate_resize_handle").is_none(),
        "a handle without a column would drag nothing"
    );

    // Blame arrives: the column appears, and the handle with it.
    open_annotated_editor(cx, &view, repo_id, &workdir, &file_rel);
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("annotate_resize_handle").is_some(),
        "the handle must appear once there is a column to resize"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn blame_lines_shift_with_unsaved_edits_instead_of_vanishing() {
    // A clean buffer is line-for-line with the blamed revision.
    for line in [0usize, 1, 40] {
        assert_eq!(
            file_editor_blame_line_for_editor_line(line, None, 0),
            u32::try_from(line + 1).ok()
        );
    }

    // An edit that moved no line boundary leaves the mapping alone, even though
    // the buffer is dirty.
    assert_eq!(
        file_editor_blame_line_for_editor_line(9, Some(3), 0),
        Some(10)
    );

    // Two lines typed at line 3: everything above is untouched...
    assert_eq!(
        file_editor_blame_line_for_editor_line(0, Some(3), 2),
        Some(1)
    );
    assert_eq!(
        file_editor_blame_line_for_editor_line(2, Some(3), 2),
        Some(3)
    );
    // ...the two inserted lines have no revision line behind them...
    assert_eq!(file_editor_blame_line_for_editor_line(3, Some(3), 2), None);
    assert_eq!(file_editor_blame_line_for_editor_line(4, Some(3), 2), None);
    // ...and everything below keeps the attribution it actually has, rather
    // than being blanked or reading two lines low.
    assert_eq!(
        file_editor_blame_line_for_editor_line(5, Some(3), 2),
        Some(4)
    );
    assert_eq!(
        file_editor_blame_line_for_editor_line(20, Some(3), 2),
        Some(19)
    );

    // Deleting two lines at line 3 shifts the other way.
    assert_eq!(
        file_editor_blame_line_for_editor_line(3, Some(3), -2),
        Some(6)
    );
    assert_eq!(
        file_editor_blame_line_for_editor_line(2, Some(3), -2),
        Some(3),
        "lines above the deletion are still themselves"
    );
}

#[gpui::test]
async fn editing_mid_file_keeps_blame_above_the_edit(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(958);
    let workdir = unique_workdir("file_editor_blame_watermark");
    let file_rel = std::path::PathBuf::from("main.rs");
    let contents = "one\ntwo\nthree\nfour\nfive\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        assert_eq!(
            diff_renderer.read(app).file_editor_first_dirty_line(),
            None,
            "a buffer that matches disk has no watermark"
        );
    });

    // Type into line index 2 ("three").
    let at = contents.find("three").expect("fixture line");
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(at..at, "// ", cx);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(pane.file_editor_is_dirty());
        assert_eq!(
            pane.file_editor_first_dirty_line(),
            Some(2),
            "the watermark is the first line the edit touched"
        );
    });

    // An edit further down must not move the watermark back up...
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            let end = pane.file_editor_input.read(cx).text().len();
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(end..end, "six\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert_eq!(
            diff_renderer.read(app).file_editor_first_dirty_line(),
            Some(2),
            "a later edit must not raise the watermark"
        );
    });

    // ...but an edit above it must lower it.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..0, "// header\n", cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert_eq!(
            diff_renderer.read(app).file_editor_first_dirty_line(),
            Some(0),
            "an earlier edit lowers the watermark"
        );
    });

    // Saving makes every line attributable again.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| pane.save_file_editor_buffer(cx));
    });
    finish_editor_saves(&view, cx);
    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(!pane.file_editor_is_dirty());
        assert_eq!(pane.file_editor_first_dirty_line(), None);
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
async fn a_stashed_buffer_brings_its_blame_watermark_back(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(959);
    let workdir = unique_workdir("file_editor_blame_stash");
    let file_rel = std::path::PathBuf::from("main.rs");
    let contents = "one\ntwo\nthree\nfour\n";
    std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, editor_state(repo_id, &workdir, &file_rel), cx);
            this.diff_renderer
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        });
    });
    cx.run_until_parked();

    let diff_renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    let at = contents.find("three").expect("fixture line");
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(at..at, "// ", cx);
            });
        });
    });
    cx.run_until_parked();

    // Leave the file and come back: the watermark must not be rebuilt from
    // nothing, or the restored buffer would blank its whole annotation column.
    cx.update(|_window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.stash_current_file_editor_buffer(cx);
            pane.editor_documents.deactivate();
            pane.ensure_file_editor_loaded(cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = diff_renderer.read(app);
        assert!(pane.file_editor_is_dirty(), "the stashed edit came back");
        assert_eq!(
            pane.file_editor_first_dirty_line(),
            Some(2),
            "and so did the line it started at"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Two lines of blame for the `open_annotated_editor` fixture file.
fn editor_blame_lines() -> Vec<gitcomet_core::services::BlameLine> {
    vec![
        gitcomet_core::services::BlameLine {
            commit_id: std::sync::Arc::from("abcdef1"),
            author: "Sampo Kivistö".into(),
            author_time_unix: Some(1_700_000_000),
            summary: "add main".into(),
            body: None,
            line: "fn main() {}".into(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        },
        gitcomet_core::services::BlameLine {
            commit_id: std::sync::Arc::from("abcdef2"),
            author: "Sampo Kivistö".into(),
            author_time_unix: Some(1_700_000_100),
            summary: "add other".into(),
            body: None,
            line: "fn other() {}".into(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        },
    ]
}

/// Show `file_rel` in the editor with annotate on and blame already loaded for
/// it, so the gutter has a populated annotation column.
fn open_annotated_editor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    file_rel: &Path,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = editor_state(repo_id, workdir, file_rel);
            let repo = &mut Arc::make_mut(&mut state).repos[0];
            repo.diff_state.blame_path = Some(file_rel.to_path_buf());
            repo.diff_state.blame_source = Some(gitcomet_core::domain::BlameSource::WorkingTree(
                gitcomet_core::domain::DiffArea::Unstaged,
            ));
            repo.diff_state.blame =
                gitcomet_state::model::Loadable::Ready(std::sync::Arc::new(editor_blame_lines()));
            push_test_state(this, state, cx);
            this.diff_renderer.update(cx, |pane, cx| {
                pane.set_annotate_enabled(true, cx);
                pane.ensure_file_editor_loaded(cx);
            });
        });
    });
    cx.run_until_parked();
}

/// Left offset of the annotate resize handle within the editor. Panics if the
/// handle is absent.
fn editor_annotate_handle_offset(cx: &mut gpui::VisualTestContext) -> Pixels {
    let handle = cx
        .debug_bounds("annotate_resize_handle")
        .expect("edit mode must mount the annotate resize handle");
    let editor = cx
        .debug_bounds("file_editor")
        .expect("expected `file_editor` bounds");
    handle.center().x - editor.left()
}

/// Press the handle and drag it `dx` to the right. Two moves: gpui starts the
/// drag on the first button-held move and only delivers `DragMoveEvent` on the
/// ones after it.
fn drag_annotate_handle(cx: &mut gpui::VisualTestContext, dx: f32) {
    let start = cx
        .debug_bounds("annotate_resize_handle")
        .expect("expected the annotate resize handle")
        .center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: start,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        point(start.x + px(dx.signum() * 8.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw_and_drain_test_window(cx);
    let end = point(start.x + px(dx), start.y);
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_event(MouseUpEvent {
        position: end,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    draw_and_drain_test_window(cx);
}
