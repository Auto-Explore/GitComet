use super::*;
use gitcomet_extension_api::{
    Extension, ExtensionId, HistoryAnnotator, HistoryRowAnnotation, HostedAction, HostedMenuItem,
    MarkEdge, Registrar, Registry, RowMark, SlotSignal,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::view::test_support::redraw;

/// What the annotators were asked: each menu with its commit and edge, and
/// how many annotations.
#[derive(Default)]
struct Asked {
    menus: RefCell<Vec<(String, MarkEdge)>>,
    annotations: Cell<usize>,
}

/// Two annotators. "agents" marks every commit's end and `c01`'s start and
/// offers a menu, empty on `c02`; "other", registered after it, marks
/// `c03`'s end over it and offers none.
struct Marks(Rc<Asked>);

impl Extension for Marks {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.marks").unwrap()
    }

    fn register(&self, r: &mut Registrar) {
        let asked = self.0.clone();
        let menus = self.0.clone();
        r.history_annotator(
            "agents",
            HistoryAnnotator::new(SlotSignal::default(), move |_, commit, _| {
                asked.annotations.set(asked.annotations.get() + 1);
                let mark = RowMark::new(gpui::green()).with_label("Running");
                let annotation = HistoryRowAnnotation::default().with_trailing(mark.clone());
                if commit.id.as_ref() == "c01" {
                    annotation.with_leading(mark)
                } else {
                    annotation
                }
            })
            .with_mark_menu(move |_, commit, edge, _| {
                menus
                    .menus
                    .borrow_mut()
                    .push((commit.id.as_ref().to_owned(), edge));
                if commit.id.as_ref() == "c02" {
                    return Vec::new();
                }
                vec![
                    HostedMenuItem::Header("Agent".into()),
                    HostedMenuItem::action(HostedAction::new("Bring to front", |_| {})),
                ]
            }),
        );
        r.history_annotator(
            "other",
            HistoryAnnotator::new(SlotSignal::default(), |_, commit, _| {
                if commit.id.as_ref() == "c03" {
                    HistoryRowAnnotation::default()
                        .with_trailing(RowMark::new(gpui::red()).with_label("Other"))
                } else {
                    HistoryRowAnnotation::default()
                }
            }),
        );
    }
}

