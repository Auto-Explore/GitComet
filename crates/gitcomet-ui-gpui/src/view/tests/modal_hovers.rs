use super::*;

#[derive(Clone, Copy, Debug)]
enum Modal {
    Palette,
    Shortcuts,
    GoTo,
    BranchPicker,
    ChildDialog,
    Documents,
}

impl Modal {
    const ALL: [Self; 6] = [
        Self::Palette,
        Self::Shortcuts,
        Self::GoTo,
        Self::BranchPicker,
        Self::ChildDialog,
        Self::Documents,
    ];

    fn open(
        self,
        view: &mut GitCometView,
        window: &mut Window,
        cx: &mut gpui::Context<GitCometView>,
    ) {
        let branch_picker = PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::Checkout,
        };
        match self {
            Self::Palette => view.open_command_palette(window, cx),
            Self::Shortcuts => view.toggle_shortcut_help(window, cx),
            Self::GoTo => view.open_reveal_commit(window, cx),
            Self::BranchPicker => view.open_popover_centered(branch_picker, window, cx),
            Self::ChildDialog => view.popover_host.update(cx, |host, cx| {
                host.open_popover_centered(branch_picker, window, cx);
            }),
            Self::Documents => view.toggle_document_picker(source_bounds(), window, cx),
        }
    }

    fn close(
        self,
        view: &mut GitCometView,
        window: &mut Window,
        cx: &mut gpui::Context<GitCometView>,
    ) {
        match self {
            Self::Palette | Self::Shortcuts => view.close_command_palette(window, cx),
            Self::GoTo => view.close_reveal_commit(window, cx),
            Self::BranchPicker | Self::ChildDialog => view.popover_host.update(cx, |host, cx| {
                host.close_popover(cx);
            }),
            Self::Documents => view.toggle_document_picker(source_bounds(), window, cx),
        }
    }
}

fn source_bounds() -> Bounds<Pixels> {
    Bounds::new(point(px(300.0), px(220.0)), gpui::size(px(200.0), px(28.0)))
}

fn arm_hovers(view: &mut GitCometView, cx: &mut gpui::Context<GitCometView>) {
    let bounds = source_bounds();
    view.show_commit_message_hover(
        commit_message_hover::CommitMessageHoverState {
            repo_id: RepoId(1),
            commit_id: CommitId("tip".into()),
            summary: "A commit message".into(),
            author: "A contributor".into(),
            when: "Today".into(),
            source_bounds: bounds,
            source_pointer_x: bounds.center().x,
        },
        bounds.center(),
        cx,
    );
    view.show_history_refs_hover(
        RepoId(1),
        CommitId("tip".into()),
        bounds,
        vec![HistoryRefListItem {
            text: HistoryTextVm::new("main".into()),
            kind: HistoryRefListItemKind::LocalBranch {
                name: "main".into(),
            },
        }]
        .into(),
        bounds.center(),
        cx,
    );
}

fn check_modal_hovers(cx: &mut gpui::TestAppContext, modal: Modal, pending: bool) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(1))));
    sync_view_snapshot(cx, &view);

    cx.update(|_, app| {
        view.update(app, |view, cx| {
            crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
                arm_hovers(view, cx)
            });
            view.tooltip_host.update(cx, |host, cx| {
                host.set_tooltip_text_if_changed(Some("Background tooltip".into()), cx);
            });
        })
    });
    if !pending {
        cx.executor().advance_clock(Duration::from_secs(1));
        test_support::redraw(cx);
        assert!(cx.debug_bounds("commit_message_hover").is_some());
        cx.update(|_, app| {
            assert!(
                view.read(app)
                    .history_refs_hover_host
                    .read(app)
                    .is_open_for_tests()
            )
        });
    }

    cx.update(|window, app| view.update(app, |view, cx| modal.open(view, window, cx)));
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("commit_message_hover").is_none(),
        "{modal:?}"
    );
    cx.update(|_, app| {
        assert!(
            !view
                .read(app)
                .history_refs_hover_host
                .read(app)
                .is_open_for_tests(),
            "{modal:?}"
        );
    });
    assert_eq!(test_support::tooltip_text(cx, &view), None, "{modal:?}");

    if pending {
        // Closing before the original delay expires must not resurrect a card.
        cx.update(|window, app| view.update(app, |view, cx| modal.close(view, window, cx)));
        test_support::redraw(cx);
        cx.executor().advance_clock(Duration::from_secs(1));
        test_support::redraw(cx);
        assert!(
            cx.debug_bounds("commit_message_hover").is_none(),
            "{modal:?}"
        );
        cx.update(|_, app| {
            assert!(
                !view
                    .read(app)
                    .history_refs_hover_host
                    .read(app)
                    .is_open_for_tests()
            )
        });
        return;
    }

    // Window-level history listeners can still fire behind a modal.
    cx.update(|_, app| view.update(app, |view, cx| arm_hovers(view, cx)));
    cx.executor().advance_clock(Duration::from_secs(1));
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("commit_message_hover").is_none(),
        "{modal:?}"
    );
    cx.update(|_, app| {
        assert!(
            !view
                .read(app)
                .history_refs_hover_host
                .read(app)
                .is_open_for_tests()
        )
    });
    cx.update(|window, app| view.update(app, |view, cx| modal.close(view, window, cx)));
    cx.executor().advance_clock(Duration::from_secs(1));
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("commit_message_hover").is_none(),
        "stale hover after {modal:?}"
    );

    // Dismissal must allow fresh hovers after the modal closes.
    cx.update(|_, app| view.update(app, |view, cx| arm_hovers(view, cx)));
    cx.executor().advance_clock(Duration::from_secs(1));
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("commit_message_hover").is_some(),
        "{modal:?}"
    );
}

#[gpui::test]
fn opening_modals_retracts_background_hovers_until_the_modal_closes(cx: &mut gpui::TestAppContext) {
    for modal in Modal::ALL {
        check_modal_hovers(cx, modal, false);
    }
}

#[gpui::test]
fn opening_modals_cancels_pending_hover_timers(cx: &mut gpui::TestAppContext) {
    for modal in Modal::ALL {
        check_modal_hovers(cx, modal, true);
    }
}

#[gpui::test]
fn keyboard_modal_dismisses_gpui_managed_tooltip_without_mouse_movement(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);
    let button = cx.debug_bounds("home_open_repo_action").unwrap();
    cx.simulate_mouse_move(button.center(), None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert!(test_support::tooltip_text(cx, &view).is_some());
    cx.update(|window, app| view.update(app, |view, cx| view.open_command_palette(window, cx)));
    test_support::redraw(cx);
    assert_eq!(test_support::tooltip_text(cx, &view), None);
}
