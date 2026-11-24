use super::*;
use crate::test_support::painted_control_quads;
use crate::view::test_support::NoopBackend;
use gitcomet_state::model::RepoState;
use std::cell::Cell;
use std::rc::Rc;

const ENTRY_COUNT: usize = 200;
/// Tall enough to show a handful of 28px rows, short enough that the list
/// cannot possibly want all `ENTRY_COUNT` of them.
const PANE_HEIGHT_PX: f32 = 300.0;

/// Hosts the pane at a fixed height, the way the bottom panel does — a
/// `uniform_list` with no bounded height would have no visible window to
/// virtualize against.
struct Host {
    pane: Entity<ReflogPaneView>,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().child(
            div()
                .h(px(PANE_HEIGHT_PX))
                .flex()
                .flex_col()
                .child(self.pane.clone()),
        )
    }
}

fn reflog_entries(count: usize) -> Vec<ReflogEntry> {
    (0..count)
        .map(|index| ReflogEntry {
            index,
            new_id: CommitId(Arc::from(format!("{index:040x}").as_str())),
            message: Arc::from(format!("commit: entry {index}").as_str()),
            time: None,
            selector: Arc::from(format!("HEAD@{{{index}}}").as_str()),
            author: Arc::from("Jane Doe"),
        })
        .collect()
}

fn seeded_state(repo_id: RepoId, entries: Vec<ReflogEntry>) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/reflog-pane-test"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.reflog = Loadable::Ready(Arc::new(entries));
    repo.reflog_rev = 1;
    Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    })
}

fn open_pane(
    cx: &mut gpui::TestAppContext,
    entries: Vec<ReflogEntry>,
) -> (Entity<ReflogPaneView>, RepoId, &mut gpui::VisualTestContext) {
    let repo_id = RepoId(1);
    let (store, _events) = AppStore::new_test(Arc::new(NoopBackend));
    let store = Arc::new(store);
    let state = seeded_state(repo_id, entries);

    let (host, cx) = cx.add_window_view(|_window, cx| {
        let ui_model = cx.new(|cx| {
            AppUiModel::new_with_preferences(
                Arc::clone(&state),
                UiPreferences {
                    appearance: crate::view::preferences::AppearancePreferences {
                        date_time_format: DateTimeFormat::YmdHms,
                        timezone: Timezone::Utc,
                        show_timezone: false,
                        ..crate::view::preferences::AppearancePreferences::default()
                    },
                    ..UiPreferences::default()
                },
                Arc::downgrade(&store),
                cx,
            )
        });
        let pane = cx.new(|cx| {
            ReflogPaneView::new(
                store,
                ui_model,
                ReflogPaneInit {
                    root_view: gpui::WeakEntity::new_invalid(),
                },
                cx,
            )
        });
        pane.update(cx, |pane, cx| pane.open(repo_id, cx));
        Host { pane }
    });

    let pane = host.read_with(&*cx, |host, _cx| host.pane.clone());
    (pane, repo_id, cx)
}

/// The regression guard for what made this panel slow: it used to build a
/// row element for every loaded reflog entry on every render. The list must
/// ask for — and paint — only the rows that fit the panel.
#[gpui::test]
fn the_list_builds_only_the_visible_window_not_the_whole_reflog(cx: &mut gpui::TestAppContext) {
    let (pane, repo_id, cx) = open_pane(cx, reflog_entries(ENTRY_COUNT));

    REFLOG_ROWS_BUILT.with(|built| built.set((0, 0)));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (calls, painted) = REFLOG_ROWS_BUILT.with(std::cell::Cell::get);
    assert!(calls > 0, "the row builder should have run");
    assert!(painted > 0, "the visible rows should have been built");

    // A screenful at 28px per row plus overscan — nowhere near all 200.
    let visible_upper_bound = (PANE_HEIGHT_PX / REFLOG_ROW_HEIGHT_PX).ceil() as usize * 3;
    assert!(
        painted <= visible_upper_bound,
        "expected at most {visible_upper_bound} rows for a {PANE_HEIGHT_PX}px panel, \
             built {painted} of {ENTRY_COUNT}"
    );

    // All 200 are still loaded and filtered — only the *rendering* is windowed.
    pane.read_with(&*cx, |pane, _cx| {
        assert_eq!(pane.panels[&repo_id].rows.len(), ENTRY_COUNT);
    });
}

