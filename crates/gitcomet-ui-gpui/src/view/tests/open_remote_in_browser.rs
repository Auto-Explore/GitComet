//! "Open remote in web browser": the decision, the picker, and the surfaces that
//! reach it. There is no global override for the real opener, so no test here
//! may reach it with exactly one web page — that would launch the browser. The
//! chord and palette tests use two web remotes (picker) or none (warning).

use super::*;
use crate::view::permalink::RemoteWebPage;

fn remote(name: &str, url: Option<&str>) -> Remote {
    Remote {
        name: name.to_string(),
        url: url.map(str::to_string),
    }
}

fn ready(remotes: Vec<Remote>) -> Loadable<Arc<Vec<Remote>>> {
    Loadable::Ready(Arc::new(remotes))
}

fn page(remote: &str, url: &str) -> RemoteWebPage {
    RemoteWebPage {
        remote: remote.to_string(),
        url: url.to_string(),
    }
}

/// Two remotes on different repositories: the picker case.
fn two_web_remotes() -> Vec<Remote> {
    vec![
        remote("origin", Some("git@github.com:Auto-Explore/GitComet.git")),
        remote("upstream", Some("https://gitlab.com/upstream/GitComet.git")),
    ]
}

#[test]
fn remote_web_request_waits_for_the_remotes_to_load() {
    for remotes in [
        Loadable::NotLoaded,
        Loadable::Loading,
        Loadable::Error("boom".to_string()),
    ] {
        let repo = repo_with_push_state(None, remotes);
        assert_eq!(remote_web_request(&repo), RemoteWebRequest::NotReady);
    }
}

#[test]
fn remote_web_request_tells_no_remotes_from_no_web_page() {
    let none = repo_with_push_state(None, ready(Vec::new()));
    assert_eq!(remote_web_request(&none), RemoteWebRequest::NoRemotes);

    let local_only = repo_with_push_state(
        None,
        ready(vec![
            remote("origin", Some("/srv/git/repo.git")),
            remote("mirror", None),
        ]),
    );
    assert_eq!(remote_web_request(&local_only), RemoteWebRequest::NoWebPage);
}

#[test]
fn remote_web_request_opens_the_only_web_page_directly() {
    let repo = repo_with_push_state(
        None,
        ready(vec![
            remote("local", Some("/srv/git/repo.git")),
            remote("origin", Some("git@github.com:Auto-Explore/GitComet.git")),
        ]),
    );
    assert_eq!(
        remote_web_request(&repo),
        RemoteWebRequest::Open(page("origin", "https://github.com/Auto-Explore/GitComet"))
    );
}

#[test]
fn remote_web_request_offers_a_choice_with_origin_first() {
    let repo = repo_with_push_state(
        None,
        ready(vec![
            remote("backup", Some("https://gitlab.com/org/backup.git")),
            remote("origin", Some("git@github.com:org/repo.git")),
        ]),
    );
    assert_eq!(
        remote_web_request(&repo),
        RemoteWebRequest::Choose(vec![
            page("origin", "https://github.com/org/repo"),
            page("backup", "https://gitlab.com/org/backup"),
        ])
    );
}

#[test]
fn remote_web_request_opens_directly_when_two_remotes_share_a_page() {
    // ssh and https remotes of one repository: nothing to choose between.
    let repo = repo_with_push_state(
        None,
        ready(vec![
            remote("https", Some("https://github.com/org/repo.git")),
            remote("origin", Some("git@github.com:org/repo.git")),
        ]),
    );
    assert_eq!(
        remote_web_request(&repo),
        RemoteWebRequest::Open(page("origin", "https://github.com/org/repo"))
    );
}

#[test]
fn remote_web_request_skips_remotes_without_a_url() {
    let repo = repo_with_push_state(
        None,
        ready(vec![
            remote("broken", None),
            remote("upstream", Some("https://gitlab.com/org/repo.git")),
        ]),
    );
    assert_eq!(
        remote_web_request(&repo),
        RemoteWebRequest::Open(page("upstream", "https://gitlab.com/org/repo"))
    );
}

