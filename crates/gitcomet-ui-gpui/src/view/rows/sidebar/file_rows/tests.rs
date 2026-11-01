use super::*;

/// Every row pass builds this lookup, three per frame, so it borrows the
/// selection's paths: a copy cost one allocation per selected file. Checked
/// by identity, since allocation counts are process-wide and parallel tests
/// inflate them.
#[gpui::test]
fn the_multi_selection_lookup_borrows_the_selected_paths(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(12);
    let list = crate::view::rows::FileListId::CommitFiles;

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        pane.update(app, |pane, _cx| {
            pane.file_list_selection.insert(
                (repo_id, list),
                crate::view::rows::FileListMultiSelection {
                    paths: ["src/a.rs", "src/b.rs", "src/c.rs"]
                        .map(std::path::PathBuf::from)
                        .to_vec(),
                    anchor: None,
                    anchor_index: None,
                    anchor_order_rev: None,
                },
            );
        });
        let pane = pane.read(app);
        let lookup: rustc_hash::FxHashSet<&std::path::Path> =
            multi_selected_paths(pane, repo_id, list).expect("a multi-selection");
        let selected = pane.commit_list_selected_paths(repo_id, list);
        assert_eq!(lookup.len(), selected.len());
        for path in selected {
            let entry = lookup
                .get(path.as_path())
                .expect("selected path in the lookup");
            assert!(std::ptr::addr_eq(*entry, path.as_path()), "{path:?} copied");
        }
    });
}
