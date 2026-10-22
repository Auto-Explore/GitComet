//! Extension controls in the commit box, and messages they put there.
use super::*;
use crate::view::extension_host;
use gitcomet_extension_api::*;
use std::cell::Cell;
use std::rc::Rc;

struct Marker;

impl Render for Marker {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .id("commit_box_marker")
            .debug_selector(|| "commit_box_marker".to_string())
            .size(px(16.0))
    }
}

struct Writer {
    builds: Rc<Cell<usize>>,
}

impl Extension for Writer {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.commit-box").unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        let builds = self.builds.clone();
        registrar.commit_box_item(
            "marker",
            CommitBoxItemDescriptor::new(move |_, _, cx| {
                builds.set(builds.get() + 1);
                cx.new(|_| Marker).into()
            }),
        );
    }
}

fn repo(id: u64, workdir: &str) -> RepoState {
    RepoState::new_opening(
        RepoId(id),
        RepoSpec {
            workdir: PathBuf::from(workdir),
        },
    )
}

fn state(repos: Vec<RepoState>, active: u64) -> Arc<AppState> {
    Arc::new(AppState {
        repos,
        active_repo: Some(RepoId(active)),
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    })
}

fn open_window(
    cx: &mut gpui::TestAppContext,
    builds: Rc<Cell<usize>>,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let registry = Registry::build(vec![Box::new(Writer { builds })]).unwrap();
    cx.update(|app| extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(
                view,
                state(
                    vec![repo(1, "/tmp/commit-box-a"), repo(2, "/tmp/commit-box-b")],
                    1,
                ),
                cx,
            );
        });
        extension_host::window_opened(&view, app);
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    (view, cx)
}

fn host(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> WindowHost {
    cx.update(|_, app| view.read(app).extension_window.as_ref().unwrap().host())
}

fn handle(
    view: &Entity<GitCometView>,
    id: u64,
    cx: &mut gpui::VisualTestContext,
) -> RepositoryHandle {
    cx.update(|window, app| {
        let root = view.read(app);
        let repo = root
            .state
            .repos
            .iter()
            .find(|repo| repo.id == RepoId(id))
            .unwrap();
        extension_host::repository_handle(window.window_handle().window_id(), repo)
    })
}

fn input(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Entity<components::TextInput> {
    cx.update(|_, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .clone()
    })
}

fn text(input: &Entity<components::TextInput>, cx: &mut gpui::VisualTestContext) -> String {
    cx.update(|_, app| input.read(app).text().to_string())
}

#[gpui::test]
fn items_sit_before_previous_messages_and_are_built_once(cx: &mut gpui::TestAppContext) {
    let builds = Rc::new(Cell::new(0));
    let (view, cx) = open_window(cx, builds.clone());
    let marker = cx
        .debug_bounds("commit_box_marker")
        .expect("the item is drawn in the commit box");
    let previous = cx
        .debug_bounds("previous_commit_messages_button")
        .expect("the commit box is drawn");
    assert!(
        marker.right() <= previous.left(),
        "{marker:?} vs {previous:?}"
    );
    assert!(
        previous.top() <= marker.center().y && marker.center().y <= previous.bottom(),
        "{marker:?} vs {previous:?}"
    );

    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(
                view,
                state(
                    vec![repo(1, "/tmp/commit-box-a"), repo(2, "/tmp/commit-box-b")],
                    2,
                ),
                cx,
            );
        });
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("commit_box_marker").is_some());
    assert_eq!(builds.get(), 1, "one item per window, across repositories");
}

#[gpui::test]
fn a_message_replaces_the_box_without_taking_focus(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_window(cx, Rc::default());
    let input = input(&view, cx);
    cx.update(|_, app| input.update(app, |input, cx| input.set_text("my draft", cx)));
    let host = host(&view, cx);
    let active = handle(&view, 1, cx);

    cx.update(|_, app| host.set_commit_message(&active, "Add the retry".into(), app))
        .unwrap();
    cx.run_until_parked();

    assert_eq!(text(&input, cx), "Add the retry");
    cx.update(|window, app| assert!(!input.read(app).focus_handle().is_focused(window)));
}

#[gpui::test]
fn a_repository_off_screen_gets_the_message_as_its_draft(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_window(cx, Rc::default());
    let input = input(&view, cx);
    cx.update(|_, app| input.update(app, |input, cx| input.set_text("my draft", cx)));
    let host = host(&view, cx);
    let other = handle(&view, 2, cx);

    cx.update(|_, app| host.set_commit_message(&other, "Fix the parser".into(), app))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(text(&input, cx), "my draft");

    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(
                view,
                state(
                    vec![repo(1, "/tmp/commit-box-a"), repo(2, "/tmp/commit-box-b")],
                    2,
                ),
                cx,
            );
        });
    });
    cx.run_until_parked();
    assert_eq!(text(&input, cx), "Fix the parser");
}

#[gpui::test]
fn a_repository_closed_before_the_message_lands_keeps_nothing(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_window(cx, Rc::default());
    let host = host(&view, cx);
    let other = handle(&view, 2, cx);

    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(view, state(vec![repo(1, "/tmp/commit-box-a")], 1), cx);
        });
        host.set_commit_message(&other, "Fix the parser".into(), app)
            .expect("the window has not yet seen the close");
    });
    cx.run_until_parked();

    cx.update(|_, app| {
        let pane = view.read(app).details_pane.read(app);
        assert!(pane.commit_message_drafts.is_empty());
    });
    assert_eq!(
        cx.update(|_, app| host.set_commit_message(&other, "Again".into(), app)),
        Err(HostError::RepositoryClosed)
    );
}
