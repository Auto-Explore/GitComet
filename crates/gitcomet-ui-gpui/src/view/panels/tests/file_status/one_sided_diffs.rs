//! A change with only one side — added, untracked or deleted — shows that side
//! alone, whatever Inline/Split says.

use super::*;
use crate::view::panes::diff::DiffBody;
use gitcomet_core::domain::{
    CommitDetails, CommitFileChange, CommitId, DiffArea, DiffPreviewTextFile, DiffPreviewTextSide,
    DiffTarget, FileDiffImage, FileStatusKind,
};
use gitcomet_state::model::{Loadable, RangeSelection, RepoId, RepoState};

fn open_view(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<super::super::super::GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    })
}

fn test_workdir(name: &str) -> std::path::PathBuf {
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("create test workdir");
    workdir
}

fn seed_comparison(repo: &mut RepoState, path: &std::path::Path, kind: FileStatusKind) {
    seed_comparison_to(repo, path, kind, Some(CommitId("tip".into())));
}

/// A comparison from `base` to `to`; `None` compares against the working tree.
fn seed_comparison_to(
    repo: &mut RepoState,
    path: &std::path::Path,
    kind: FileStatusKind,
    to: Option<CommitId>,
) {
    let from = CommitId("base".into());
    repo.history_state.range_selection = Some(RangeSelection::new(
        from.clone(),
        to.clone(),
        "base".into(),
        "tip".into(),
    ));
    repo.history_state.range_files = Loadable::Ready(Arc::new(vec![CommitFileChange::new(
        path.to_path_buf(),
        kind,
    )]));
    repo.history_state.range_files_rev = repo.history_state.range_files_rev.wrapping_add(1);
    repo.diff_state.diff_target =
        Some(DiffTarget::commit_range(from, to, Some(path.to_path_buf())));
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);
}

fn seed_commit(repo: &mut RepoState, path: &std::path::Path, kind: FileStatusKind) {
    let commit_id = CommitId("deadbeef".into());
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id.clone(),
        message: "change".to_string(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "2026-10-29T12:00:00Z".to_string(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: vec![CommitFileChange::new(path.to_path_buf(), kind)],
    }));
    repo.history_state.commit_details_rev = repo.history_state.commit_details_rev.wrapping_add(1);
    repo.diff_state.diff_target = Some(DiffTarget::commit(commit_id, path.to_path_buf()));
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);
}

fn seed_preview_text(
    repo: &mut RepoState,
    workdir: &std::path::Path,
    side: DiffPreviewTextSide,
    text: &str,
) -> std::path::PathBuf {
    let source = workdir.join(".one_sided_preview_source");
    std::fs::write(&source, text).expect("write preview source");
    repo.diff_state.diff = Loadable::Error("a one-sided change loads no patch".into());
    repo.diff_state.diff_file = Loadable::Error("a one-sided change loads no file diff".into());
    repo.diff_state.diff_preview_text_file = Loadable::Ready(Some(Arc::new(DiffPreviewTextFile {
        path: source.clone(),
        side,
        large_file: None,
    })));
    source
}

fn push_repo(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: RepoId,
    seed: impl FnOnce(&mut RepoState),
    workdir: &std::path::Path,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, workdir);
            seed(&mut repo);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
}

fn wait_for_preview_text(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    display_path: std::path::PathBuf,
    text: &'static str,
) {
    wait_for_main_pane_condition(
        cx,
        view,
        "the present side loads as a whole-file preview",
        |pane| {
            pane.worktree_preview_path.as_ref() == Some(&display_path)
                && matches!(pane.worktree_preview, Loadable::Ready(_))
                && pane.worktree_preview_text.as_ref() == text
        },
        |pane| {
            format!(
                "body={:?} preview_path={:?} preview={:?}",
                pane.diff_surface().body,
                pane.worktree_preview_path,
                pane.worktree_preview,
            )
        },
    );
    crate::view::test_support::redraw(cx);
}

fn assert_single_canvas(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) {
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.diff_view, DiffViewMode::Split);
        assert_eq!(pane.diff_surface().body, DiffBody::FilePreview);
    });
    assert!(cx.debug_bounds("diff_view_toggle").is_none());
    assert!(cx.debug_bounds("diff_inline").is_none());
}

