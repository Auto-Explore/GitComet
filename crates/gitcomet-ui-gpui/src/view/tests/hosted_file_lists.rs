//! Hosted file lists over in-memory files: caller-defined groups, mark
//! glyphs and visible-path chips.

use super::*;
use crate::view::hosted::file_list::{FileListView, HostedFileList};
use gitcomet_core::domain::{CommitFileChange, FileStatusKind};
use gitcomet_extension_api::panes::FileListImpl;
use gitcomet_extension_api::{
    FileListFilterChip, FileListGroups, FileListMarks, FileListMode, FileListVisible, Registry,
    RepositoryHandle, RowGlyph, RowMark,
};
use std::collections::{BTreeMap, BTreeSet};

struct ListHolder(gpui::AnyView);

impl Render for ListHolder {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

/// A hosted list of `paths`, all modified, in a window of its own.
fn mount_list(
    cx: &mut gpui::TestAppContext,
    paths: impl IntoIterator<Item = String>,
) -> (
    HostedFileList,
    gpui::Entity<FileListView>,
    &mut gpui::VisualTestContext,
) {
    let changes = paths
        .into_iter()
        .map(|path| CommitFileChange::new(PathBuf::from(path), FileStatusKind::Modified))
        .collect();
    let (list, entity, _shell, cx) = mount_changes(cx, changes, None);
    (list, entity, cx)
}

/// A hosted list of `changes` in a window of its own, beside the shell that
/// shows its menus. With `defaults` it starts from them, as the user's;
/// without, in the tree layout and path order.
fn mount_changes(
    cx: &mut gpui::TestAppContext,
    changes: Vec<CommitFileChange>,
    defaults: Option<crate::view::FileListDefaults>,
) -> (
    HostedFileList,
    gpui::Entity<FileListView>,
    gpui::WindowHandle<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|app| {
        let registry = Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .unwrap();
        crate::view::extension_host::install(registry, app);
    });
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (shell, shell_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let host = shell_cx.update(|_, app| shell.read(app).extension_window.as_ref().unwrap().host());
    let workdir = PathBuf::from("/tmp/hosted-file-lists");
    let lifetime = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: workdir.clone(),
        },
    )
    .lifetime();
    let repository = RepositoryHandle::new(host.id(), RepoId(1), lifetime, workdir);
    let files = Arc::new(changes);
    let shell_window = shell_cx
        .window_handle()
        .downcast::<GitCometView>()
        .expect("the shell window");
    let entity = shell_cx.update(|_, app| {
        app.new(|cx| match defaults {
            Some(defaults) => {
                cx.set_global(defaults);
                FileListView::snapshot(host, repository, files, cx)
            }
            None => FileListView::benchmark_snapshot(host, repository, files, cx),
        })
    });
    let view: gpui::AnyView = entity.clone().into();
    let (_holder, cx) = cx.add_window_view(move |_, _| ListHolder(view));
    let list = HostedFileList {
        entity: entity.clone(),
    };
    draw(cx);
    (list, entity, shell_window, cx)
}

fn edited(path: &str, text: &str) -> CommitFileChange {
    let mut edit = gitcomet_core::edit_signature::EditSignatureBuilder::default();
    edit.added(text.as_bytes());
    CommitFileChange::new(PathBuf::from(path), FileStatusKind::Modified)
        .with_line_counts(Some(1), Some(0))
        .with_edit(edit.finish())
}

/// The shell window, drawn, for the menus a hosted list opens there.
fn shell_window(
    cx: &mut gpui::VisualTestContext,
    shell: gpui::WindowHandle<GitCometView>,
) -> gpui::VisualTestContext {
    let mut shell_cx = gpui::VisualTestContext::from_window(shell.into(), cx);
    draw(&mut shell_cx);
    shell_cx
}

fn shell_popover(
    cx: &mut gpui::VisualTestContext,
    shell: gpui::WindowHandle<GitCometView>,
) -> Option<PopoverKind> {
    cx.update(|_, app| {
        shell
            .read(app)
            .expect("the shell window")
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    })
}

