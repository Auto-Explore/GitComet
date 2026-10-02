//! A selected extension repository view in the action bar: its own context
//! replaces History's controls, and Back/Forward route to the view.

use super::*;
use crate::view::extension_host;
use gitcomet_extension_api::*;
use gitcomet_extension_example::review::{self, ReviewExtension};
use std::cell::Cell;
use std::rc::Rc;

fn merging_repo(workdir: &Path) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: workdir.to_path_buf(),
        },
    );
    repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'topic'".into()));
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    })
}

fn open_view<'a>(
    cx: &'a mut gpui::TestAppContext,
    registry: Registry,
    workdir: &Path,
) -> (gpui::Entity<GitCometView>, &'a mut gpui::VisualTestContext) {
    cx.update(|app| extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, merging_repo(workdir), cx)
        });
    });
    test_support::redraw(cx);
    (view, cx)
}

#[gpui::test]
fn a_selected_view_brings_its_own_action_bar_context(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let workdir = Path::new("/tmp/extension-action-bar");
    let registry = Registry::build(vec![Box::new(ReviewExtension)]).unwrap();
    let (_view, cx) = open_view(cx, registry, workdir);
    let history_controls = ["worktree_badge", "tracking_actions", "merge_controls"];
    for selector in history_controls {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "History shows {selector}"
        );
    }
    assert!(cx.debug_bounds("extension_action_bar").is_none());

    // The example's Changes view (the second) has a context.
    click_debug_selector(cx, "repository_view_1");
    test_support::redraw(cx);
    for selector in history_controls {
        assert!(
            cx.debug_bounds(selector).is_none(),
            "{selector} stays with History"
        );
    }
    for selector in [
        "global_nav",
        "extension_action_bar",
        "example_changes_actions",
        "right_action_group",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector}");
    }
    click_debug_selector(cx, "example_changes_mark_reviewed");
    cx.run_until_parked();
    let window_id = cx.update(|window, _| window.window_handle().window_id());
    cx.update(|_, app| {
        assert_eq!(
            review::reviews(app).read(app).count(window_id, workdir),
            1,
            "the context acts on its repository"
        );
    });

    // The Review view has none: Back/Forward alone.
    click_debug_selector(cx, "repository_view_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("extension_action_bar").is_none());
    assert!(cx.debug_bounds("tracking_actions").is_none());
    assert!(cx.debug_bounds("global_nav").is_some());

    click_debug_selector(cx, "repository_view_history");
    test_support::redraw(cx);
    for selector in history_controls {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "History shows {selector} again"
        );
    }
    assert!(cx.debug_bounds("extension_action_bar").is_none());

    // Kept with its view: back on Changes, the same context returns.
    click_debug_selector(cx, "repository_view_1");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_changes_actions").is_some());
}

struct Navigable {
    back: Rc<Cell<u32>>,
    forward: Rc<Cell<u32>>,
}

impl Extension for Navigable {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.navigable").unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        let back = self.back.clone();
        let forward = self.forward.clone();
        registrar.repository_view(
            "steps",
            RepositoryViewDescriptor::new("Steps", "", |_, _, cx| cx.new(|_| Steps).into())
                .with_navigation(ViewNavigation::new(
                    |_, _| true,
                    |_, _| false,
                    move |_, _| back.set(back.get() + 1),
                    move |_, _| forward.set(forward.get() + 1),
                )),
        );
    }
}

struct Steps;

impl gpui::Render for Steps {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "steps_view".to_string())
            .size_full()
    }
}

/// The selected view's navigation answers the action bar's Back/Forward and
/// the mouse side buttons; History answers them again once it is back.
#[gpui::test]
fn back_and_forward_route_to_the_selected_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let back = Rc::new(Cell::new(0));
    let forward = Rc::new(Cell::new(0));
    let registry = Registry::build(vec![Box::new(Navigable {
        back: back.clone(),
        forward: forward.clone(),
    })])
    .unwrap();
    let (_view, cx) = open_view(cx, registry, Path::new("/tmp/extension-navigation"));
    click_debug_selector(cx, "repository_view_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("steps_view").is_some());

    click_debug_selector(cx, "global_nav_back");
    cx.run_until_parked();
    assert_eq!(back.get(), 1, "the view can go back");
    // Its forward is unavailable, so the disabled button does nothing.
    click_debug_selector(cx, "global_nav_forward");
    cx.run_until_parked();
    assert_eq!(forward.get(), 0);

    let inside = cx.debug_bounds("steps_view").unwrap().center();
    let side_button = |cx: &mut gpui::VisualTestContext, direction| {
        cx.simulate_mouse_down(
            inside,
            gpui::MouseButton::Navigate(direction),
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            inside,
            gpui::MouseButton::Navigate(direction),
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
    };
    side_button(cx, gpui::NavigationDirection::Back);
    assert_eq!(back.get(), 2, "the side button routes to the view");
    side_button(cx, gpui::NavigationDirection::Forward);
    assert_eq!(forward.get(), 0, "an unavailable step is not taken");

    click_debug_selector(cx, "repository_view_history");
    test_support::redraw(cx);
    side_button(cx, gpui::NavigationDirection::Back);
    assert_eq!(back.get(), 2, "History navigates for itself");
}
