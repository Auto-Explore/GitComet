use super::*;

#[gpui::test]
fn opening_a_prefetched_file_reuses_its_structural_rows(cx: &mut gpui::TestAppContext) {
    let dir = rust_repository();
    let base = head(dir.path());
    git(dir.path(), &["commit", "-qam", "change"]);
    let tip = head(dir.path());
    let target = |path: &str| {
        gitcomet_core::domain::DiffTarget::commit_range(
            gitcomet_core::domain::CommitId(base.as_str().into()),
            Some(gitcomet_core::domain::CommitId(tip.as_str().into())),
            Some(path.into()),
        )
    };
    let (_dir, store, view, cx) = open_repository_in(cx, Arc::new(TestBackend), dir);
    let pane = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        host.create_diff_pane(
            &host.active_repository(app).unwrap().unwrap(),
            target("a.rs"),
            DiffPaneOptions {
                engine_override: Some(gitcomet_core::diff_engine::DiffEngine::Difftastic),
                ..Default::default()
            },
            app,
        )
        .unwrap()
    });
    settle(cx, &view, &store, "first file", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let entity = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap();
    cx.update(|_, app| pane.prefetch_target(Some(target("b.rs")), app));
    settle(cx, &view, &store, "upcoming file's structural rows", |cx| {
        cx.update(|_, app| entity.read(app).prefetched_rows().is_some())
    });
    let prepared = cx.update(|_, app| {
        assert_eq!(pane.target(app), Some(target("a.rs")));
        let rows = entity.read(app).prefetched_rows().unwrap();
        pane.set_target(target("b.rs"), app);
        rows
    });
    settle(cx, &view, &store, "second file", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let renderer = hosted_renderer(cx, &pane);
    cx.update(|_, app| {
        let shown = renderer.read(app).file_diff_row_provider.as_ref().unwrap();
        assert!(Arc::ptr_eq(&prepared, shown));
    });
}