#[test]
fn each_unavailable_request_explains_its_own_case() {
    for (request, reason_says, message_says) in [
        (RemoteWebRequest::NotReady, "loading", "loading"),
        (RemoteWebRequest::NoRemotes, "remote first", "no remotes"),
        (RemoteWebRequest::NoWebPage, "web page", "web page"),
    ] {
        let reason = request.unavailable_reason().unwrap_or_default();
        let message = request.unavailable_message().unwrap_or_default();
        assert!(reason.contains(reason_says), "{request:?}: {reason}");
        assert!(message.contains(message_says), "{request:?}: {message}");
    }
}

#[test]
fn only_unavailable_requests_explain_themselves() {
    for request in [
        RemoteWebRequest::NotReady,
        RemoteWebRequest::NoRemotes,
        RemoteWebRequest::NoWebPage,
    ] {
        assert!(request.unavailable_reason().is_some(), "{request:?}");
        assert!(request.unavailable_message().is_some(), "{request:?}");
    }
    for request in [
        RemoteWebRequest::Open(page("origin", "https://github.com/org/repo")),
        RemoteWebRequest::Choose(vec![
            page("origin", "https://github.com/org/repo"),
            page("backup", "https://gitlab.com/org/backup"),
        ]),
    ] {
        assert_eq!(request.unavailable_reason(), None, "{request:?}");
        assert_eq!(request.unavailable_message(), None, "{request:?}");
    }
}

fn remote_browser_view(
    cx: &mut gpui::TestAppContext,
    remotes: Loadable<Arc<Vec<Remote>>>,
) -> (
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    install_app_shortcuts_for_test(cx, backend);
    set_remotes(&store, &view, cx, remotes);
    (store, view, cx)
}

fn set_remotes(
    store: &AppStore,
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    remotes: Loadable<Arc<Vec<Remote>>>,
) {
    // Bump the rev like the store does, so open popovers see the change.
    let rev = store
        .snapshot()
        .repos
        .first()
        .map_or(0, |repo| repo.remotes_rev);
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.git_runtime = available_git_runtime_state();
    state.repos[0].remotes = remotes;
    state.repos[0].remotes_rev = rev.wrapping_add(1);
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, view);
}

fn picker_is_open(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> bool {
    let picker = PopoverKind::remote(RepoId(1), RemotePopoverKind::OpenInBrowserMenu);
    cx.update(|_window, app| view.read(app).popover_host.read(app).is_kind_open(&picker))
}

fn toasts(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Vec<(components::ToastKind, String)> {
    cx.update(|_window, app| view.read(app).toast_host.read(app).toasts_for_tests(app))
}

type Opened = Arc<Mutex<Vec<String>>>;

/// Drive the real entry point with a recording opener standing in for the
/// browser, so assertions describe a page that was actually handed over.
fn open_with_stub(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> Opened {
    let opened = Opened::default();
    let recorder = Arc::clone(&opened);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.open_remote_in_browser_with(window, cx, move |url| {
                recorder
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(url);
                Ok(())
            });
        });
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    opened
}

fn opened_urls(opened: &Opened) -> Vec<String> {
    opened
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[gpui::test]
fn the_only_web_page_is_handed_to_the_browser(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = remote_browser_view(
        cx,
        ready(vec![
            remote("local", Some("/srv/git/repo.git")),
            remote("origin", Some("git@github.com:Auto-Explore/GitComet.git")),
        ]),
    );

    let opened = open_with_stub(cx, &view);

    assert_eq!(
        opened_urls(&opened),
        ["https://github.com/Auto-Explore/GitComet"]
    );
    assert!(
        !picker_is_open(cx, &view),
        "a single web page opens without a picker"
    );
    assert!(
        toasts(cx, &view).is_empty(),
        "a launch that worked says nothing"
    );
}

#[gpui::test]
fn several_web_pages_open_a_picker_with_origin_selected(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = remote_browser_view(cx, ready(two_web_remotes()));

    let opened = open_with_stub(cx, &view);

    assert!(
        opened_urls(&opened).is_empty(),
        "which page opens is the user's call"
    );
    assert!(picker_is_open(cx, &view), "expected the remote picker");
    assert!(
        cx.debug_bounds("modal_scrim").is_some(),
        "the centered picker sits on the shared modal scrim"
    );
    assert!(cx.debug_bounds("open_remote_in_browser_0").is_some());
    assert!(cx.debug_bounds("open_remote_in_browser_1").is_some());
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.context_menu_focus_handle_for_tests()
                .is_focused(window),
            "the picker takes the keyboard"
        );
        // Header, separator, then origin's row: Enter opens origin.
        assert_eq!(host.context_menu_selected_ix_for_tests(), Some(2));
    });
}

