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
    let files = Arc::new(
        paths
            .into_iter()
            .map(|path| CommitFileChange::new(PathBuf::from(path), FileStatusKind::Modified))
            .collect::<Vec<_>>(),
    );
    let entity = shell_cx.update(|_, app| {
        app.new(|cx| FileListView::benchmark_snapshot(host, repository, files, cx))
    });
    let view: gpui::AnyView = entity.clone().into();
    let (_holder, cx) = cx.add_window_view(move |_, _| ListHolder(view));
    let list = HostedFileList {
        entity: entity.clone(),
    };
    draw(cx);
    (list, entity, cx)
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