/// The user's defaults decide the layout and sort a hosted list opens with;
/// its layout icon then steps through the layouts in its own shape.
#[gpui::test]
fn a_hosted_list_starts_from_the_defaults_and_steps_its_layout(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let defaults = crate::view::FileListDefaults {
        layout: crate::view::FileListLayout::Groups,
        sort: crate::view::rows::CommitFileSort::Edits,
    };
    let changes = vec![
        edited("a.rs", "x"),
        edited("b.rs", "y"),
        edited("c.rs", "x"),
    ];
    let (list, entity, _shell, cx) = mount_changes(cx, changes, Some(defaults));
    let id = list_id(cx, &entity);
    let icon = |key: &str| selector(format!("hosted_file_list_{id}_layout_button_{key}"));

    assert_eq!(shown(cx, &list), vec!["a.rs", "c.rs", "b.rs"], "Edits");
    assert!(cx.debug_bounds(icon("groups")).is_some());
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_group_Modified")))
            .is_some(),
        "grouped by kind"
    );

    click_debug_selector(cx, icon("groups"));
    draw(cx);
    assert!(
        cx.debug_bounds(icon("flat")).is_some(),
        "Groups → Flat list"
    );
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_group_Modified")))
            .is_none()
    );
    click_debug_selector(cx, icon("flat"));
    draw(cx);
    assert!(cx.debug_bounds(icon("tree")).is_some(), "Flat list → Tree");
    assert_eq!(
        shown(cx, &list),
        vec!["a.rs", "c.rs", "b.rs"],
        "the layout leaves the sort alone"
    );
}

/// The sort button and a right click on the layout icon open their menus
/// through the host, and an entry applies to this list.
#[gpui::test]
fn a_hosted_lists_menus_choose_its_sort_and_layout(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let changes = vec![
        edited("a.rs", "x"),
        edited("b.rs", "y"),
        edited("c.rs", "x"),
    ];
    let (list, entity, shell, cx) = mount_changes(cx, changes, None);
    let id = list_id(cx, &entity);
    assert_eq!(shown(cx, &list), vec!["a.rs", "b.rs", "c.rs"]);

    click_debug_selector(cx, selector(format!("hosted_file_list_{id}_sort")));
    cx.run_until_parked();
    assert!(matches!(
        shell_popover(cx, shell),
        Some(PopoverKind::Hosted { menu: true, .. })
    ));
    let mut shell_cx = shell_window(cx, shell);
    assert!(
        shell_cx
            .debug_bounds("context_menu_entry_icon_Path: Ascending")
            .is_some(),
        "the current sort is checked"
    );
    click_debug_selector(&mut shell_cx, "context_menu_edits_repeated_first");
    draw(cx);
    assert_eq!(shown(cx, &list), vec!["a.rs", "c.rs", "b.rs"]);

    let layout = selector(format!("hosted_file_list_{id}_layout_button_tree"));
    let center = cx.debug_bounds(layout).expect("the layout icon").center();
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(layout).is_some(),
        "a right click opens the menu without stepping"
    );
    let mut shell_cx = shell_window(cx, shell);
    click_debug_selector(&mut shell_cx, "context_menu_groups");
    draw(cx);
    assert!(
        cx.debug_bounds(selector(format!(
            "hosted_file_list_{id}_layout_button_groups"
        )))
        .is_some()
    );
}

fn draw(cx: &mut gpui::VisualTestContext) {
    for _ in 0..2 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
}

fn list_id(cx: &mut gpui::VisualTestContext, entity: &gpui::Entity<FileListView>) -> u64 {
    cx.update(|_, app| entity.read(app).test_parts().0)
}

