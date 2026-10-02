use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::RepoSpec;

const REPO: RepoId = RepoId(7);

fn files_state(mode: SidebarMode) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        REPO,
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-explorer-settings"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.file_browser.active = true;
    repo.file_browser.entries = Loadable::Ready(Arc::new(vec![FileEntry {
        name: "main.rs".to_string(),
        path: Arc::new(PathBuf::from("main.rs")),
        kind: FileEntryKind::File,
        depth: 0,
        ignored: false,
    }]));
    repo.file_browser.bump_rev();
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        sidebar_mode: mode,
        ..Default::default()
    })
}

fn sidebar_window(
    cx: &mut gpui::TestAppContext,
    state: Arc<AppState>,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, state, cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    (view, cx)
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not painted"));
    cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
}

#[gpui::test]
fn files_settings_cog_sits_between_search_and_locate_on_the_files_tab_only(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = sidebar_window(cx, files_state(SidebarMode::Files));

    let search = cx.debug_bounds("sidebar_search_toggle").unwrap();
    let cog = cx
        .debug_bounds("sidebar_explorer_settings")
        .expect("the Files tab strip shows the settings cog");
    let locate = cx.debug_bounds("sidebar_locate_open_file").unwrap();
    assert!(
        search.right() <= cog.left() && cog.right() <= locate.left(),
        "search {search:?}, cog {cog:?}, locate {locate:?}"
    );
    assert_eq!(cog.center().y, search.center().y);

    let state = files_state(SidebarMode::Branches);
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, state, cx);
        })
    });
    test_support::redraw(cx);
    assert!(cx.debug_bounds("sidebar_explorer_settings").is_none());
    assert!(cx.debug_bounds("sidebar_locate_active_branch").is_some());
}

#[gpui::test]
fn files_settings_menu_toggles_ignored_files_and_stays_open(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = sidebar_window(cx, files_state(SidebarMode::Files));

    click(cx, "sidebar_explorer_settings");
    let menu = PopoverKind::ExplorerSettingsMenu { repo_id: REPO };
    cx.update(|_window, app| {
        assert_eq!(
            test_support::popover_kind(view.read(app), app),
            Some(menu.clone())
        );
    });
    assert!(
        cx.debug_bounds("context_menu_entry_icon_Show hidden files")
            .is_some(),
        "hidden files are shown by default"
    );
    assert!(
        cx.debug_bounds("context_menu_entry_icon_Show ignored files")
            .is_none()
    );

    click(cx, "context_menu_show_ignored_files");
    test_support::drain_store_worker(&view, cx);
    cx.update(|_window, app| {
        view.update(app, |view, cx| test_support::sync_store_snapshot(view, cx))
    });
    test_support::redraw(cx);

    cx.update(|_window, app| {
        let view = view.read(app);
        let browser = &view.store.snapshot().repos[0].file_browser;
        assert!(browser.show_ignored && browser.show_hidden);
        assert_eq!(test_support::popover_kind(view, app), Some(menu));
    });
    assert!(
        cx.debug_bounds("context_menu_entry_icon_Show ignored files")
            .is_some(),
        "the open menu repaints its check mark"
    );
}

fn visibility_state(show_hidden: bool) -> Arc<AppState> {
    let entry = |name: &str, kind, ignored| FileEntry {
        name: name.to_string(),
        path: Arc::new(PathBuf::from(name)),
        kind,
        depth: 0,
        ignored,
    };
    let mut state = (*files_state(SidebarMode::Files)).clone();
    let browser = &mut state.repos[0].file_browser;
    browser.entries = Loadable::Ready(Arc::new(vec![
        entry(".github", FileEntryKind::Directory, false),
        entry("ignored.log", FileEntryKind::File, true),
        entry("plain.rs", FileEntryKind::File, false),
    ]));
    browser.show_hidden = show_hidden;
    browser.show_ignored = true;
    browser.bump_rev();
    Arc::new(state)
}

#[gpui::test]
fn tinted_rows_lay_out_like_plain_rows_and_hidden_ones_go_with_the_setting(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = sidebar_window(cx, visibility_state(true));
    for scale in [100, 150] {
        cx.update(|window, app| {
            ui_scale::set_default(app, scale);
            ui_scale::apply_to_window(window, scale);
            window.refresh();
        });
        test_support::redraw(cx);
        let labels = ["explorer_label_0", "explorer_label_1", "explorer_label_2"].map(|selector| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is not painted at {scale}%"))
        });
        assert!(
            labels
                .iter()
                .all(|label| label.size.height == labels[2].size.height),
            "{labels:?} at {scale}%"
        );
    }

    // Like the reducer, move the rev: the visible-row cache keys on it.
    let mut state = (*visibility_state(false)).clone();
    state.repos[0].file_browser.bump_rev();
    let state = Arc::new(state);
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, state, cx);
        })
    });
    test_support::redraw(cx);
    assert!(cx.debug_bounds("explorer_label_1").is_some());
    assert!(
        cx.debug_bounds("explorer_label_2").is_none(),
        ".github must leave the tree when hidden files are off"
    );
}
