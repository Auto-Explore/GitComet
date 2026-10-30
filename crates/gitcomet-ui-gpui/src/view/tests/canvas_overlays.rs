//! Extension canvas overlays: an extension's view over a repository's main
//! canvas, with the panes around it kept and the view behind it untouched.

use super::*;
use crate::view::extension_host;
use crate::view::extension_panels::repo_key;
use gitcomet_extension_api::{
    CanvasOverlayDescriptor, CanvasOverlayVisibility, ContributionId, DiffPaneOptions,
    DiffSnapshot, Extension, ExtensionId, HostError, Registrar, Registry, RepositoryHandle,
    ShellEvent, ViewTarget, WindowExtension, WindowHost,
};
use gitcomet_extension_example::review::{self, ReviewExtension};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const TOGGLE_OVERLAY: &str = "extension:com.example.review/toggle-review-overlay";

const PROBE_ID: &str = "com.example.probe";
/// The probe's plain overlay, registered after the example's.
const PLAIN: usize = 1;
/// The probe's overlay hosting a diff pane.
const DIFF: usize = 2;

type OverlayLog = Rc<RefCell<Vec<(&'static str, CanvasOverlayVisibility)>>>;

/// Counts the `ViewChanged` and `MainViewChanged` events its windows
/// receive, and registers two overlays of its own that log their visibility
/// in one shared order.
struct Probe {
    view_changes: Rc<Cell<usize>>,
    overlays: OverlayLog,
}

struct Plain;

impl Render for Plain {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "probe_plain_overlay".to_string())
            .size_full()
    }
}

fn logged(log: &OverlayLog, name: &'static str) -> CanvasOverlayDescriptor {
    let log = Rc::clone(log);
    let descriptor = match name {
        "plain" => CanvasOverlayDescriptor::new(|_, _, cx| cx.new(|_| Plain).into()),
        _ => CanvasOverlayDescriptor::new(|context, _, cx| {
            context
                .window
                .create_snapshot_pane(
                    DiffSnapshot::new("a.rs", "old\n", "new\n"),
                    DiffPaneOptions::default(),
                    cx,
                )
                .expect("a main window makes diff panes")
                .view()
        }),
    };
    descriptor.on_visibility(move |_, visibility, _| log.borrow_mut().push((name, visibility)))
}

impl Extension for Probe {
    fn id(&self) -> ExtensionId {
        ExtensionId::new(PROBE_ID).unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        registrar
            .canvas_overlay("plain", logged(&self.overlays, "plain"))
            .canvas_overlay("diff", logged(&self.overlays, "diff"));
    }

    fn window_opened(
        &self,
        _host: WindowHost,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Box<dyn WindowExtension>> {
        Some(Box::new(ViewChangeCount(Rc::clone(&self.view_changes))))
    }
}

struct ViewChangeCount(Rc<Cell<usize>>);

impl WindowExtension for ViewChangeCount {
    fn on_event(&mut self, event: &ShellEvent, _host: &WindowHost, _cx: &mut App) {
        if matches!(event, ShellEvent::ViewChanged | ShellEvent::MainViewChanged) {
            self.0.set(self.0.get() + 1);
        }
    }
}

fn repo(repo_id: RepoId, workdir: &str) -> RepoState {
    RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from(workdir),
        },
    )
}

fn state(repos: Vec<RepoState>, active: RepoId) -> Arc<AppState> {
    Arc::new(AppState {
        active_repo: Some(active),
        repos,
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    })
}

struct Opened<'a> {
    view: gpui::Entity<GitCometView>,
    cx: &'a mut gpui::VisualTestContext,
    host: WindowHost,
    repository: RepositoryHandle,
    view_changes: Rc<Cell<usize>>,
    probe_overlays: OverlayLog,
}