#[gpui::test]
fn nothing_to_open_warns_instead_of_launching(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, view, cx) = remote_browser_view(cx, ready(Vec::new()));

    for (ix, (remotes, message)) in [
        (
            ready(Vec::new()),
            "This repository has no remotes to open in a web browser.",
        ),
        (
            ready(vec![remote("origin", Some("/srv/git/repo.git"))]),
            "None of this repository's remote URLs points to a web page.",
        ),
        (
            Loadable::Loading,
            "This repository's remotes are still loading.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        set_remotes(&store, &view, cx, remotes);
        let opened = open_with_stub(cx, &view);

        assert!(opened_urls(&opened).is_empty(), "{message}");
        assert!(!picker_is_open(cx, &view), "{message}");
        let toasts = toasts(cx, &view);
        assert_eq!(toasts.len(), ix + 1, "one toast per attempt: {toasts:?}");
        assert_eq!(
            toasts.last(),
            Some(&(components::ToastKind::Warning, message.to_string()))
        );
    }
}

#[gpui::test]
fn without_a_repository_nothing_happens(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(backend);
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let opened = open_with_stub(cx, &view);

    assert!(opened_urls(&opened).is_empty());
    assert!(toasts(cx, &view).is_empty());
    cx.update(|_window, app| {
        assert!(!test_support::popover_is_open(view.read(app), app));
    });
}

fn press(cx: &mut gpui::VisualTestContext, keystroke: &str) {
    cx.simulate_keystrokes(keystroke);
    test_support::redraw(cx);
}

#[gpui::test]
fn secondary_k_toggles_the_picker(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = remote_browser_view(cx, ready(two_web_remotes()));
    // Nothing focused: only the app-level handler can hear the chord.
    focus_detached_window_focus(cx);

    press(cx, "secondary-k");
    assert!(
        picker_is_open(cx, &view),
        "expected secondary-k to open the picker"
    );

    // Now from inside the picker, which holds the keyboard.
    press(cx, "secondary-k");
    assert!(
        !picker_is_open(cx, &view),
        "expected secondary-k to close it again"
    );

    press(cx, "secondary-k");
    assert!(
        picker_is_open(cx, &view),
        "expected it to reopen after a toggle"
    );
    press(cx, "escape");
    assert!(
        !picker_is_open(cx, &view),
        "expected escape to close the picker"
    );
}

#[gpui::test]
fn secondary_k_in_the_command_palette_swaps_it_for_the_picker(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = remote_browser_view(cx, ready(two_web_remotes()));

    press(cx, "secondary-p");
    assert!(command_palette_is_open(cx, &view));

    // The palette's text input has focus and must let the chord through.
    press(cx, "secondary-k");
    assert!(
        !command_palette_is_open(cx, &view),
        "two modal scrims must never stack"
    );
    assert!(
        picker_is_open(cx, &view),
        "expected the picker in its place"
    );
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.context_menu_focus_handle_for_tests()
                .is_focused(window),
            "the palette handing focus back must not take it from the picker"
        );
    });
}

