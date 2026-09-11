use super::*;

#[derive(Clone, Copy)]
enum Removal {
    Wheel,
    Programmatic,
    Replace,
    Empty,
    HostClear,
    RemoveSignature,
}

fn tooltip_is_retracted_after_row_removal(
    cx: &mut gpui::TestAppContext,
    area: HistoryRowHoverArea,
    removal: Removal,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let page = Arc::new(log_page(
        (0..80)
            .map(|ix| commit(&format!("c{ix:02}"), &[], &format!("commit {ix:02}")))
            .collect(),
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-row-tooltip-removal"),
        },
    );
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    repo.history_state.commit_signatures = Arc::new(
        [(
            CommitId("c00".into()),
            gitcomet_core::domain::CommitSignature {
                status: gitcomet_core::domain::SignatureStatus::Good,
                format: gitcomet_core::domain::SignatureFormat::Ssh,
                signer: Some("Ada".into()),
                key_id: Some("test-key".into()),
            },
        )]
        .into_iter()
        .collect(),
    );
    repo.history_state.commit_signatures_rev = 1;
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..Default::default()
    });
    set_history_view_state_for_tests(cx, &view, state.clone());
    ensure_history_cache_for_tests(cx, &view, state.clone());
    wait_until(cx, "first history row", |cx| {
        cx.debug_bounds("history_row_0").is_some()
    });
    let row = cx.debug_bounds("history_row_0").unwrap();
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    // Exercise the canvas listener, including its actual cell bounds.
    let mut hover = None;
    let mut x = row.right() - px(4.0);
    while x > row.left() {
        let position = point(x, row.center().y);
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.run_until_parked();
        if cx.update(|_, app| history.read(app).row_hover(app)) == Some((0, area)) {
            hover = Some(position);
            break;
        }
        x -= px(4.0);
    }
    let hover = hover.expect("requested tooltip cell should be hoverable");
    assert!(crate::view::test_support::tooltip_text(cx, &view).is_some());
    if matches!(removal, Removal::HostClear) {
        cx.update(|_, app| {
            let host = history.read(app).tooltip_host.upgrade().unwrap();
            host.update(app, |host, cx| {
                host.clear_tooltip(cx);
            });
        });
        assert_eq!(crate::view::test_support::tooltip_text(cx, &view), None);
        cx.simulate_mouse_move(hover, None, gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(
            crate::view::test_support::tooltip_text(cx, &view).is_some(),
            "hovering the same cell after an external clear must rearm its tooltip"
        );
        return;
    }
    match removal {
        Removal::HostClear => unreachable!(),
        Removal::RemoveSignature => {
            let mut next = (*state).clone();
            next.repos[0].history_state.commit_signatures = Arc::default();
            next.repos[0].history_state.commit_signatures_rev += 1;
            cx.update(|_, app| {
                let ui_model = view.read(app).ui_model.clone();
                ui_model.update(app, |model, cx| model.set_state(Arc::new(next), cx));
            });
        }
        Removal::Wheel => cx.simulate_event(gpui::ScrollWheelEvent {
            position: hover,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-600.0))),
            ..Default::default()
        }),
        Removal::Programmatic => cx.update(|window, app| {
            history.update(app, |history, cx| {
                history
                    .history_scroll
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(point(px(0.0), px(-600.0)));
                cx.notify();
            });
            window.refresh();
        }),
        Removal::Replace | Removal::Empty => {
            let mut next = (*state).clone();
            let repo = &mut next.repos[0];
            let commits = match removal {
                Removal::Replace => vec![commit("replacement", &[], "replacement")],
                _ => Vec::new(),
            };
            let page = Arc::new(log_page(commits, None));
            repo.log = Loadable::Ready(page.clone());
            repo.log_rev += 1;
            repo.history_state.log = Loadable::Ready(page);
            repo.history_state.log_rev += 1;
            let next = Arc::new(next);
            set_history_view_state_for_tests(cx, &view, next.clone());
            ensure_history_cache_for_tests(cx, &view, next);
        }
    }
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    if matches!(removal, Removal::Wheel | Removal::Programmatic) {
        assert!(
            cx.debug_bounds("history_row_0").is_none(),
            "the owning row must leave the viewport"
        );
    }
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        None,
        "removing the owning row must retract its tooltip without a mouse move"
    );
    assert_eq!(cx.update(|_, app| history.read(app).row_hover(app)), None);
    cx.simulate_mouse_move(point(px(1.0), px(1.0)), None, gpui::Modifiers::default());
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        None,
        "the removed row must not rearm the shared tooltip elsewhere"
    );
}

#[gpui::test]
fn date_tooltip_clears_when_its_row_is_wheel_scrolled_away(cx: &mut gpui::TestAppContext) {
    tooltip_is_retracted_after_row_removal(cx, HistoryRowHoverArea::Date, Removal::Wheel);
}

#[gpui::test]
fn signature_tooltip_clears_when_its_row_is_wheel_scrolled_away(cx: &mut gpui::TestAppContext) {
    tooltip_is_retracted_after_row_removal(cx, HistoryRowHoverArea::Signature, Removal::Wheel);
}

#[gpui::test]
fn date_tooltip_clears_when_its_row_is_scrolled_programmatically(cx: &mut gpui::TestAppContext) {
    tooltip_is_retracted_after_row_removal(cx, HistoryRowHoverArea::Date, Removal::Programmatic);
}

#[gpui::test]
fn signature_tooltip_clears_when_its_row_is_replaced(cx: &mut gpui::TestAppContext) {
    tooltip_is_retracted_after_row_removal(cx, HistoryRowHoverArea::Signature, Removal::Replace);
}

#[gpui::test]
fn date_tooltip_clears_when_the_list_becomes_empty(cx: &mut gpui::TestAppContext) {
    tooltip_is_retracted_after_row_removal(cx, HistoryRowHoverArea::Date, Removal::Empty);
}

#[gpui::test]
fn a_row_tooltip_recovers_after_the_host_is_cleared_elsewhere(cx: &mut gpui::TestAppContext) {
    tooltip_is_retracted_after_row_removal(cx, HistoryRowHoverArea::Date, Removal::HostClear);
}

#[gpui::test]
fn removing_a_signature_badge_retracts_its_tooltip_without_mouse_movement(
    cx: &mut gpui::TestAppContext,
) {
    tooltip_is_retracted_after_row_removal(
        cx,
        HistoryRowHoverArea::Signature,
        Removal::RemoveSignature,
    );
}