fn set_split(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) {
    cx.update(|_window, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.set_diff_view_mode(DiffViewMode::Split, cx)
            });
    });
}

#[gpui::test]
fn comparison_deleted_file_shows_only_its_old_side(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    let workdir = test_workdir("comparison_deleted_one_sided");
    let path = std::path::PathBuf::from("gone.txt");
    push_repo(
        cx,
        &view,
        RepoId(901),
        |repo| {
            seed_comparison(repo, &path, FileStatusKind::Deleted);
            seed_preview_text(repo, &workdir, DiffPreviewTextSide::Old, "old\n");
        },
        &workdir,
    );

    wait_for_preview_text(cx, &view, workdir.join(&path), "old\n");
    assert_single_canvas(cx, &view);
    let _ = std::fs::remove_dir_all(&workdir);
}

/// The old side is a blob, so a directory now at the path in the checkout
/// (`gone.txt` replaced by `gone.txt/inner`) must not hide it: the preview
/// plan loads no patch, so a fallback to the file diff showed nothing.
#[gpui::test]
fn comparison_deleted_file_previews_when_its_path_is_now_a_directory(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    let workdir = test_workdir("comparison_deleted_now_directory");
    let path = std::path::PathBuf::from("gone.txt");
    std::fs::create_dir_all(workdir.join(&path)).expect("directory at the deleted path");
    push_repo(
        cx,
        &view,
        RepoId(904),
        |repo| {
            seed_comparison(repo, &path, FileStatusKind::Deleted);
            seed_preview_text(repo, &workdir, DiffPreviewTextSide::Old, "old\n");
        },
        &workdir,
    );

    wait_for_preview_text(cx, &view, workdir.join(&path), "old\n");
    assert_single_canvas(cx, &view);
    let _ = std::fs::remove_dir_all(&workdir);
}

/// A comparison against the working tree previews an added file straight off
/// disk; the reducer's reload hands back the same path, so only the disk
/// notice tells the reader the file changed.
#[gpui::test]
fn worktree_comparison_added_file_notices_disk_edits(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    let workdir = test_workdir("worktree_comparison_added_disk");
    let path = std::path::PathBuf::from("live.txt");
    let abs = workdir.join(&path);
    std::fs::write(&abs, "before\n").expect("write live file");
    let seed = |repo: &mut RepoState, rev: u64| {
        seed_comparison_to(repo, &path, FileStatusKind::Added, None);
        repo.diff_state.diff = Loadable::Error("a one-sided change loads no patch".into());
        repo.diff_state.diff_file = Loadable::Error("a one-sided change loads no file diff".into());
        repo.diff_state.diff_preview_text_file =
            Loadable::Ready(Some(Arc::new(DiffPreviewTextFile {
                path: abs.clone(),
                side: DiffPreviewTextSide::New,
                large_file: None,
            })));
        repo.worktree_change_rev = rev;
    };
    push_repo(cx, &view, RepoId(905), |repo| seed(repo, 0), &workdir);
    wait_for_preview_text(cx, &view, abs.clone(), "before\n");

    std::fs::write(&abs, "after\n").expect("external edit");
    push_repo(cx, &view, RepoId(905), |repo| seed(repo, 1), &workdir);
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(
            pane.file_disk_notice.is_some(),
            "an edit to the live side must raise the disk notice"
        );
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn comparison_added_file_shows_only_its_new_side(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    let workdir = test_workdir("comparison_added_one_sided");
    let path = std::path::PathBuf::from("new.txt");
    push_repo(
        cx,
        &view,
        RepoId(902),
        |repo| {
            seed_comparison(repo, &path, FileStatusKind::Added);
            seed_preview_text(repo, &workdir, DiffPreviewTextSide::New, "new\n");
        },
        &workdir,
    );

    wait_for_preview_text(cx, &view, workdir.join(&path), "new\n");
    assert_single_canvas(cx, &view);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn comparison_modified_file_keeps_the_split_diff(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    let workdir = test_workdir("comparison_modified_two_sided");
    let path = std::path::PathBuf::from("both.txt");
    push_repo(
        cx,
        &view,
        RepoId(903),
        |repo| {
            seed_comparison(repo, &path, FileStatusKind::Modified);
            repo.diff_state.diff_file =
                Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some("old\n".into()),
                    Some("new\n".into()),
                ))));
            repo.diff_state.diff_file_rev = 1;
        },
        &workdir,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.one_sided_change_side(), None);
        assert_eq!(pane.diff_surface().body, DiffBody::FileDiff);
    });
    assert!(cx.debug_bounds("diff_inline").is_some());
    let _ = std::fs::remove_dir_all(&workdir);
}