#[gpui::test]
fn a_right_click_on_a_mark_opens_its_annotators_menu(cx: &mut gpui::TestAppContext) {
    let asked = Rc::new(Asked::default());
    cx.update(|cx| {
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(Marks(asked.clone()))]).unwrap(),
            cx,
        )
    });
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    let commits = (0..6)
        .map(|ix| commit(&format!("c{ix:02}"), &[], &format!("commit {ix:02}")))
        .collect::<Vec<_>>();
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-mark-menu"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "c00")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    });
    // Selecting dispatches into the store, so it holds the repo the view
    // renders.
    store_for_assert.replace_snapshot_for_test(Arc::clone(&state));
    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| model.set_state(Arc::clone(&state), cx));
    });
    cx.run_until_parked();
    ensure_history_cache_for_tests(cx, &view, state);
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_3").is_some()
    });

    let message = cx
        .debug_bounds("history_message_header_cell")
        .expect("the message column");
    let at = |cx: &mut gpui::VisualTestContext, row: &'static str, x: Pixels| {
        point(x, cx.debug_bounds(row).expect(row).center().y)
    };
    // The trailing mark ends at the message column's right edge; the leading
    // one starts just inside its left.
    let trailing = |cx: &mut gpui::VisualTestContext, row| at(cx, row, message.right() - px(3.0));
    let leading = |cx: &mut gpui::VisualTestContext, row| at(cx, row, message.left() + px(8.0));
    let subject = |cx: &mut gpui::VisualTestContext, row| at(cx, row, message.center().x);
    // Pressed at `press`, released at `release` on the same row.
    let right_drag =
        |cx: &mut gpui::VisualTestContext, press: Point<Pixels>, release: Point<Pixels>| {
            cx.simulate_mouse_move(press, None, gpui::Modifiers::default());
            cx.simulate_mouse_down(press, gpui::MouseButton::Right, gpui::Modifiers::default());
            cx.simulate_mouse_move(
                release,
                Some(gpui::MouseButton::Right),
                gpui::Modifiers::default(),
            );
            cx.simulate_mouse_up(
                release,
                gpui::MouseButton::Right,
                gpui::Modifiers::default(),
            );
            cx.run_until_parked();
            redraw(cx);
        };
    let right_click = |cx: &mut gpui::VisualTestContext, position: Point<Pixels>| {
        right_drag(cx, position, position)
    };
    let selected = || {
        store_for_assert
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.clone())
    };
    let opened = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            (
                crate::view::test_support::popover_kind(view.read(app), app),
                view.read(app)
                    .active_context_menu_invoker
                    .as_ref()
                    .map(|invoker| invoker.to_string()),
            )
        })
    };
    let close = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let popover_host = view.read(app).popover_host.clone();
            popover_host.update(app, |host, cx| host.close_popover(cx));
        });
        cx.run_until_parked();
        redraw(cx);
    };
    let commit_menu = |id: &str| {
        Some(PopoverKind::CommitMenu {
            repo_id,
            commit_id: CommitId(id.into()),
        })
    };

    // Drawing asks for annotations, never for a menu.
    let annotations = asked.annotations.get();
    for _ in 0..3 {
        redraw(cx);
    }
    assert!(asked.menus.borrow().is_empty());
    assert_eq!(
        asked.annotations.get(),
        annotations,
        "annotations are cached"
    );

    // The end mark: its annotator's menu, the row showing its menu open.
    let position = trailing(cx, "history_row_0");
    right_click(cx, position);
    let (kind, invoker) = opened(cx);
    assert!(
        matches!(kind, Some(PopoverKind::Hosted { menu: true, .. })),
        "{kind:?}"
    );
    assert_eq!(invoker.as_deref(), Some("history_commit_menu_1_c00"));
    assert_eq!(*asked.menus.borrow(), [("c00".into(), MarkEdge::Trailing)]);
    assert_eq!(
        asked.annotations.get(),
        annotations,
        "the click asks the mark's owner alone, which drawing already found"
    );
    // The row is selected, as for the commit's own menu.
    wait_until(cx, "c00 selected by the right-click", |_| {
        selected() == Some(CommitId("c00".into()))
    });
    close(cx);

    // Where the press lands decides, not the release.
    let press = trailing(cx, "history_row_5");
    let release = subject(cx, "history_row_5");
    right_drag(cx, press, release);
    assert!(matches!(
        opened(cx).0,
        Some(PopoverKind::Hosted { menu: true, .. })
    ));
    assert_eq!(
        asked.menus.borrow().last(),
        Some(&("c05".into(), MarkEdge::Trailing))
    );
    close(cx);

    // The start mark is asked for as the leading edge.
    let position = leading(cx, "history_row_1");
    right_click(cx, position);
    assert!(matches!(
        opened(cx).0,
        Some(PopoverKind::Hosted { menu: true, .. })
    ));
    assert_eq!(
        asked.menus.borrow().last(),
        Some(&("c01".into(), MarkEdge::Leading))
    );
    close(cx);

    // The rest of the row is the commit's.
    let position = subject(cx, "history_row_0");
    right_click(cx, position);
    assert_eq!(opened(cx).0, commit_menu("c00"));
    close(cx);

    // A menu with nothing to choose is the commit's too.
    let position = trailing(cx, "history_row_2");
    right_click(cx, position);
    assert_eq!(opened(cx).0, commit_menu("c02"));
    assert_eq!(
        asked.menus.borrow().last(),
        Some(&("c02".into(), MarkEdge::Trailing)),
        "its menu was asked for"
    );
    close(cx);

    // A mark drawn over another's belongs to the annotator that drew it,
    // which offers no menu: the one under it is not asked.
    let asked_before = asked.menus.borrow().len();
    let position = trailing(cx, "history_row_3");
    right_click(cx, position);
    assert_eq!(opened(cx).0, commit_menu("c03"));
    assert_eq!(asked.menus.borrow().len(), asked_before);
    close(cx);

    // A left click on a mark only selects; nothing opens.
    let position = trailing(cx, "history_row_4");
    cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
    cx.simulate_click(position, gpui::Modifiers::default());
    wait_until(cx, "c04 selected by the click", |_| {
        selected() == Some(CommitId("c04".into()))
    });
    assert_eq!(opened(cx).0, None);
    assert_eq!(asked.menus.borrow().len(), asked_before);
}