/// A main window over `initial` with the example and an event counter.
fn open(cx: &mut gpui::TestAppContext, initial: Arc<AppState>) -> Opened<'_> {
    let view_changes = Rc::new(Cell::new(0));
    let probe_overlays = OverlayLog::default();
    let registry = Registry::build(vec![
        Box::new(ReviewExtension),
        Box::new(Probe {
            view_changes: Rc::clone(&view_changes),
            overlays: Rc::clone(&probe_overlays),
        }),
    ])
    .expect("valid registration");
    cx.update(|app| extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, initial, cx)
        });
        extension_host::window_opened(&view, app);
        window.activate();
    });
    settle(cx);
    let host = cx.update(|_, app| view.read(app).extension_window.as_ref().unwrap().host());
    let repository = cx.update(|_, app| host.active_repository(app).unwrap().unwrap());
    Opened {
        view,
        cx,
        host,
        repository,
        view_changes,
        probe_overlays,
    }
}

fn overlay() -> ContributionId {
    review::extension_id()
        .contribution(review::REVIEW_OVERLAY)
        .unwrap()
}

fn probe(local: &'static str) -> ContributionId {
    ExtensionId::new(PROBE_ID)
        .unwrap()
        .contribution(local)
        .unwrap()
}

fn settle(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    test_support::redraw(cx);
}

impl Opened<'_> {
    fn is_shown(&mut self) -> bool {
        let (host, repository) = (&self.host, &self.repository);
        self.cx
            .update(|_, app| host.is_canvas_overlay_shown(repository, &overlay(), app))
    }

    fn show(&mut self) {
        let (host, repository) = (&self.host, &self.repository);
        self.cx.update(|_, app| {
            host.show_canvas_overlay(repository, &overlay(), app)
                .unwrap()
        });
        settle(self.cx);
    }

    fn show_probe(&mut self, local: &'static str) {
        let (host, repository) = (&self.host, &self.repository);
        self.cx.update(|_, app| {
            host.show_canvas_overlay(repository, &probe(local), app)
                .unwrap()
        });
        settle(self.cx);
    }

    fn overlay_entity(&mut self, index: usize) -> Option<gpui::EntityId> {
        let view = &self.view;
        self.cx.update(|_, app| {
            view.read(app)
                .canvas_overlay_view(index)
                .map(|view| view.entity_id())
        })
    }

    fn hide(&mut self) {
        let (host, repository) = (&self.host, &self.repository);
        self.cx.update(|_, app| {
            host.hide_canvas_overlay(repository, &overlay(), app)
                .unwrap()
        });
        settle(self.cx);
    }

    fn toggle_through_the_command(&mut self) {
        let view = &self.view;
        self.cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.execute_command(TOGGLE_OVERLAY, Some(window), cx)
            })
        });
    }

    /// Every visibility change the example's observer heard, oldest first.
    fn changes(&mut self) -> Vec<CanvasOverlayVisibility> {
        self.cx.update(|_, app| {
            review::reviews(app)
                .read(app)
                .overlay_changes()
                .iter()
                .map(|(_, visibility)| *visibility)
                .collect()
        })
    }

    fn drawn(&mut self, selector: &'static str) -> bool {
        self.cx.debug_bounds(selector).is_some()
    }

    /// The overlay's built view over the active repository.
    fn overlay_view(&mut self) -> Option<gpui::EntityId> {
        let view = &self.view;
        self.cx.update(|_, app| {
            let root = view.read(app);
            let key = repo_key(root.active_repo()?);
            let overlays = root.extension_window.as_ref()?.canvas_overlays();
            let view = overlays.borrow().view(key, 0);
            view.map(|view| view.entity_id())
        })
    }

    fn push(&mut self, next: Arc<AppState>) {
        let view = &self.view;
        self.cx.update(|_, app| {
            view.update(app, |this, cx| {
                test_support::push_test_state(this, next, cx)
            })
        });
        settle(self.cx);
    }
}

