//! What the canvas says when a diff has no rows to show.

use super::*;

const FILE_TEXT: &str = "fn main() {}\nfn helper() {}\n";
const RENAME_ONLY_PATCH: &str = "\
diff --git a/old_name.rs b/new_name.rs
similarity index 100%
rename from old_name.rs
rename to new_name.rs
";
const RENAME_AND_MODE_PATCH: &str = "\
diff --git a/old_name.rs b/new_name.rs
old mode 100644
new mode 100755
similarity index 100%
rename from old_name.rs
rename to new_name.rs
";
const COPY_ONLY_PATCH: &str = "\
diff --git a/old_name.rs b/new_name.rs
similarity index 100%
copy from old_name.rs
copy to new_name.rs
";
const MODE_ONLY_PATCH: &str = "\
diff --git a/new_name.rs b/new_name.rs
old mode 100644
new mode 100755
";

fn collapsed_commit_file_diff<'a>(
    cx: &'a mut gpui::TestAppContext,
    repo_id: u64,
    fixture_name: &str,
    diff_view: DiffViewMode,
    renamed_from: Option<&str>,
    patch: &str,
    text: &str,
) -> (
    gpui::Entity<super::super::super::GitCometView>,
    &'a mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let target = gitcomet_core::domain::DiffTarget::commit(
        gitcomet_core::domain::CommitId("deadbeef".into()),
        PathBuf::from("new_name.rs"),
    )
    .with_old_path(renamed_from.map(PathBuf::from));
    push_commit_file_diff_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(repo_id),
        fixture_name,
        target.clone(),
        1,
        patch.to_string(),
        text.to_string(),
        text.to_string(),
    );
    cx.update(|_window, app| {
        let renderer = view.read(app).diff_renderer.clone();
        renderer.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);
    wait_for_main_pane_condition(
        cx,
        &view,
        "the collapsed projection of a file diff without changed lines",
        |pane| {
            pane.file_diff_cache_target == Some(target.clone())
                && pane.is_collapsed_diff_projection_active()
        },
        |pane| {
            (
                pane.file_diff_cache_target.clone(),
                pane.is_collapsed_diff_projection_active(),
            )
        },
    );
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(pane.collapsed_diff_visible_rows.is_empty());
    });
    (view, cx)
}

