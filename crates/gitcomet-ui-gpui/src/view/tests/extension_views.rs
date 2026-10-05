//! Extension repository views in the action bar: tabs for History and each
//! view, a More tab for the views listed under it, a selected view's own
//! context in place of History's controls, and Back/Forward routed to it.

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

/// The repository's views are tabs at the head of the action bar's right
/// group, as tall as the bar: History and each view, the selected one
/// underlined. No strip sits above the main area any more.
#[gpui::test]
fn the_views_are_tabs_in_the_action_bar(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let registry = Registry::build(vec![Box::new(ReviewExtension)]).unwrap();
    let (_view, cx) = open_view(cx, registry, Path::new("/tmp/extension-view-tabs"));
    cx.simulate_resize(gpui::size(px(1600.0), px(900.0)));
    test_support::redraw(cx);
    assert!(cx.debug_bounds("repository_view_strip").is_none());
    let bar = cx.debug_bounds("action_bar").expect("the action bar");
    let right = cx
        .debug_bounds("right_action_group")
        .expect("its right group");
    let tabs = cx
        .debug_bounds("repository_view_tabs")
        .expect("the view tabs");
    assert!(
        tabs.left() >= right.left() && tabs.right() <= right.right(),
        "the tabs belong to the right group: {tabs:?} in {right:?}"
    );
    let terminal = cx.debug_bounds("terminal").expect("Terminal");
    assert!(tabs.right() <= terminal.left(), "the tabs lead it");
    for (selector, label) in [
        ("repository_view_history", "repository_view_history_label"),
        ("repository_view_0", "repository_view_0_label"),
        ("repository_view_1", "repository_view_1_label"),
    ] {
        let tab = cx.debug_bounds(selector).expect(selector);
        assert_eq!(tab.size.height, bar.size.height, "{selector} fills the bar");
        assert!(cx.debug_bounds(label).is_some(), "{label}");
    }
    assert!(
        cx.debug_bounds("repository_view_history_underline")
            .is_some()
    );
    assert!(cx.debug_bounds("repository_view_0_underline").is_none());
    assert!(
        cx.debug_bounds("repository_view_more").is_none(),
        "no view is listed under More"
    );

    click_debug_selector(cx, "repository_view_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_view").is_some());
    assert!(cx.debug_bounds("repository_view_0_underline").is_some());
    assert!(
        cx.debug_bounds("repository_view_history_underline")
            .is_none()
    );
}

/// Narrow, the tabs keep their icons and give up their labels, and the two
/// groups of the bar still do not overlap at the narrowest window.
#[gpui::test]
fn compact_view_tabs_show_icons_alone(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let registry = Registry::build(vec![Box::new(ReviewExtension)]).unwrap();
    let (_view, cx) = open_view(cx, registry, Path::new("/tmp/extension-view-tabs-compact"));
    cx.simulate_resize(gpui::size(px(820.0), px(600.0)));
    test_support::redraw(cx);
    assert!(cx.debug_bounds("repository_view_history_icon").is_some());
    assert!(cx.debug_bounds("repository_view_history_label").is_none());
    assert!(cx.debug_bounds("repository_view_0_label").is_none());
    let left = cx.debug_bounds("left_action_group").expect("left group");
    let right = cx.debug_bounds("right_action_group").expect("right group");
    assert!(
        left.right() <= right.left(),
        "the groups must not overlap: {left:?}, {right:?}"
    );
}

/// Three views, two of them listed under More.
struct Listed;

impl Extension for Listed {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.listed").unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        let view = |selector: &'static str| {
            move |_: RepositoryViewContext, _: &mut Window, cx: &mut App| {
                cx.new(|_| Labelled(selector)).into()
            }
        };
        registrar
            .repository_view(
                "alpha",
                RepositoryViewDescriptor::new("Alpha", "icons/history.svg", view("alpha_view")),
            )
            .repository_view(
                "beta",
                RepositoryViewDescriptor::new("Beta", "icons/history.svg", view("beta_view"))
                    .under_more(),
            )
            .repository_view(
                "gamma",
                RepositoryViewDescriptor::new("Gamma", "icons/terminal.svg", view("gamma_view"))
                    .under_more(),
            );
    }
}

struct Labelled(&'static str);

impl gpui::Render for Labelled {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let selector = self.0;
        div()
            .debug_selector(move || selector.to_string())
            .size_full()
    }
}