#[gpui::test]
fn canvas_overlays_cover_history_and_return_to_it_untouched(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-history")], RepoId(1)),
    );
    assert!(opened.drawn("history_mode_header"));
    let main_pane = opened
        .cx
        .update(|_, app| opened.view.read(app).main_pane.entity_id());

    opened.toggle_through_the_command();
    assert!(opened.is_shown(), "the show is recorded at once");
    settle(opened.cx);
    assert!(opened.drawn("example_review_overlay"));
    assert!(!opened.drawn("history_mode_header"), "History is covered");
    for kept in [
        "repository_view_tabs",
        "sidebar_pane",
        "details_pane",
        "example_review_status",
    ] {
        assert!(opened.drawn(kept), "{kept} stays around the overlay");
    }
    assert_eq!(opened.changes(), [CanvasOverlayVisibility::Shown]);
    let built = opened.overlay_view().expect("the overlay is built");

    opened.toggle_through_the_command();
    assert!(!opened.is_shown(), "the hide is recorded at once");
    settle(opened.cx);
    assert!(!opened.drawn("example_review_overlay"));
    assert!(opened.drawn("history_mode_header"), "History is back");
    assert_eq!(
        opened
            .cx
            .update(|_, app| opened.view.read(app).main_pane.entity_id()),
        main_pane
    );
    assert_eq!(
        opened.changes(),
        [
            CanvasOverlayVisibility::Shown,
            CanvasOverlayVisibility::Hidden
        ]
    );

    opened.show();
    assert_eq!(
        opened.overlay_view(),
        Some(built),
        "a hidden overlay is kept, not rebuilt"
    );
    assert!(opened.drawn("example_review_overlay"));
    assert_eq!(
        opened.view_changes.get(),
        0,
        "an overlay is not a view change"
    );
}

#[gpui::test]
fn canvas_overlays_cover_an_extension_view_without_rebuilding_it(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-view")], RepoId(1)),
    );
    click_debug_selector(opened.cx, "repository_view_0");
    settle(opened.cx);
    assert!(opened.drawn("example_review_view"));
    let review_view = |opened: &mut Opened<'_>| {
        let view = &opened.view;
        opened.cx.update(|_, app| {
            let root = view.read(app);
            let repo = root.active_repo().unwrap();
            let router = root.repository_views.as_ref().unwrap();
            router.active_view(repo).unwrap().entity_id()
        })
    };
    let built = review_view(&mut opened);
    let selected = opened.view_changes.get();
    assert!(selected > 0, "selecting a view is a view change");

    opened.show();
    let view = opened.view.clone();
    assert_eq!(
        opened.cx.update(|_, app| view
            .read(app)
            .repository_view_tabs()
            .expect("the example registers views")
            .selected),
        Some(0),
        "the view behind stays the selected one"
    );
    assert!(!opened.drawn("example_review_view"));
    assert!(opened.drawn("example_review_overlay"));
    opened.hide();
    assert!(opened.drawn("example_review_view"));
    assert!(!opened.drawn("example_review_overlay"));
    assert_eq!(review_view(&mut opened), built, "the view behind is kept");
    assert_eq!(
        opened.view_changes.get(),
        selected,
        "covering and uncovering a view does not change it"
    );
}

#[gpui::test]
fn asking_for_the_main_canvas_hides_the_overlay(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-tabs")], RepoId(1)),
    );
    let selected_tab = |opened: &mut Opened<'_>| {
        let view = &opened.view;
        opened.cx.update(|_, app| {
            view.read(app)
                .repository_view_tabs()
                .expect("the example registers views")
                .selected
        })
    };

    // The tab of the view already behind it uncovers it, and stays selected
    // throughout.
    opened.show();
    assert_eq!(selected_tab(&mut opened), None);
    click_debug_selector(opened.cx, "repository_view_history");
    settle(opened.cx);
    assert!(!opened.is_shown());
    assert!(opened.drawn("history_mode_header"));
    assert_eq!(
        opened.changes().last(),
        Some(&CanvasOverlayVisibility::Hidden)
    );
    assert_eq!(opened.view_changes.get(), 0, "History was already selected");

    // Another tab uncovers it and goes to its view.
    opened.show();
    click_debug_selector(opened.cx, "repository_view_0");
    settle(opened.cx);
    assert!(!opened.is_shown());
    assert!(opened.drawn("example_review_view"));
    assert_eq!(selected_tab(&mut opened), Some(0));

    // As does navigation on the extension's behalf.
    opened.show();
    let (host, repository) = (opened.host.clone(), opened.repository.clone());
    opened.cx.update(|_, app| {
        host.navigate(&repository, ViewTarget::History, app)
            .unwrap()
    });
    settle(opened.cx);
    assert!(!opened.is_shown());
    assert!(opened.drawn("history_mode_header"));

    // Forward waits while it is shown; Back only uncovers what is behind.
    opened.show();
    let view = opened.view.clone();
    opened
        .cx
        .update(|_, app| view.update(app, |this, cx| this.dispatch_global_nav(true, cx)));
    settle(opened.cx);
    assert!(opened.is_shown(), "Forward does nothing over an overlay");
    let changes = opened.view_changes.get();
    opened
        .cx
        .update(|_, app| view.update(app, |this, cx| this.dispatch_global_nav(false, cx)));
    settle(opened.cx);
    assert!(!opened.is_shown(), "Back uncovers the view behind");
    assert!(opened.drawn("history_mode_header"));
    assert_eq!(opened.view_changes.get(), changes);

    // The action bar's Back button too.
    opened.show();
    click_debug_selector(opened.cx, "global_nav_back");
    settle(opened.cx);
    assert!(!opened.is_shown());
    assert!(opened.drawn("history_mode_header"));
}