/// `git add -N` lists the file as added in the unstaged area.
#[gpui::test]
fn intent_to_add_file_shows_only_its_new_side(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    let workdir = test_workdir("intent_to_add_one_sided");
    let path = std::path::PathBuf::from("planned.txt");
    std::fs::write(workdir.join(&path), "planned\n").expect("write intent-to-add file");
    push_repo(
        cx,
        &view,
        RepoId(904),
        |repo| {
            set_test_file_status(
                repo,
                path.clone(),
                FileStatusKind::Added,
                DiffArea::Unstaged,
            );
            seed_preview_text(repo, &workdir, DiffPreviewTextSide::New, "planned\n");
        },
        &workdir,
    );

    wait_for_preview_text(cx, &view, workdir.join(&path), "planned\n");
    assert_single_canvas(cx, &view);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn added_svg_code_view_shows_only_its_new_side(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    cx.update(|_window, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, _cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Svg, RenderedPreviewMode::Source);
            });
    });
    let workdir = test_workdir("added_svg_code_one_sided");
    let path = std::path::PathBuf::from("logo.svg");
    push_repo(
        cx,
        &view,
        RepoId(905),
        |repo| {
            seed_commit(repo, &path, FileStatusKind::Added);
            seed_preview_text(repo, &workdir, DiffPreviewTextSide::New, "<svg/>\n");
        },
        &workdir,
    );

    wait_for_preview_text(cx, &view, workdir.join(&path), "<svg/>\n");
    assert_single_canvas(cx, &view);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(
            pane.diff_surface().toggle_kind,
            Some(RenderedPreviewKind::Svg)
        );
    });
    let _ = std::fs::remove_dir_all(&workdir);
}

fn show_commit_image(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: RepoId,
    kind: FileStatusKind,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
) -> std::path::PathBuf {
    let workdir = test_workdir(&format!("commit_image_{}", repo_id.0));
    let path = std::path::PathBuf::from("logo.png");
    push_repo(
        cx,
        view,
        repo_id,
        |repo| {
            seed_commit(repo, &path, kind);
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file_image = Loadable::Ready(Some(Arc::new(FileDiffImage {
                path: path.clone(),
                old: old.map(<[u8]>::to_vec),
                new: new.map(<[u8]>::to_vec),
                ..Default::default()
            })));
        },
        &workdir,
    );
    wait_for_file_image_diff_cache(cx, view, "the commit image", |pane| {
        pane.file_image_diff_cache_old.is_some() || pane.file_image_diff_cache_new.is_some()
    });
    crate::view::test_support::redraw(cx);
    workdir
}

const PNG: &[u8] =
    include_bytes!("../../../../../../../assets/linux/hicolor/32x32/apps/gitcomet.png");

#[gpui::test]
fn deleted_image_shows_one_picture(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    let workdir = show_commit_image(
        cx,
        &view,
        RepoId(906),
        FileStatusKind::Deleted,
        Some(PNG),
        None,
    );

    assert!(cx.debug_bounds("diff_image_single").is_some());
    assert!(cx.debug_bounds("diff_view_toggle").is_none());
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn modified_image_keeps_both_pictures(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    set_split(cx, &view);
    let workdir = show_commit_image(
        cx,
        &view,
        RepoId(907),
        FileStatusKind::Modified,
        Some(PNG),
        Some(PNG),
    );

    assert!(cx.debug_bounds("diff_image_container").is_some());
    assert!(cx.debug_bounds("diff_image_single").is_none());
    let _ = std::fs::remove_dir_all(&workdir);
}
