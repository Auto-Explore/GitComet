use super::*;
use gitcomet_extension_api::*;
use std::cell::Cell;
use std::rc::Rc;

struct Providers {
    sidebar: Rc<Cell<usize>>,
    history: Rc<Cell<usize>>,
    signal: SlotSignal,
}

impl Extension for Providers {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.signals").unwrap()
    }

    fn register(&self, r: &mut Registrar) {
        let sidebar = self.sidebar.clone();
        r.sidebar_provider(
            "files",
            SidebarProvider::new(self.signal.clone(), move |_, _| {
                sidebar.set(sidebar.get() + 1);
                vec![
                    SidebarSectionRows::new("files", "Files", Vec::new()).with_files(
                        SidebarFileSet::new(vec![PathBuf::from("example.rs")], |_, _| {}),
                    ),
                ]
            }),
        );
        let history = self.history.clone();
        r.history_annotator(
            "marks",
            HistoryAnnotator::new(self.signal.clone(), move |_, _, _| {
                history.set(history.get() + 1);
                HistoryRowAnnotation::default().with_opacity(0.5)
            }),
        );
    }
}

#[gpui::test]
fn descriptor_revisions_and_slot_notifications_are_independent(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let sidebar_calls = Rc::new(Cell::new(0));
    let history_calls = Rc::new(Cell::new(0));
    let signal = SlotSignal::default();
    cx.update(|cx| {
        super::super::extension_host::install(
            Registry::build(vec![Box::new(Providers {
                sidebar: sidebar_calls.clone(),
                history: history_calls.clone(),
                signal: signal.clone(),
            })])
            .unwrap(),
            cx,
        )
    });
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|w, cx| GitCometView::new(store, events, None, w, cx));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/extension-signals".into(),
        },
    );
    repo.open = Loadable::Ready(());
    let commit = gitcomet_core::domain::Commit {
        id: CommitId("1111222233334444555566667777888899990000".into()),
        parent_ids: Default::default(),
        summary: "Example".into(),
        author: "Example".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    };
    let state = Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo.clone()],
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    });
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            test_support::push_test_state(view, state, cx)
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    let (host, status, sidebar, history) = cx.update(|_, cx| {
        let view = view.read(cx);
        (
            view.extension_window.as_ref().unwrap().host(),
            view.bottom_status_bar.clone(),
            view.sidebar_pane.clone(),
            view.main_pane.read(cx).history_view.clone(),
        )
    });
    let status_notifications = Rc::new(Cell::new(0));
    let sidebar_notifications = Rc::new(Cell::new(0));
    let history_notifications = Rc::new(Cell::new(0));
    let _subscriptions = cx.update(|_, cx| {
        let status_count = status_notifications.clone();
        let sidebar_count = sidebar_notifications.clone();
        let history_count = history_notifications.clone();
        vec![
            cx.observe(&status, move |_, _| {
                status_count.set(status_count.get() + 1)
            }),
            cx.observe(&sidebar, move |_, _| {
                sidebar_count.set(sidebar_count.get() + 1)
            }),
            cx.observe(&history, move |_, _| {
                history_count.set(history_count.get() + 1)
            }),
        ]
    });
    // The Send notifier coalesces a worker's burst and wakes only this slot.
    let notifier = host.notifier();
    cx.background_executor
        .spawn(async move {
            for _ in 0..100 {
                notifier.notify(Slot::Status);
            }
        })
        .detach();
    cx.run_until_parked();
    assert_eq!(status_notifications.get(), 1);
    assert_eq!(sidebar_notifications.get(), 0);
    assert_eq!(history_notifications.get(), 0);
    // History data is cached per commit and provider revision.
    let annotate = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, cx| history.read(cx).history_row_annotation(&repo, &commit, cx))
    };
    assert_eq!(annotate(cx).opacity, 0.5);
    assert_eq!(annotate(cx).opacity, 0.5);
    assert_eq!(history_calls.get(), 1);
    let sidebar_before = sidebar_calls.get();
    assert!(sidebar_before > 0);
    for _ in 0..3 {
        test_support::redraw(cx);
    }
    assert_eq!(sidebar_calls.get(), sidebar_before);
    signal.bump();
    host.notifier().notify(Slot::History);
    host.notifier().notify(Slot::Sidebar);
    cx.run_until_parked();
    test_support::redraw(cx);
    assert_eq!(annotate(cx).opacity, 0.5);
    assert_eq!(history_calls.get(), 2);
    assert_eq!(sidebar_calls.get(), sidebar_before + 1);
}