#[gpui::test]
fn secondary_k_warns_once_per_press_without_a_web_remote(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = remote_browser_view(cx, ready(Vec::new()));
    let warning = (
        components::ToastKind::Warning,
        "This repository has no remotes to open in a web browser.".to_string(),
    );

    // From a focused text input (the palette's) and from nothing focused.
    press(cx, "secondary-p");
    press(cx, "secondary-k");
    assert_eq!(toasts(cx, &view), std::slice::from_ref(&warning));

    press(cx, "escape");
    focus_detached_window_focus(cx);
    press(cx, "secondary-k");
    assert_eq!(toasts(cx, &view), [warning.clone(), warning]);
}

#[gpui::test]
fn the_palette_command_opens_the_picker_and_tracks_the_remotes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, view, cx) = remote_browser_view(cx, ready(two_web_remotes()));
    let unavailable = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .command_palette_context(app)
                .remote_web_page_unavailable
        })
    };

    assert_eq!(unavailable(cx), None);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("open-remote-in-browser", Some(window), cx);
        });
    });
    test_support::redraw(cx);
    assert!(
        picker_is_open(cx, &view),
        "expected the palette to open the picker"
    );

    set_remotes(
        &store,
        &view,
        cx,
        ready(vec![remote("origin", Some("/srv/git/repo.git"))]),
    );
    assert_eq!(unavailable(cx), Some("No remote URL points to a web page"));
}

#[gpui::test]
fn a_failed_launch_is_reported_in_the_error_banner(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, view, cx) = remote_browser_view(
        cx,
        ready(vec![remote("origin", Some("git@github.com:org/repo.git"))]),
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.open_remote_in_browser_with(window, cx, |_url| {
                Err(std::io::Error::other("no browser installed"))
            });
        });
    });

    pump_until(cx, "the launch failure to reach the banner", || {
        store
            .snapshot()
            .banner_error
            .as_ref()
            .is_some_and(|banner| {
                banner.message == "Failed to open link: no browser installed"
                    && banner.repo_id == Some(RepoId(1))
            })
    });
    assert!(
        toasts(cx, &view).is_empty(),
        "an error goes to the banner, not a toast"
    );
}

#[gpui::test]
fn the_picker_replaces_an_open_go_to_dialog(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = remote_browser_view(cx, ready(two_web_remotes()));
    press(cx, "secondary-g");
    assert!(
        reveal_commit_is_open(cx, &view),
        "expected the Go to dialog"
    );

    let opened = open_with_stub(cx, &view);

    assert!(opened_urls(&opened).is_empty());
    assert!(
        !reveal_commit_is_open(cx, &view),
        "two modal scrims must never stack"
    );
    assert!(
        picker_is_open(cx, &view),
        "expected the picker in its place"
    );
}

#[gpui::test]
fn an_open_picker_follows_the_remotes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, view, cx) = remote_browser_view(cx, ready(two_web_remotes()));
    let _ = open_with_stub(cx, &view);
    assert!(cx.debug_bounds("open_remote_in_browser_1").is_some());
    assert!(cx.debug_bounds("open_remote_in_browser_2").is_none());

    // A remote added while the picker is up gets its row without reopening:
    // the rows are rebuilt from state, and the fingerprint hashes the rev.
    let mut remotes = two_web_remotes();
    remotes.push(remote(
        "backup",
        Some("https://codeberg.org/org/backup.git"),
    ));
    set_remotes(&store, &view, cx, ready(remotes));

    assert!(picker_is_open(cx, &view));
    assert!(
        cx.debug_bounds("open_remote_in_browser_2").is_some(),
        "expected the new remote's row"
    );
}

#[gpui::test]
fn secondary_k_without_a_repository_does_nothing(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(Arc::clone(&backend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    install_app_shortcuts_for_test(cx, backend);
    focus_detached_window_focus(cx);

    press(cx, "secondary-k");

    assert!(toasts(cx, &view).is_empty());
    cx.update(|_window, app| {
        assert!(!test_support::popover_is_open(view.read(app), app));
    });
}