/// The header reads left-to-right as name, then count, then controls: the
/// filter box is parked against the trailing edge, not next to the tab.
#[gpui::test]
fn the_filter_box_sits_at_the_trailing_edge_of_the_header(cx: &mut gpui::TestAppContext) {
    let (_pane, _repo_id, cx) = open_pane(cx, reflog_entries(8));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let tab = cx
        .debug_bounds("reflog_panel_tab")
        .expect("the header should carry the Reflog tab");
    let filter = cx
        .debug_bounds("reflog_panel_filter")
        .expect("the header should carry the filter box");

    assert!(
        filter.left() >= tab.right(),
        "the filter box should follow the tab, not precede it"
    );
    // Flush with the trailing edge, bar the header's own 4px padding —
    // a filter merely sitting *after* the tab would satisfy the check above.
    let viewport = cx.update(|window, _app| window.viewport_size());
    let trailing_gap = viewport.width - filter.right();
    assert!(
        trailing_gap <= px(5.0),
        "the filter box should be flush with the header's trailing edge \
             (gap was {trailing_gap:?})"
    );
}

#[gpui::test]
fn selecting_a_reflog_entry_highlights_only_its_selector(cx: &mut gpui::TestAppContext) {
    let mut entries = reflog_entries(3);
    entries[1].new_id = entries[0].new_id.clone();
    let (_pane, _repo_id, cx) = open_pane(cx, entries);
    crate::test_support::refresh_and_draw(cx);
    let resting = painted_control_quads(cx, "reflog_row-0");
    assert!(!resting.is_empty(), "the first row must be painted");

    for selector in ["reflog_row-0", "reflog_row-1"] {
        let bounds = cx.debug_bounds(selector).expect("row should be visible");
        cx.simulate_click(
            point(bounds.left() + px(35.0), bounds.center().y),
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_move(point(px(1.0), px(1.0)), None, gpui::Modifiers::default());
        crate::test_support::refresh_and_draw(cx);
        let other = if selector == "reflog_row-0" {
            "reflog_row-1"
        } else {
            "reflog_row-0"
        };
        assert_ne!(painted_control_quads(cx, selector), resting);
        assert_eq!(
            painted_control_quads(cx, other),
            resting,
            "an entry pointing to the same commit must stay unselected"
        );
    }
}

#[gpui::test]
fn right_clicking_a_reflog_entry_highlights_only_its_selector(cx: &mut gpui::TestAppContext) {
    let mut entries = reflog_entries(3);
    entries[1].new_id = entries[0].new_id.clone();
    entries[1].time = Some(std::time::UNIX_EPOCH);
    let (pane, repo_id, cx) = open_pane(cx, entries);
    crate::test_support::refresh_and_draw(cx);
    let resting = painted_control_quads(cx, "reflog_row-0");
    assert!(!resting.is_empty(), "the first row must be painted");
    for selector in [
        "reflog_selector-1",
        "reflog_sha-1",
        "reflog_date-1",
        "reflog_message-1",
    ] {
        pane.update(cx, |pane, cx| {
            pane.panels.get_mut(&repo_id).unwrap().selected = None;
            cx.notify();
        });
        crate::test_support::refresh_and_draw(cx);
        let bounds = cx.debug_bounds(selector).unwrap();
        let position = point(bounds.left() + px(2.0), bounds.center().y);
        cx.simulate_mouse_down(position, MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_move(point(px(1.0), px(1.0)), None, gpui::Modifiers::default());
        crate::test_support::refresh_and_draw(cx);

        assert_ne!(
            painted_control_quads(cx, "reflog_row-1"),
            resting,
            "{selector}"
        );
        assert_eq!(
            painted_control_quads(cx, "reflog_row-0"),
            resting,
            "{selector}"
        );
    }
}

#[gpui::test]
fn reflog_message_sha_and_date_can_be_selected_and_copied(cx: &mut gpui::TestAppContext) {
    let mut entries = reflog_entries(1);
    entries[0].new_id = CommitId(Arc::from("abcdef1234567890"));
    entries[0].message = Arc::from("checkout: moving from main to topic");
    entries[0].time = Some(std::time::UNIX_EPOCH);
    let (pane, _repo_id, cx) = open_pane(cx, entries);
    cx.update(|_window, app| {
        app.bind_keys([
            gpui::KeyBinding::new("ctrl-a", crate::kit::SelectAll, Some("TextInput")),
            gpui::KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
        ]);
    });
    crate::test_support::refresh_and_draw(cx);

    for (selector, expected) in [
        ("reflog_message-0", "checkout: moving from main to topic"),
        ("reflog_sha-0", "abcdef12"),
        ("reflog_date-0", "1970-01-01 00:00:00 ("),
    ] {
        pane.update(cx, |_, cx| {
            crate::clipboard::write_text(
                cx,
                String::new(),
                crate::clipboard::CopySource::TextInputShortcut,
            );
        });
        let bounds = cx.debug_bounds(selector).expect("cell must be visible");
        cx.simulate_click(
            point(bounds.left() + px(2.0), bounds.center().y),
            gpui::Modifiers::default(),
        );
        cx.simulate_keystrokes("ctrl-a ctrl-c");
        let copied = pane.update(cx, |_, cx| crate::clipboard::read_text(cx));
        let copied = copied.expect("selecting and copying a cell must write its text");
        if selector == "reflog_date-0" {
            assert!(copied.starts_with(expected), "{copied:?}");
        } else {
            assert_eq!(copied, expected);
        }
    }
}

#[gpui::test]
fn dragging_reflog_column_dividers_resizes_headers_and_rows(cx: &mut gpui::TestAppContext) {
    let (pane, _repo_id, cx) = open_pane(cx, reflog_entries(8));
    crate::test_support::refresh_and_draw(cx);

    for (handle, header, cell) in [
        (
            "reflog_resize_selector",
            "reflog_header_selector",
            "reflog_selector-0",
        ),
        ("reflog_resize_sha", "reflog_header_sha", "reflog_sha-0"),
        ("reflog_resize_date", "reflog_header_date", "reflog_date-0"),
    ] {
        let before = cx.debug_bounds(header).unwrap().size.width;
        let start = cx
            .debug_bounds(handle)
            .expect("column must have a resize handle")
            .center();
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        for delta in [8.0, 24.0] {
            cx.simulate_mouse_move(
                point(start.x + px(delta), start.y),
                Some(MouseButton::Left),
                gpui::Modifiers::default(),
            );
            crate::test_support::refresh_and_draw(cx);
        }
        cx.simulate_mouse_up(
            point(start.x + px(24.0), start.y),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        crate::test_support::refresh_and_draw(cx);
        let after = cx.debug_bounds(header).unwrap().size.width;
        assert_eq!(after, before + px(24.0));
        assert_eq!(cx.debug_bounds(cell).unwrap().size.width, after);
    }

    let resized = cx.debug_bounds("reflog_header_sha").unwrap().size.width;
    pane.update(cx, |pane, cx| pane.set_ui_scale_percent(150, cx));
    crate::test_support::refresh_and_draw(cx);
    assert_eq!(
        cx.debug_bounds("reflog_header_sha").unwrap().size.width,
        resized * 1.5
    );
    pane.update(cx, |pane, cx| pane.set_ui_scale_percent(100, cx));
    crate::test_support::refresh_and_draw(cx);

    for (delta, expected_min) in [(-2000.0, 40.0), (2000.0, 40.0)] {
        let start = cx.debug_bounds("reflog_resize_sha").unwrap().center();
        cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        for offset in [delta / 2.0, delta] {
            cx.simulate_mouse_move(
                point(start.x + px(offset), start.y),
                Some(MouseButton::Left),
                gpui::Modifiers::default(),
            );
            crate::test_support::refresh_and_draw(cx);
        }
        cx.simulate_mouse_up(
            point(start.x + px(delta), start.y),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        crate::test_support::refresh_and_draw(cx);
        assert!(cx.debug_bounds("reflog_header_sha").unwrap().size.width >= px(expected_min));
        assert!(cx.debug_bounds("reflog_header_message").unwrap().size.width >= px(70.0));
    }
}

#[gpui::test]
fn narrow_reflog_columns_keep_the_message_visible_with_large_text(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (_pane, _repo_id, cx) = open_pane(cx, reflog_entries(20));
    cx.simulate_resize(gpui::size(px(495.0), px(400.0)));

    for font_size in [14, 20, 24] {
        let metrics = crate::appearance::Appearance {
            ui_font_size_px: font_size,
            ..Default::default()
        };
        cx.update(|window, app| {
            app.set_global(metrics);
            crate::theme::set_window_theme(
                app,
                window.window_handle().window_id(),
                AppTheme::gitcomet_dark().with_appearance(metrics),
            );
        });
        crate::test_support::refresh_and_draw(cx);
        crate::test_support::refresh_and_draw(cx);
        let message = cx.debug_bounds("reflog_message-0").unwrap();
        let header = cx.debug_bounds("reflog_header_message").unwrap();
        let scale = UiScale::from_percent(100).with_appearance(metrics);
        assert!(message.size.width >= reflog_column(scale, 70.0));
        assert!(
            message.right() <= px(495.0),
            "the message must fit inside the narrow pane at font size {font_size}"
        );
        assert_eq!(
            header.size.height,
            cx.debug_bounds("reflog_header_selector")
                .unwrap()
                .size
                .height
        );
        assert_eq!(
            cx.debug_bounds("reflog_header_sha").unwrap().size.width,
            cx.debug_bounds("reflog_sha-0").unwrap().size.width
        );
    }
}

/// A focused `TextInput` notifies every 800ms from its caret-blink task.
/// The filter observer must ignore those: notifying on them repainted the
/// panel 1.25 times a second for as long as the box held focus.
#[gpui::test]
fn a_blink_notify_with_unchanged_text_does_not_repaint_the_panel(cx: &mut gpui::TestAppContext) {
    let (pane, repo_id, cx) = open_pane(cx, reflog_entries(8));
    let input = pane.read_with(&*cx, |pane, _cx| pane.panels[&repo_id].query_input.clone());

    let notified = Rc::new(Cell::new(0usize));
    let counter = Rc::clone(&notified);
    let subscription = cx.update(|_window, app| {
        app.observe(&pane, move |_pane, _app| {
            counter.set(counter.get() + 1);
        })
    });

    // Exactly what the blink task does: notify without touching the text.
    input.update(cx, |_input, cx| cx.notify());
    assert_eq!(
        notified.get(),
        0,
        "a caret blink must not repaint the reflog panel"
    );

    // A real edit still gets through.
    input.update(cx, |input, cx| input.set_text("entry 3", cx));
    assert!(
        notified.get() > 0,
        "typing in the filter box must repaint the panel"
    );
    pane.read_with(&*cx, |pane, _cx| {
        assert_eq!(pane.panels[&repo_id].query, "entry 3");
    });

    drop(subscription);
}

#[gpui::test]
fn an_open_reflog_waits_for_the_repository_to_finish_reopening(cx: &mut gpui::TestAppContext) {
    let (pane, repo_id, cx) = open_pane(cx, reflog_entries(4));
    pane.update(cx, |pane, cx| {
        let mut state = (*pane.state).clone();
        state.repos[0].reflog = Loadable::NotLoaded;
        state.repos[0].open = Loadable::Loading;
        let opening = pane.notify_fingerprint(&state);
        pane.apply_state_snapshot(Arc::new(state.clone()), cx);
        pane.close(repo_id, cx);
        pane.open(repo_id, cx);
        assert!(pane.is_open(repo_id));
        assert!(pane.request_missing_reflogs().is_empty());
        state.repos[0].open = Loadable::Ready(());
        assert_ne!(pane.notify_fingerprint(&state), opening);
        pane.apply_state_snapshot(Arc::new(state), cx);
        assert_eq!(pane.request_missing_reflogs(), vec![repo_id]);
    });
}

#[gpui::test]
fn a_parked_repository_drops_its_reflog_rows_until_it_reopens(cx: &mut gpui::TestAppContext) {
    let (pane, repo_id, cx) = open_pane(cx, reflog_entries(4));
    pane.update(cx, |pane, cx| {
        assert!(pane.sync_rows(repo_id) > 0);
        let mut state = (*pane.state).clone();
        state.repos[0].set_backend_parked_for_test(true);
        state.repos[0].reflog = Loadable::NotLoaded;
        pane.apply_state_snapshot(Arc::new(state.clone()), cx);
        assert!(pane.panels[&repo_id].rows.is_empty());
        assert!(pane.request_missing_reflogs().is_empty());
        state.repos[0].set_backend_parked_for_test(false);
        pane.apply_state_snapshot(Arc::new(state), cx);
        assert_eq!(pane.request_missing_reflogs(), vec![repo_id]);
    });
}

/// `reload_repo` drops the reflog back to `NotLoaded` on every repo reload;
/// with the panel open that used to leave it on "Loading…" forever, because
/// the only load dispatch was on open.
#[gpui::test]
fn a_reflog_cleared_under_an_open_panel_is_requested_again(cx: &mut gpui::TestAppContext) {
    let (pane, repo_id, cx) = open_pane(cx, reflog_entries(4));

    pane.read_with(&*cx, |pane, _cx| {
        assert!(pane.is_open(repo_id), "the panel should be open");
    });

    // Loaded data needs nothing.
    pane.read_with(&*cx, |pane, _cx| {
        assert!(pane.request_missing_reflogs().is_empty());
    });

    // Stand in for what `reload_repo` does: drop the entries on the floor.
    let cleared = pane.read_with(&*cx, |pane, _cx| {
        let mut state = (*pane.state).clone();
        state.repos[0].reflog = Loadable::NotLoaded;
        state.repos[0].reflog_rev = 2;
        Arc::new(state)
    });
    let requested = pane.update(cx, |pane, cx| {
        pane.apply_state_snapshot(Arc::clone(&cleared), cx);
        pane.request_missing_reflogs()
    });
    assert_eq!(
        requested,
        vec![repo_id],
        "an open panel whose reflog was cleared must ask for it again"
    );

    // An error is not retried: a repo with an unborn HEAD would spin.
    let failed = pane.read_with(&*cx, |pane, _cx| {
        let mut state = (*pane.state).clone();
        state.repos[0].reflog = Loadable::Error("unborn HEAD".to_string());
        state.repos[0].reflog_rev = 3;
        Arc::new(state)
    });
    let requested = pane.update(cx, |pane, cx| {
        pane.apply_state_snapshot(failed, cx);
        pane.request_missing_reflogs()
    });
    assert!(requested.is_empty(), "a failed load must not be retried");

    // Neither is a repo whose panel the user closed.
    let requested = pane.update(cx, |pane, cx| {
        pane.close(repo_id, cx);
        pane.apply_state_snapshot(cleared, cx);
        pane.request_missing_reflogs()
    });
    assert!(
        requested.is_empty(),
        "a closed panel must not keep loading reflogs"
    );
}