#[gpui::test]
fn choosing_a_commit_or_file_hides_the_overlay_but_a_reload_does_not(
    cx: &mut gpui::TestAppContext,
) {
    let mut repo = repo(RepoId(1), "/tmp/overlay-navigation");
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("a.txt"),
        DiffArea::Unstaged,
    ));
    let mut current = (*state(vec![repo], RepoId(1))).clone();
    let mut opened = open(cx, Arc::new(current.clone()));
    opened.show();

    // Staging and the reload after it change the status and reload the open
    // diff, but leave the user where they were.
    let repo = &mut current.repos[0];
    repo.status = Loadable::Ready(Arc::new(RepoStatus {
        staged: Arc::new(vec![FileStatus {
            path: PathBuf::from("a.txt"),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
        unstaged: Arc::default(),
    }));
    repo.status_rev += 1;
    repo.diff_state.diff_state_rev += 1;
    opened.push(Arc::new(current.clone()));
    assert!(opened.is_shown(), "a reload is not a navigation");
    assert!(opened.drawn("example_review_overlay"));

    let repo = &mut current.repos[0];
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("b.txt"),
        DiffArea::Unstaged,
    ));
    repo.diff_state.diff_target_rev += 1;
    opened.push(Arc::new(current.clone()));
    assert!(!opened.is_shown(), "choosing a file asks for the canvas");
    assert!(!opened.drawn("example_review_overlay"));

    opened.show();
    let repo = &mut current.repos[0];
    repo.history_state.selected_commit = Some(CommitId("c0ffee".into()));
    repo.history_state.selected_commit_rev += 1;
    opened.push(Arc::new(current.clone()));
    assert!(!opened.is_shown(), "choosing a commit asks for the canvas");
}

#[gpui::test]
fn canvas_overlays_belong_to_their_repository(cx: &mut gpui::TestAppContext) {
    // One pair throughout: each repository keeps its lifetime.
    let pair = vec![
        repo(RepoId(1), "/tmp/overlay-first"),
        repo(RepoId(2), "/tmp/overlay-second"),
    ];
    let repos = || pair.clone();
    let mut opened = open(cx, state(repos(), RepoId(1)));
    opened.show();
    let built = opened.overlay_view().unwrap();

    opened.push(state(repos(), RepoId(2)));
    assert!(!opened.drawn("example_review_overlay"));
    assert!(opened.drawn("history_mode_header"));
    assert!(opened.is_shown(), "the first repository stays covered");

    opened.push(state(repos(), RepoId(1)));
    assert!(opened.drawn("example_review_overlay"));
    assert_eq!(opened.overlay_view(), Some(built));
}

