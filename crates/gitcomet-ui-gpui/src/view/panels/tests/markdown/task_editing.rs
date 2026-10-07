//! Task-list checkboxes: drawing, toggling, and which previews may edit them.

use super::*;

#[gpui::test]
fn worktree_markdown_preview_draws_task_items_as_editable_checkboxes(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(74),
        "markdown_task_checkboxes",
        "- [ ] open\n- [x] done\n- plain\n",
    );

    let open_ix = fixture.row_ix("open");
    let plain_ix = fixture.row_ix("plain");
    let checkbox = cx
        .debug_bounds(String::leak(format!("markdown_task_checkbox_{open_ix}")))
        .expect("a task item draws a checkbox");
    let text = cx
        .debug_bounds(String::leak(format!("markdown_preview_text_box_{open_ix}")))
        .expect("task text box");
    assert!(
        checkbox.right() <= text.left(),
        "the box stands before the text; box={checkbox:?}, text={text:?}"
    );
    assert!(
        cx.debug_bounds(String::leak(format!("markdown_task_checkbox_{plain_ix}")))
            .is_none(),
        "a plain item keeps its bullet"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.markdown_preview_tasks_editable(),
            "an unstaged working-tree file is the file the click writes to"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn toggling_a_task_invalidates_the_file_preview(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8808),
        "markdown_task_toggle_reload",
        "- [ ] ship it\n",
    );
    let row_ix = fixture.row_ix("ship it");
    let task = fixture.document.rows[row_ix].task.expect("task row");

    cx.update(|_window, app| {
        view.read(app).store.insert_repo_for_test(
            gitcomet_state::model::RepoId(8808),
            Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                &fixture.workdir,
            )),
        );
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.toggle_markdown_preview_task(DiffTextRegion::Inline, task, cx);
            });
        });
    });
    cx.run_until_parked();

    let saved_path = fixture.workdir.join("docs/preview.md");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let saved = {
            // Saves briefly park the original before installing the new file.
            // Share the worker's lock so the read cannot land in that gap.
            let _filesystem = gitcomet_core::filesystem::global()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::fs::read_to_string(&saved_path).expect("read saved preview")
        };
        if saved == "- [x] ship it\n" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "checkbox save did not reach disk"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let still_the_old_document = matches!(
            &pane.worktree_markdown.document,
            gitcomet_state::model::Loadable::Ready(document)
                if Arc::ptr_eq(document, &fixture.document)
        );
        assert!(
            !still_the_old_document,
            "after writing the toggle the preview keeps the pre-toggle document, so the box \
             never flips and a second click reports the file as changed on disk"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn a_deleted_file_offers_no_editable_checkboxes(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open_with_status(
        cx,
        &view,
        gitcomet_state::model::RepoId(8809),
        "markdown_task_deleted_file",
        "- [ ] gone\n",
        gitcomet_core::domain::FileStatusKind::Deleted,
    );
    // Deleted from the working tree: the preview shows the old text, but there
    // is no file to write the toggle into.
    std::fs::remove_file(fixture.workdir.join("docs/preview.md")).expect("delete the file");
    draw_frames(cx, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.markdown_preview_tasks_editable(),
            "a deleted file's checkboxes look clickable but every click fails to read the file"
        );
    });

    fixture.cleanup();
}