fn selector(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn shown(cx: &mut gpui::VisualTestContext, list: &HostedFileList) -> Vec<String> {
    cx.update(|_, app| {
        list.files(app)
            .into_iter()
            .map(|file| file.path.to_string_lossy().into_owned())
            .collect()
    })
}

/// A glyph takes a column before the file icon in every row once any mark
/// has one; a label alone adds no column.
#[gpui::test]
fn mark_glyphs_draw_in_their_own_column(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (list, entity, cx) = mount_list(cx, ["a.rs", "b.rs"].map(String::from));
    let id = list_id(cx, &entity);
    let glyph = |path: &str| selector(format!("hosted_file_list_{id}_glyph_{path}"));
    let marks = |revision, mark: RowMark| {
        FileListMarks::new(revision, BTreeMap::from([(PathBuf::from("a.rs"), mark)]))
    };
    cx.update(|_, app| list.set_marks(marks(1, RowMark::new(gpui::red()).with_label("note")), app));
    draw(cx);
    assert!(cx.debug_bounds(glyph("a.rs")).is_none());

    let flagged = RowMark::new(gpui::red())
        .with_glyph(RowGlyph::Text("F".into()))
        .with_label("flagged");
    cx.update(|_, app| list.set_marks(marks(2, flagged), app));
    draw(cx);
    let a = cx
        .debug_bounds(glyph("a.rs"))
        .expect("a.rs draws its glyph");
    let row_a = cx
        .debug_bounds(selector(format!("hosted_file_list_{id}_file_a.rs")))
        .unwrap();
    assert!(
        a.origin.x - row_a.origin.x < row_a.size.width / 4.0,
        "the glyph leads the row"
    );
    assert!(
        cx.debug_bounds(glyph("b.rs")).is_none(),
        "an empty slot only"
    );
}

/// A Visible chip shows only its paths and clears on a second click; a
/// replacement of the same label applies while it is active.
#[gpui::test]
fn visible_chips_filter_and_follow_their_replacements(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (list, entity, cx) = mount_list(cx, ["a.rs", "b.rs", "c.rs"].map(String::from));
    let id = list_id(cx, &entity);
    let chip = |revision, paths: &[&str]| {
        FileListFilterChip::visible(
            "Flagged",
            FileListVisible::new(
                revision,
                paths.iter().map(PathBuf::from).collect::<BTreeSet<_>>(),
            ),
        )
    };
    cx.update(|_, app| list.set_filter_chips(vec![chip(1, &["a.rs"])], app));
    draw(cx);
    assert_eq!(shown(cx, &list), vec!["a.rs", "b.rs", "c.rs"]);

    let flagged = selector(format!("file_filter_{id}_0"));
    click_debug_selector(cx, flagged);
    draw(cx);
    assert_eq!(shown(cx, &list), vec!["a.rs"]);

    cx.update(|_, app| list.set_filter_chips(vec![chip(2, &["a.rs", "c.rs"])], app));
    draw(cx);
    assert_eq!(
        shown(cx, &list),
        vec!["a.rs", "c.rs"],
        "the active chip follows"
    );

    click_debug_selector(cx, flagged);
    draw(cx);
    assert_eq!(shown(cx, &list), vec!["a.rs", "b.rs", "c.rs"]);

    // The extension can set the same filter directly.
    cx.update(|_, app| {
        let visible = FileListVisible::new(3, BTreeSet::from([PathBuf::from("b.rs")]));
        list.set_visible(Some(visible), app)
    });
    assert_eq!(shown(cx, &list), vec!["b.rs"]);
}

/// Custom groups over 100k files: scrolling reads the grouped rows it has,
/// and only a new grouping revision regroups.
#[gpui::test]
fn scrolling_a_grouped_100k_list_never_regroups(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let paths = (0..100_000).map(|n| format!("src/group_{}/file_{n}.rs", n / 100));
    let (list, entity, cx) = mount_list(cx, paths);
    let id = list_id(cx, &entity);
    let groups = |revision| {
        FileListGroups::new(
            revision,
            vec!["Even".into(), "Odd".into()],
            |path: &Path| Some(path.as_os_str().len() % 2),
        )
    };
    cx.update(|_, app| {
        list.set_mode(FileListMode::Grouped, app);
        list.set_groups(Some(groups(1)), app);
    });
    draw(cx);
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_group_Even")))
            .is_some()
    );
    let builds = cx.update(|_, app| entity.read(app).test_group_builds());
    let scroll = cx.update(|_, app| entity.read(app).test_parts().1);
    for item in [50_000, 99_000, 10, 70_000] {
        scroll.scroll_to_item(item, gpui::ScrollStrategy::Top);
        draw(cx);
    }
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_sticky_Odd")))
            .is_some(),
        "the Odd group is pinned while its files scroll"
    );
    assert_eq!(
        cx.update(|_, app| entity.read(app).test_group_builds()),
        builds,
        "scrolling never regroups"
    );

    cx.update(|_, app| list.set_groups(Some(groups(2)), app));
    draw(cx);
    let (buckets, _) = cx.update(|_, app| entity.read(app).test_group_builds());
    assert_eq!(buckets, builds.0 + 1, "a new revision regroups once");
}