/// Views listed under More share one tab, whose menu lists them; while one
/// of them shows, the tab names it and carries the underline.
#[gpui::test]
fn views_under_more_share_one_tab_that_names_the_selected_one(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let registry = Registry::build(vec![Box::new(Listed)]).unwrap();
    let (_view, cx) = open_view(cx, registry, Path::new("/tmp/extension-view-more"));
    cx.simulate_resize(gpui::size(px(1600.0), px(900.0)));
    test_support::redraw(cx);
    assert!(cx.debug_bounds("repository_view_0").is_some());
    assert!(cx.debug_bounds("repository_view_1").is_none());
    assert!(cx.debug_bounds("repository_view_2").is_none());
    assert!(cx.debug_bounds("repository_view_more_label").is_some());
    assert!(cx.debug_bounds("repository_view_more_end_icon").is_some());
    assert!(cx.debug_bounds("repository_view_more_underline").is_none());
    let more = cx.debug_bounds("repository_view_more").expect("More");

    click_debug_selector(cx, "repository_view_more");
    cx.run_until_parked();
    test_support::redraw(cx);
    let beta = cx
        .debug_bounds("context_menu_beta")
        .expect("Beta is listed");
    assert!(cx.debug_bounds("context_menu_gamma").is_some());
    assert!(
        cx.debug_bounds("context_menu_alpha").is_none(),
        "Alpha has a tab"
    );
    assert!(beta.top() >= more.bottom(), "the menu opens below its tab");
    assert!(
        cx.debug_bounds("context_menu_entry_icon_Gamma").is_some(),
        "each entry with its view's icon"
    );

    click_debug_selector(cx, "context_menu_gamma");
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("gamma_view").is_some(), "the view shows");
    assert!(
        cx.debug_bounds("context_menu_beta").is_none(),
        "the menu closed"
    );
    assert!(cx.debug_bounds("repository_view_more_underline").is_some());
    assert!(cx.debug_bounds("repository_view_more_icon").is_some());
    assert!(
        cx.debug_bounds("repository_view_history_underline")
            .is_none()
    );

    // History again: More stops naming the view.
    click_debug_selector(cx, "repository_view_history");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("gamma_view").is_none());
    assert!(cx.debug_bounds("repository_view_more_underline").is_none());
    assert!(cx.debug_bounds("repository_view_more_icon").is_none());
}

struct KeyboardView {
    focus: gpui::FocusHandle,
    input: gpui::Entity<components::TextInput>,
}
impl gpui::Focusable for KeyboardView {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}
impl gpui::Render for KeyboardView {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus)
            .child(self.input.clone())
    }
}
struct KeyboardViews;
impl Extension for KeyboardViews {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.keyboard").unwrap()
    }
    fn register(&self, registrar: &mut Registrar) {
        for (id, more) in [("first", false), ("second", true)] {
            let descriptor =
                RepositoryViewDescriptor::new(id, "icons/history.svg", |_, window, app| {
                    app.new(|cx| KeyboardView {
                        focus: cx.focus_handle(),
                        input: cx
                            .new(|cx| components::TextInput::new(Default::default(), window, cx)),
                    })
                    .into()
                })
                .with_focus::<KeyboardView>();
            registrar.repository_view(
                id,
                if more {
                    descriptor.under_more()
                } else {
                    descriptor
                },
            );
        }
    }
}

#[gpui::test]
fn switching_views_focuses_the_selected_view_once(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let registry = Registry::build(vec![Box::new(KeyboardViews)]).unwrap();
    let (view, cx) = open_view(cx, registry, Path::new("/tmp/extension-keyboard"));
    // Simulate a keyboard command invoked while History's commit box holds
    // focus. That field disappears as the selected view opens.
    cx.update(|window, app| {
        let input = view
            .read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .clone();
        window.focus(&input.read(app).focus_handle(), app);
        view.update(app, |view, cx| {
            view.select_routed_view(
                crate::view::repository_views::RoutedArea::Main,
                Some(0),
                window,
                cx,
            )
        });
        let router = view.read(app).repository_views.as_ref().unwrap();
        let repo = view.read(app).active_repo().unwrap();
        let selected = router
            .built(repo, 0)
            .unwrap()
            .downcast::<KeyboardView>()
            .unwrap();
        assert!(
            selected.read(app).focus.is_focused(window),
            "focus moves before the first frame"
        );
    });
    test_support::redraw(cx);
    let selected = cx.update(|_, app| {
        let root = view.read(app);
        root.repository_views
            .as_ref()
            .unwrap()
            .built(root.active_repo().unwrap(), 0)
            .unwrap()
            .downcast::<KeyboardView>()
            .unwrap()
    });
    let input = selected.read_with(cx, |view, _| view.input.clone());
    cx.update(|window, app| window.focus(&input.read(app).focus_handle(), app));
    test_support::redraw(cx);
    assert!(
        cx.update(|window, app| input.read(app).focus_handle().is_focused(window)),
        "redraw leaves the field alone"
    );
    // A view selected through More must keep focus after its menu closes.
    click_debug_selector(cx, "repository_view_more");
    test_support::redraw(cx);
    click_debug_selector(cx, "context_menu_second");
    test_support::redraw(cx);
    assert!(cx.update(|window, app| {
        let root = view.read(app);
        root.repository_views
            .as_ref()
            .unwrap()
            .built(root.active_repo().unwrap(), 1)
            .unwrap()
            .downcast::<KeyboardView>()
            .unwrap()
            .read(app)
            .focus
            .is_focused(window)
    }));
    click_debug_selector(cx, "repository_view_history");
    test_support::redraw(cx);
    assert!(cx.update(|window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .is_focused(window)
    }));
}