fn zero_row_title_and_message(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> Option<(&'static str, String)> {
    cx.update(|_window, app| {
        view.read(app)
            .diff_renderer
            .read(app)
            .zero_row_notice()
            .map(|notice| (notice.title(), notice.message().to_string()))
    })
}

fn assert_rename_without_changed_lines_names_the_compared_file(
    cx: &mut gpui::TestAppContext,
    repo_id: u64,
    diff_view: DiffViewMode,
) {
    let (view, cx) = collapsed_commit_file_diff(
        cx,
        repo_id,
        &format!("zero_rows_rename_{repo_id}"),
        diff_view,
        Some("old_name.rs"),
        RENAME_ONLY_PATCH,
        FILE_TEXT,
    );
    assert_eq!(
        zero_row_title_and_message(cx, &view),
        Some((
            "Renamed",
            "No changed lines compared with old_name.rs.".to_string()
        ))
    );
    assert!(cx.debug_bounds("diff_no_changed_lines").is_some());
    assert!(cx.debug_bounds("diff_empty_file").is_none());
}

#[gpui::test]
fn collapsed_inline_rename_without_changed_lines_names_the_compared_file(
    cx: &mut gpui::TestAppContext,
) {
    assert_rename_without_changed_lines_names_the_compared_file(cx, 961, DiffViewMode::Inline);
}

#[gpui::test]
fn collapsed_split_rename_without_changed_lines_names_the_compared_file(
    cx: &mut gpui::TestAppContext,
) {
    assert_rename_without_changed_lines_names_the_compared_file(cx, 962, DiffViewMode::Split);
}

#[gpui::test]
fn collapsed_mode_change_without_changed_lines_is_not_an_empty_file(cx: &mut gpui::TestAppContext) {
    let (view, cx) = collapsed_commit_file_diff(
        cx,
        963,
        "zero_rows_mode_change",
        DiffViewMode::Inline,
        None,
        MODE_ONLY_PATCH,
        FILE_TEXT,
    );
    assert_eq!(
        zero_row_title_and_message(cx, &view),
        Some(("Diff", "No changed lines.".to_string()))
    );
    assert!(cx.debug_bounds("diff_no_changed_lines").is_some());
    assert!(cx.debug_bounds("diff_empty_file").is_none());
}

#[gpui::test]
fn collapsed_copy_without_changed_lines_is_not_titled_renamed(cx: &mut gpui::TestAppContext) {
    let (view, cx) = collapsed_commit_file_diff(
        cx,
        968,
        "zero_rows_copy",
        DiffViewMode::Inline,
        Some("old_name.rs"),
        COPY_ONLY_PATCH,
        FILE_TEXT,
    );
    assert_eq!(
        zero_row_title_and_message(cx, &view),
        Some((
            "Diff",
            "No changed lines compared with old_name.rs.".to_string()
        ))
    );
}

#[gpui::test]
fn collapsed_rename_with_a_mode_change_is_titled_renamed(cx: &mut gpui::TestAppContext) {
    let (view, cx) = collapsed_commit_file_diff(
        cx,
        969,
        "zero_rows_rename_and_mode",
        DiffViewMode::Inline,
        Some("old_name.rs"),
        RENAME_AND_MODE_PATCH,
        FILE_TEXT,
    );
    assert_eq!(
        zero_row_title_and_message(cx, &view),
        Some((
            "Renamed",
            "No changed lines compared with old_name.rs.".to_string()
        ))
    );
}

#[gpui::test]
fn full_mode_shows_the_lines_of_a_rename_without_changed_lines(cx: &mut gpui::TestAppContext) {
    let (view, cx) = collapsed_commit_file_diff(
        cx,
        966,
        "zero_rows_full_mode",
        DiffViewMode::Inline,
        Some("old_name.rs"),
        RENAME_ONLY_PATCH,
        FILE_TEXT,
    );
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Full);
    assert_eq!(zero_row_title_and_message(cx, &view), None);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.diff_visible_len(), 2);
    });
    assert!(cx.debug_bounds("diff_no_changed_lines").is_none());
}

#[gpui::test]
fn collapsed_diff_of_a_renamed_file_without_lines_is_an_empty_file(cx: &mut gpui::TestAppContext) {
    let (view, cx) = collapsed_commit_file_diff(
        cx,
        965,
        "zero_rows_empty_file",
        DiffViewMode::Inline,
        Some("old_name.rs"),
        RENAME_ONLY_PATCH,
        "",
    );
    assert_eq!(
        zero_row_title_and_message(cx, &view),
        Some(("Renamed", "Empty file.".to_string()))
    );
    assert!(cx.debug_bounds("diff_empty_file").is_some());
    assert!(cx.debug_bounds("diff_no_changed_lines").is_none());
}

#[gpui::test]
fn collapsed_diff_of_a_file_without_lines_keeps_the_diff_title(cx: &mut gpui::TestAppContext) {
    let (view, cx) = collapsed_commit_file_diff(
        cx,
        970,
        "zero_rows_plain_empty_file",
        DiffViewMode::Inline,
        None,
        MODE_ONLY_PATCH,
        "",
    );
    assert_eq!(
        zero_row_title_and_message(cx, &view),
        Some(("Diff", "Empty file.".to_string()))
    );
}

#[gpui::test]
fn a_comparison_without_changes_says_no_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(967);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_zero_rows_comparison_root",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let target = gitcomet_core::domain::DiffTarget::commit_range(
        gitcomet_core::domain::CommitId("aaaa".into()),
        Some(gitcomet_core::domain::CommitId("bbbb".into())),
        None,
    );
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), "");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "the comparison's empty patch",
        |pane| pane.rendered_patch_diff_cache_is_current(),
        |pane| pane.diff_cache_target.clone(),
    );
    draw_and_drain_test_window(cx);

    assert_eq!(
        zero_row_title_and_message(cx, &view),
        Some(("Diff", "No changes.".to_string()))
    );
}
