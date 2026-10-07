use super::*;

#[gpui::test]
fn active_search_switches_to_source_on_mermaid_navigation(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(94);
    let revision = std::cell::Cell::new(0);
    let select = |cx: &mut gpui::VisualTestContext, path: &str, source: &str| {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                let mut repo = opening_repo_state(repo_id, directory.path());
                set_test_file_status(
                    &mut repo,
                    path,
                    gitcomet_core::domain::FileStatusKind::Modified,
                    DiffArea::Unstaged,
                );
                revision.set(revision.get() + 1);
                repo.diff_state.diff_state_rev = revision.get();
                repo.diff_state.diff_file_rev = revision.get();
                repo.diff_state.diff_file =
                    Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
                        path.into(),
                        Some(source.into()),
                        Some(source.into()),
                    ))));
                push_test_state(view, app_state_with_repo(repo, repo_id), cx);
            })
        });
        // Let the model observer publish the new snapshot before probing it.
        cx.update(|_, app| {
            view.read(app).main_pane.clone().update(app, |pane, cx| {
                assert!(
                    matches!(pane.rendered_file_diff_loadable(), Some(Loadable::Ready(Some(file)))
                    if file.new.as_deref() == Some(source))
                );
                pane.diff_content_mode = DiffContentMode::Full;
                if !pane.diff_search_active {
                    pane.rendered_preview_modes
                        .set(RenderedPreviewKind::Diagram, RenderedPreviewMode::Rendered);
                }
                pane.ensure_diagram_search_source(cx);

                assert_eq!(
                    pane.rendered_preview_modes
                        .get(RenderedPreviewKind::Diagram),
                    if pane.diff_search_active && path.ends_with(".mmd") {
                        RenderedPreviewMode::Source
                    } else {
                        RenderedPreviewMode::Rendered
                    }
                );
            });
        });
    };
    select(cx, "ordinary.rs", "let signal = 1;");
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, _| {
            pane.diff_search_active = true;
            pane.diff_search_query = "wave".into();
        });
    });
    // Search stays active across navigation to Mermaid.
    select(cx, "graph.mmd", "flowchart LR;A-->B");
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.diff_search_active);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Diagram),
            RenderedPreviewMode::Source
        );
    });
}

#[gpui::test]
fn mermaid_file_modes_follow_current_target_and_removed_formats_stay_code(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let repo_id = gitcomet_state::model::RepoId(93);
    let select = |cx: &mut gpui::VisualTestContext, path: &str, source: &str| {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                let mut repo = opening_repo_state(repo_id, directory.path());
                set_test_file_status(
                    &mut repo,
                    path,
                    gitcomet_core::domain::FileStatusKind::Modified,
                    DiffArea::Unstaged,
                );
                repo.diff_state.diff_file =
                    Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
                        path.into(),
                        Some(source.into()),
                        Some(source.into()),
                    ))));
                let state = app_state_with_repo(repo, repo_id);
                push_test_state(view, state, cx);
                view.main_pane.update(cx, |pane, cx| {
                    pane.diff_content_mode = DiffContentMode::Full;
                    pane.rendered_preview_modes
                        .set(RenderedPreviewKind::Diagram, RenderedPreviewMode::Rendered);
                    pane.ensure_diagram_search_source(cx);
                });
            })
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    };
    for (path, source, expected) in [
        ("graph.d2", "a -> b", None),
        ("graph.dot", "digraph { a -> b }", None),
        (
            "graph.mmd",
            "flowchart LR
A-->B",
            Some(RenderedPreviewKind::Diagram),
        ),
        (
            "graph.puml",
            "@startuml
A -> B
@enduml",
            None,
        ),
        ("graph.wavejson", "{signal:[{wave:'01..'}]}", None),
        ("timing.json5", "{signal:[{wave:'01..'}]}", None),
    ] {
        select(cx, path, source);
        cx.update(|_, app| {
            assert_eq!(
                view.read(app)
                    .main_pane
                    .read(app)
                    .main_pane_surface()
                    .toggle_kind,
                expected,
                "{path}"
            );
        });
        assert_eq!(
            cx.debug_bounds("diagram_diff_view_toggle").is_some(),
            expected.is_some(),
            "{path}"
        );
    }
    select(cx, "ordinary.json", "{\"signal\":[1,2]}");
    cx.update(|_, app| {
        assert_eq!(
            view.read(app)
                .main_pane
                .read(app)
                .main_pane_surface()
                .toggle_kind,
            None
        )
    });
    select(cx, "graph.mmd", "flowchart LR\nA-->B");
    let left = cx.debug_bounds("diff_diagram_left").unwrap();
    let right = cx.debug_bounds("diff_diagram_right").unwrap();
    assert_eq!(left.origin.y, right.origin.y);
    assert!((left.size.width - right.size.width).abs() <= px(1.0));
    assert!(left.right() < right.left());
    assert!(cx.debug_bounds("diagram_source_toggle").is_none());
    assert!(cx.debug_bounds("diagram_copy_source").is_none());
    assert!(cx.debug_bounds("diff_inline").is_none());
    cx.update(|_, app| {
        view.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.diff_content_mode = DiffContentMode::Collapsed;
            cx.notify();
        });
    });
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("diff_diagram_left").is_some());
    assert!(cx.debug_bounds("diff_diagram_right").is_some());
    let code = cx.debug_bounds("diagram_diff_view_code").unwrap();
    cx.simulate_click(code.center(), Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("diff_diagram_container").is_none());
    assert!(cx.debug_bounds("diff_inline").is_some());
    cx.update(|_, app| {
        assert_eq!(
            view.read(app)
                .main_pane
                .read(app)
                .rendered_preview_modes
                .get(RenderedPreviewKind::Diagram),
            RenderedPreviewMode::Source
        )
    });
    let preview = cx.debug_bounds("diagram_diff_view_preview").unwrap();
    cx.simulate_click(preview.center(), Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("diff_diagram_container").is_some());
}

#[test]
fn diagram_commit_and_range_targets_offer_the_same_preview_mode() {
    let path = std::path::PathBuf::from("graph.mmd");
    for target in [
        DiffTarget::commit(gitcomet_core::domain::CommitId("abc".into()), path.clone()),
        DiffTarget::commit_range(
            gitcomet_core::domain::CommitId("abc".into()),
            None,
            Some(path),
        ),
    ] {
        assert_eq!(
            crate::view::diff_target_rendered_preview_kind(Some(&target)),
            Some(RenderedPreviewKind::Diagram)
        );
    }
}
