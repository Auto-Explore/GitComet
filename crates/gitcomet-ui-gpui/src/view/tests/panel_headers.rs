//! Both side panels switch tabs with one header, and neither pane is
//! narrower than its header.
use super::*;
use crate::view::extension_host;
use gitcomet_extension_api::*;

const TABS: usize = 8;

/// Many details tabs with long titles and no icons of their own.
struct Crowded;

impl Extension for Crowded {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.crowded").unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        for index in 0..TABS {
            registrar.details_tab(
                format!("tab-{index}"),
                DetailsTabDescriptor::new(
                    format!("A contributed details panel with a long title {index}"),
                    |_, _, cx| cx.new(|_| Blank).into(),
                ),
            );
        }
    }
}

struct Blank;

impl gpui::Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn open_window(
    cx: &mut gpui::TestAppContext,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let registry = Registry::build(vec![Box::new(Crowded)]).unwrap();
    cx.update(|app| extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(
                view,
                Arc::new(AppState {
                    repos: vec![RepoState::new_opening(
                        RepoId(1),
                        RepoSpec {
                            workdir: PathBuf::from("/tmp/panel-headers"),
                        },
                    )],
                    active_repo: Some(RepoId(1)),
                    git_runtime: available_git_runtime_state(),
                    ..AppState::test_default()
                }),
                cx,
            );
        });
    });
    cx.simulate_resize(gpui::size(px(2400.0), px(900.0)));
    test_support::redraw(cx);
    (view, cx)
}

/// The details strip is the sidebar's header: the same control, as tall.
#[gpui::test]
fn both_panels_switch_tabs_with_the_same_header(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_window(cx);
    let bounds = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
    };
    assert_eq!(
        bounds(cx, "details_tab_strip").size.height,
        bounds(cx, "sidebar_header").size.height
    );
    let track = bounds(cx, "details_tab_control");
    assert_eq!(
        track.size.height,
        bounds(cx, "sidebar_view_toggle").size.height
    );
    assert!(track.contains(&bounds(cx, "details_tab_details").center()));
    // A tab without an icon of its own still shows one, so it can fold.
    assert!(cx.debug_bounds("details_tab_0_icon").is_some());
}

/// With many tabs and a long selected title, the details pane keeps the
/// width its header needs: the last tab is drawn whole inside the strip.
#[gpui::test]
fn the_details_pane_is_never_narrower_than_its_header(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_window(cx);
    click_debug_selector(cx, "details_tab_7");
    cx.run_until_parked();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_details_width_from_pixels(px(40.0));
            cx.notify();
        })
    });
    for _ in 0..3 {
        test_support::redraw(cx);
    }
    let (minimum, width) = cx.update(|_, app| {
        let view = view.read(app);
        (view.details_min_width(), view.details_width)
    });
    assert!(
        minimum > px(280.0),
        "the header sets the minimum: {minimum:?}"
    );
    assert!(width >= minimum, "{width:?} below {minimum:?}");
    let strip = cx.debug_bounds("details_tab_strip").unwrap();
    let last = cx.debug_bounds("details_tab_7").unwrap();
    assert!(last.right() <= strip.right(), "{last:?} cut by {strip:?}");
    assert!(
        cx.debug_bounds("details_tab_7_label").is_some(),
        "the selected title shows"
    );
}

/// With the sidebar collapsed, a wide details header still sets how narrow
/// the window may get, so the main area keeps its own minimum.
#[gpui::test]
fn a_wide_details_header_widens_the_window_with_the_sidebar_collapsed(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_window(cx);
    click_debug_selector(cx, "details_tab_7");
    cx.run_until_parked();
    cx.update(|_, app| view.update(app, |view, cx| view.set_sidebar_collapsed(true, cx)));
    cx.run_until_parked();
    cx.simulate_resize(gpui::size(px(500.0), px(450.0)));
    for _ in 0..3 {
        test_support::redraw(cx);
    }
    cx.update(|window, app| {
        let view = view.read(app);
        let needed = view.pane_collapsed_width()
            + view.main_min_width()
            + view.details_min_width()
            + view.pane_resize_handle_width();
        let width = window.bounds().size.width;
        assert!(width >= needed, "{width:?} narrower than {needed:?}");
    });
}
