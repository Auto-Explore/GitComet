//! A repository view's own details and sidebar tabs: listed only with the
//! view, opened when it opens, giving way when it is left; selected and
//! revealed on request; and markdown rendered for an extension's views.

use super::*;
use crate::view::extension_host;
use gitcomet_extension_api::*;

const EXTENSION: &str = "com.example.panels";

fn id(local: &'static str) -> ContributionId {
    ExtensionId::new(EXTENSION)
        .unwrap()
        .contribution(local)
        .unwrap()
}

/// A view with a details tab and a sidebar tab of its own, and a details
/// tab listed everywhere.
struct Panels;

impl Extension for Panels {
    fn id(&self) -> ExtensionId {
        ExtensionId::new(EXTENSION).unwrap()
    }

    fn register(&self, registrar: &mut Registrar) {
        let view = |selector: &'static str| {
            move |_: RepositoryViewContext, _: &mut Window, cx: &mut App| {
                cx.new(|_| Labelled(selector)).into()
            }
        };
        let review = ViewTarget::Extension(id("review"));
        registrar
            .repository_view(
                "review",
                RepositoryViewDescriptor::new("Review", "icons/history.svg", view("panels_review")),
            )
            .details_tab(
                "notes",
                DetailsTabDescriptor::new("Notes", view("panels_notes"))
                    .with_view(review.clone())
                    .opens_with_view(),
            )
            .details_tab(
                "always",
                DetailsTabDescriptor::new("Always", view("panels_always")),
            )
            .details_tab(
                "doc",
                DetailsTabDescriptor::new("Doc", |context, _, cx| {
                    let markdown = context.window.create_markdown_view(cx).unwrap();
                    markdown.set_source("# Heading\n\nA paragraph with `code`.\n", cx);
                    cx.new(|_| Wrapped(markdown.view())).into()
                }),
            )
            .sidebar_tab(
                "files",
                SidebarTabDescriptor::new("Changed files", view("panels_files"))
                    .with_view(review)
                    .opens_with_view(),
            );
    }
}

/// An extension's own view around a markdown view.
struct Wrapped(gpui::AnyView);

impl gpui::Render for Wrapped {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
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

fn open_view(
    cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let registry = Registry::build(vec![Box::new(Panels)]).unwrap();
    cx.update(|app| extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/extension-view-panels"),
        },
    );
    repo.open = Loadable::Ready(());
    let state = Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            test_support::push_test_state(this, state, cx)
        });
    });
    cx.simulate_resize(gpui::size(px(1600.0), px(900.0)));
    test_support::redraw(cx);
    (view, cx)
}

fn shows(cx: &mut gpui::VisualTestContext, selector: &'static str) -> bool {
    cx.debug_bounds(selector).is_some()
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    click_debug_selector(cx, selector);
    cx.run_until_parked();
    test_support::redraw(cx);
}

#[gpui::test]
fn a_views_own_tabs_open_with_it_and_give_way_when_it_is_left(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_view, cx) = open_view(cx);
    // History lists only the tab that belongs to every view.
    assert!(!shows(cx, "details_tab_0"), "Notes belongs to Review");
    assert!(shows(cx, "details_tab_1"));
    assert!(!shows(cx, "sidebar_tab_extension_0"));
    assert!(shows(cx, "sidebar_tab_branches"));

    click(cx, "repository_view_0");
    assert!(shows(cx, "panels_review"));
    assert!(shows(cx, "details_tab_0"));
    assert!(shows(cx, "panels_notes"), "Notes opens with Review");
    assert!(shows(cx, "sidebar_tab_extension_0"));
    assert!(
        shows(cx, "panels_files"),
        "Changed files opens with Review, after Branches and Files"
    );
    let files = cx.debug_bounds("sidebar_tab_files").unwrap();
    let changed = cx.debug_bounds("sidebar_tab_extension_0").unwrap();
    assert!(files.right() <= changed.left());

    click(cx, "repository_view_history");
    assert!(!shows(cx, "panels_notes"));
    assert!(!shows(cx, "details_tab_0"));
    assert!(!shows(cx, "panels_files"));
    assert!(!shows(cx, "sidebar_tab_extension_0"));
    assert!(!shows(cx, "sidebar_extension_tab_content"));

    // What was selected before the view opened comes back after it.
    click(cx, "details_tab_1");
    assert!(shows(cx, "panels_always"));
    click(cx, "repository_view_0");
    assert!(shows(cx, "panels_notes"));
    click(cx, "repository_view_history");
    assert!(shows(cx, "panels_always"));
}

#[gpui::test]
fn show_selects_a_listed_tab_and_reveals_its_pane(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = open_view(cx);
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let repository = cx.update(|_window, app| host.active_repository(app).unwrap().unwrap());

    // Not listed in History: nothing happens.
    cx.update(|_window, app| host.show_details_tab(&repository, &id("notes"), app))
        .unwrap();
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(!shows(cx, "panels_notes"));

    click(cx, "repository_view_0");
    click(cx, "details_tab_details");
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.set_details_collapsed(true, cx);
            view.set_sidebar_collapsed(true, cx);
        })
    });
    test_support::redraw(cx);
    assert!(!shows(cx, "panels_notes"));
    cx.update(|_window, app| {
        host.show_details_tab(&repository, &id("notes"), app)
            .unwrap();
        host.show_sidebar_tab(&repository, &id("files"), app)
            .unwrap();
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.update(|_window, app| {
        let view = view.read(app);
        !view.details_collapsed && !view.sidebar_collapsed
    }));
    assert!(shows(cx, "panels_notes"));
    assert!(shows(cx, "panels_files"));
    assert_eq!(
        cx.update(|_window, app| host.show_details_tab(&repository, &id("review"), app)),
        Err(HostError::Unsupported),
        "a repository view is not a details tab"
    );
}

#[gpui::test]
fn a_markdown_view_renders_the_source_it_is_given(cx: &mut gpui::TestAppContext) {
    use crate::view::markdown_preview::MarkdownPreviewRowKind as Kind;
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = open_view(cx);
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let markdown = cx.update(|_window, app| host.create_markdown_view(app).unwrap());
    cx.update(|_window, app| markdown.set_source("# Title\n\nSome *text*.\n\n- one\n- two\n", app));
    let entity = markdown
        .view()
        .downcast::<crate::view::hosted::markdown::MarkdownView>()
        .unwrap();
    let kinds = cx.update(|_window, app| entity.read(app).row_kinds());
    assert!(kinds.contains(&Kind::Heading { level: 1 }), "{kinds:?}");
    assert!(kinds.contains(&Kind::Paragraph));
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| matches!(kind, Kind::ListItem { .. }))
            .count(),
        2
    );

    // Drawn inside an extension's view, at its content's height.
    click(cx, "details_tab_2");
    let drawn = cx
        .debug_bounds("hosted_markdown")
        .expect("the markdown view");
    assert!(drawn.size.height > px(20.0), "{drawn:?}");
}
