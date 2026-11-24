use super::*;

#[gpui::test]
fn one_sided_hosted_comparisons_fill_the_pane_and_keep_line_actions(
    test_cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::DiffLayout;

    let (dir, store, view, cx) = open_repository(test_cx);
    let base = CommitId(head(dir.path()).into());
    std::fs::remove_file(dir.path().join("b.rs")).unwrap();
    std::fs::write(dir.path().join("added.mmd"), "graph TD\n A --> B\n").unwrap();
    std::fs::write(dir.path().join("added.md"), "# Added document\n").unwrap();
    std::fs::write(
        dir.path().join("new.rs"),
        "first new line\nsecond new line\n",
    )
    .unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "review changes"]);
    let tip = CommitId(head(dir.path()).into());
    let target =
        |path: &str| DiffTarget::commit_range(base.clone(), Some(tip.clone()), Some(path.into()));
    let clicks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (pane, list) = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        let clicked = clicks.clone();
        let pane = host
            .create_diff_pane(
                &repository,
                target("a.rs"),
                DiffPaneOptions {
                    layout: DiffLayout::Split,
                    on_gutter_click: Some(std::rc::Rc::new(move |side, line, _| {
                        clicked.borrow_mut().push((side, line))
                    })),
                    ..Default::default()
                },
                app,
            )
            .unwrap();
        let list = host
            .create_file_list(
                &repository,
                ChangeSource::comparison(base.clone(), Some(tip.clone()), Default::default()),
                |_, _, _| {},
                app,
            )
            .unwrap();
        (pane, list)
    });
    settle(cx, &view, &store, "hosted comparison", |cx| {
        cx.update(|_, app| !list.is_loading(app) && !pane.is_loading(app))
    });
    let mut list = Some(list);
    let (_, pane_cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
    let id = pane_cx.update(|_, app| {
        pane.view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap()
            .read(app)
            .view_id()
    });
    for (layout, mode) in [
        (DiffLayout::Split, DiffViewMode::Split),
        (DiffLayout::Inline, DiffViewMode::Inline),
    ] {
        pane_cx.update(|_, app| pane.set_layout(layout, app));
        for (path, expected) in [
            ("new.rs", Some(DiffLineSide::New)),
            ("b.rs", Some(DiffLineSide::Old)),
            ("a.rs", None),
        ] {
            pane_cx.update(|_, app| pane.set_target(target(path), app));
            settle(pane_cx, &view, &store, path, |cx| {
                cx.update(|_, app| !pane.is_loading(app))
            });
            if let Some(list) = list.take() {
                drop(list);
                settle(pane_cx, &view, &store, "closed companion list", |_| {
                    store.snapshot().repos[0].change_lists.is_empty()
                });
            }
            crate::view::test_support::redraw(pane_cx);
            pane_cx.run_until_parked();
            assert_eq!(
                pane_cx.debug_bounds("diff_inline").is_some(),
                expected.is_none(),
                "{path} {layout:?}"
            );
            if let Some(side) = expected {
                let bounds = pane_cx
                    .debug_bounds(selector(format!("hosted_diff_{id}_row_0")))
                    .expect("commentable row");
                let width = pane_cx.update(|window, _| window.viewport_size().width);
                assert!(
                    bounds.size.width > width * 0.9,
                    "{path} {layout:?}: {bounds:?}, viewport {width:?}"
                );
                click_with(
                    pane_cx,
                    selector(format!("hosted_diff_{id}_gutter_0")),
                    Default::default(),
                );
                pane_cx.run_until_parked();
                assert_eq!(clicks.borrow_mut().pop(), Some((side, 1)));
            } else {
                let renderer = hosted_renderer(pane_cx, &pane);
                pane_cx.update(|_, app| assert_eq!(renderer.read(app).diff_view, mode));
            }
        }
    }
    let renderer = hosted_renderer(pane_cx, &pane);
    pane_cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer.set_diff_content_mode(DiffContentMode::Collapsed, cx);
            for kind in [RenderedPreviewKind::Diagram, RenderedPreviewKind::Markdown] {
                renderer
                    .rendered_preview_modes
                    .set(kind, RenderedPreviewMode::Rendered);
            }
            cx.notify();
        })
    });
    for (path, body) in [
        ("added.mmd", "diff_diagram_single"),
        ("added.md", "diff_markdown_preview_container"),
    ] {
        pane_cx.update(|_, app| pane.set_target(target(path), app));
        settle(pane_cx, &view, &store, path, |cx| {
            cx.debug_bounds(body).is_some()
        });
        assert!(pane_cx.debug_bounds("diff_diagram_left").is_none());
        assert!(pane_cx.debug_bounds("diff_split").is_none());
    }
}