#[gpui::test]
fn closing_the_repository_drops_its_overlay(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-close")], RepoId(1)),
    );
    opened.show();
    let weak = {
        let view = &opened.view;
        opened
            .cx
            .update(|_, app| view.read(app).canvas_overlay_view(0).unwrap().downgrade())
    };
    let key = opened
        .cx
        .update(|_, app| repo_key(opened.view.read(app).active_repo().unwrap()));

    opened.push(Arc::new(AppState {
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    }));
    assert!(!opened.is_shown());
    assert_eq!(
        opened.changes(),
        [
            CanvasOverlayVisibility::Shown,
            CanvasOverlayVisibility::Dropped
        ]
    );
    let view = &opened.view;
    opened.cx.update(|_, app| {
        let overlays = view
            .read(app)
            .extension_window
            .as_ref()
            .unwrap()
            .canvas_overlays();
        assert!(overlays.borrow().view(key, 0).is_none());
    });
    assert!(weak.upgrade().is_none(), "the overlay's view is dropped");
}

#[gpui::test]
fn dropping_an_overlay_builds_it_again_on_the_next_show(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-drop")], RepoId(1)),
    );
    opened.show();
    let built = opened.overlay_view().unwrap();
    let (host, repository) = (opened.host.clone(), opened.repository.clone());
    opened.cx.update(|_, app| {
        host.drop_canvas_overlay(&repository, &overlay(), app)
            .unwrap();
        assert!(!host.is_canvas_overlay_shown(&repository, &overlay(), app));
        assert_eq!(
            host.drop_canvas_overlay(&repository, &overlay(), app),
            Ok(()),
            "dropping what is not built is fine"
        );
    });
    settle(opened.cx);
    assert!(opened.drawn("history_mode_header"));
    assert_eq!(
        opened.changes(),
        [
            CanvasOverlayVisibility::Shown,
            CanvasOverlayVisibility::Dropped
        ]
    );
    opened.show();
    assert_ne!(opened.overlay_view(), Some(built));
    assert!(opened.drawn("example_review_overlay"));
}

#[gpui::test]
fn unknown_overlays_and_windows_without_a_canvas_are_unsupported(cx: &mut gpui::TestAppContext) {
    let opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-unknown")], RepoId(1)),
    );
    let unknown = review::extension_id().contribution("nope").unwrap();
    let (host, repository) = (opened.host.clone(), opened.repository.clone());
    opened.cx.update(|_, app| {
        assert_eq!(
            host.show_canvas_overlay(&repository, &unknown, app),
            Err(HostError::Unsupported)
        );
        assert_eq!(
            host.hide_canvas_overlay(&repository, &unknown, app),
            Err(HostError::Unsupported)
        );
        assert!(!host.is_canvas_overlay_shown(&repository, &unknown, app));
    });

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let config = GitCometViewConfig {
        view_mode: GitCometViewMode::FocusedDiff,
        focused_diff: Some(crate::FocusedDiffConfig {
            label_left: "before".into(),
            label_right: "after".into(),
            display_path: Some("example.rs".into()),
            input: crate::FocusedDiffInput::Patch(
                "diff --git a/example.rs b/example.rs\n--- a/example.rs\n+++ b/example.rs\n@@ -1 +1 @@\n-old\n+new\n".into(),
            ),
        }),
        workspace: WorkspaceBootstrap::Empty,
        ..Default::default()
    };
    let (focused, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(store, events, config, window, cx)
    });
    cx.update(|_, app| {
        focused.update(app, |this, cx| {
            test_support::push_test_state(
                this,
                state(vec![repo(RepoId(1), "/tmp/overlay-focused")], RepoId(1)),
                cx,
            )
        })
    });
    cx.run_until_parked();
    let host = cx.update(|_, app| focused.read(app).extension_window.as_ref().unwrap().host());
    cx.update(|_, app| {
        let repository = host.active_repository(app).unwrap().unwrap();
        assert_eq!(
            host.show_canvas_overlay(&repository, &overlay(), app),
            Err(HostError::Unsupported),
            "a focused diff has no repository canvas"
        );
    });
}

