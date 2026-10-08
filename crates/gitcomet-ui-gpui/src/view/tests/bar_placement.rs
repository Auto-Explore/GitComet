//! Exercise placement through the extension API against real chrome and handlers.
use super::*;
use crate::view::extension_host;
use gitcomet_extension_api::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Layout;

impl Extension for Layout {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.layout").unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        for (item, location) in [
            (BuiltinBarItem::Terminal, BarItemLocation::StatusBarEnd),
            (
                BuiltinBarItem::CreateBranch,
                BarItemLocation::StatusBarStart,
            ),
            (BuiltinBarItem::Stash, BarItemLocation::StatusBarEnd),
            (
                BuiltinBarItem::SidebarToggle,
                BarItemLocation::ActionBarStart,
            ),
            (BuiltinBarItem::DetailsToggle, BarItemLocation::ActionBarEnd),
            (BuiltinBarItem::Zoom, BarItemLocation::ActionBarEnd),
        ] {
            registrar.place_bar_item(item, location);
        }
    }
}

fn open_window(
    cx: &mut gpui::TestAppContext,
    registry: Registry,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    cx.update(|app| extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            let repo_id = RepoId(1);
            test_support::push_test_state(
                view,
                Arc::new(AppState {
                    repos: vec![RepoState::new_opening(
                        repo_id,
                        RepoSpec {
                            workdir: PathBuf::from("/tmp/bar-placement"),
                        },
                    )],
                    active_repo: Some(repo_id),
                    git_runtime: available_git_runtime_state(),
                    ..AppState::test_default()
                }),
                cx,
            );
        });
        extension_host::window_opened(&view, app);
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    (view, cx)
}

fn assert_in_bar(cx: &mut gpui::VisualTestContext, selector: &'static str, bar: &'static str) {
    let item = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    let bar = cx
        .debug_bounds(bar)
        .unwrap_or_else(|| panic!("missing {bar}"));
    assert!(
        bar.contains(&item.center()),
        "{selector}: {item:?} vs {bar:?}"
    );
}

#[gpui::test]
fn builtins_move_in_both_directions_and_keep_their_handlers(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = open_window(cx, Registry::build(vec![Box::new(Layout)]).unwrap());
    for selector in ["terminal", "create_branch", "stash"] {
        assert_in_bar(cx, selector, "bottom_status_bar");
    }
    for selector in ["sidebar_toggle", "details_toggle", "bottom_status_bar_zoom"] {
        assert_in_bar(cx, selector, "action_bar");
    }
    let terminal = cx.debug_bounds("terminal").unwrap();
    let details = cx.debug_bounds("details_toggle").unwrap();
    assert!(terminal.center().y > details.center().y);
    // Footer actions shed their labels, even in a wide window.
    assert!(terminal.size.width < gpui::px(50.0));

    let (sidebar_before, details_before) = cx.update(|_, app| {
        let root = view.read(app);
        (root.sidebar_collapsed, root.details_collapsed)
    });
    click_debug_selector(cx, "sidebar_toggle");
    click_debug_selector(cx, "details_toggle");
    cx.update(|_, app| {
        let root = view.read(app);
        assert_eq!(root.sidebar_collapsed, !sidebar_before);
        assert_eq!(root.details_collapsed, !details_before);
    });
    // A moved action still opens the host's existing prompt from its new bounds.
    click_debug_selector(cx, "create_branch");
    cx.update(|_, app| {
        assert!(matches!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::CreateBranchFromRefPrompt {
                repo_id: RepoId(1),
                ..
            })
        ));
    });
}

struct Indicator {
    value: usize,
}

impl Render for Indicator {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let value = self.value;
        div()
            .id("placed_indicator")
            .debug_selector(move || format!("placed_indicator_{value}"))
            .child(value.to_string())
    }
}

struct Contributed {
    builds: Rc<Cell<usize>>,
    indicator: Rc<RefCell<Option<Entity<Indicator>>>>,
}

impl Extension for Contributed {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.bar-items").unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        let builds = self.builds.clone();
        let indicator = self.indicator.clone();
        registrar.place_bar_item(
            BuiltinBarItem::RepositoryViews,
            BarItemLocation::StatusBarStart,
        );
        registrar.status_item(
            "indicator",
            StatusItemDescriptor::new(move |_, _, cx| {
                builds.set(builds.get() + 1);
                let view = cx.new(|_| Indicator { value: 1 });
                *indicator.borrow_mut() = Some(view.clone());
                view.into()
            })
            .with_location(BarItemLocation::ActionBarEnd)
            .with_view(ViewTarget::History),
        );
        registrar.repository_view(
            "other",
            RepositoryViewDescriptor::new("Other", "icons/history.svg", |_, _, cx| {
                cx.new(|_| Indicator { value: 100 }).into()
            }),
        );
    }
}

#[gpui::test]
fn contributed_items_keep_scope_lifetime_and_independent_invalidation(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let builds = Rc::new(Cell::new(0));
    let indicator = Rc::new(RefCell::new(None));
    let (_, cx) = open_window(
        cx,
        Registry::build(vec![Box::new(Contributed {
            builds: builds.clone(),
            indicator: indicator.clone(),
        })])
        .unwrap(),
    );
    assert_in_bar(cx, "placed_indicator_1", "action_bar");
    assert_in_bar(cx, "repository_view_history", "bottom_status_bar");
    cx.update(|_, app| {
        indicator
            .borrow()
            .as_ref()
            .unwrap()
            .update(app, |view, cx| {
                view.value = 2;
                cx.notify();
            });
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("placed_indicator_1").is_none());
    assert_in_bar(cx, "placed_indicator_2", "action_bar");
    click_debug_selector(cx, "repository_view_0");
    test_support::redraw(cx);
    assert!(cx.debug_bounds("placed_indicator_2").is_none());
    click_debug_selector(cx, "repository_view_history");
    test_support::redraw(cx);
    assert_in_bar(cx, "placed_indicator_2", "action_bar");
    assert_eq!(builds.get(), 1, "bar items are kept across view switches");
}