#[gpui::test]
fn a_hidden_overlay_returns_focus_only_if_it_held_it(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-focus")], RepoId(1)),
    );
    let view = opened.view.clone();
    let (overlay_focus, history_focus) = opened.cx.update(|_, app| {
        let root = view.read(app);
        (
            root.canvas_overlay_focus.clone(),
            root.diff_renderer.read(app).diff_panel_focus_handle.clone(),
        )
    });

    opened.show();
    assert!(
        opened
            .cx
            .update(|window, app| overlay_focus.contains_focused(window, app)),
        "a shown overlay takes the keyboard"
    );
    opened.hide();
    assert!(
        opened
            .cx
            .update(|window, _| history_focus.is_focused(window)),
        "hiding it gives the keyboard back to the view behind"
    );

    opened.show();
    let elsewhere = opened.cx.update(|window, app| {
        let elsewhere = app.focus_handle();
        window.focus(&elsewhere, app);
        elsewhere
    });
    opened.hide();
    assert!(
        opened.cx.update(|window, _| elsewhere.is_focused(window)),
        "a hide never takes focus it did not hold"
    );
}

#[gpui::test]
fn diff_keys_leave_historys_diff_alone_under_an_overlay(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-keys")], RepoId(1)),
    );
    let view = opened.view.clone();
    let target = |opened: &mut Opened<'_>| {
        opened.cx.update(|window, app| {
            view.read(app)
                .diff_shortcut_target(window, app)
                .map(|renderer| renderer.entity_id())
        })
    };
    let history = opened
        .cx
        .update(|_, app| view.read(app).diff_renderer.entity_id());
    assert_eq!(target(&mut opened), Some(history));
    opened.show();
    assert_eq!(
        target(&mut opened),
        None,
        "with no diff pane in the overlay, the keys do nothing"
    );
    opened.hide();
    assert_eq!(target(&mut opened), Some(history));

    opened.show_probe("diff");
    let hosted = target(&mut opened);
    assert!(
        hosted.is_some_and(|hosted| hosted != history),
        "the keys act on the diff pane inside the overlay"
    );
}

#[gpui::test]
fn showing_another_overlay_hides_the_first_and_keeps_it(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-replace")], RepoId(1)),
    );
    opened.show_probe("plain");
    let plain = opened.overlay_entity(PLAIN).expect("built on show");
    assert!(opened.drawn("probe_plain_overlay"));

    opened.show_probe("diff");
    assert!(!opened.drawn("probe_plain_overlay"));
    assert!(opened.overlay_entity(DIFF).is_some());
    let (host, repository) = (opened.host.clone(), opened.repository.clone());
    opened.cx.update(|_, app| {
        assert!(!host.is_canvas_overlay_shown(&repository, &probe("plain"), app));
        assert!(host.is_canvas_overlay_shown(&repository, &probe("diff"), app));
    });
    assert_eq!(
        *opened.probe_overlays.borrow(),
        [
            ("plain", CanvasOverlayVisibility::Shown),
            ("plain", CanvasOverlayVisibility::Hidden),
            ("diff", CanvasOverlayVisibility::Shown),
        ]
    );

    opened.show_probe("plain");
    assert!(opened.drawn("probe_plain_overlay"));
    assert_eq!(
        opened.overlay_entity(PLAIN),
        Some(plain),
        "the replaced overlay was kept"
    );
}

#[gpui::test]
fn a_document_takes_the_slot_from_the_overlay(cx: &mut gpui::TestAppContext) {
    let mut opened = open(
        cx,
        state(vec![repo(RepoId(1), "/tmp/overlay-document")], RepoId(1)),
    );
    opened.show();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.txt");
    std::fs::write(&path, "notes\n").unwrap();
    let view = opened.view.clone();
    opened
        .cx
        .update(|_, app| view.update(app, |this, cx| this.open_document_paths(vec![path], cx)));
    settle(opened.cx);
    assert!(opened.cx.update(|_, app| view.read(app).documents_active));
    assert!(!opened.is_shown());
    assert_eq!(
        opened.changes().last(),
        Some(&CanvasOverlayVisibility::Hidden)
    );

    // Showing the overlay again leaves the document.
    opened.show();
    assert!(!opened.cx.update(|_, app| view.read(app).documents_active));
    assert!(opened.drawn("example_review_overlay"));
}
