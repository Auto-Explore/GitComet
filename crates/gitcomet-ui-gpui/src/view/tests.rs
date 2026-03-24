use super::*;
use gitcomet_core::domain::{Branch, CommitId, Remote, RemoteBranch, RepoSpec, Upstream, Worktree};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use gitcomet_state::store::AppStore;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "Test backend does not open repositories",
        )))
    }
}

fn pump_for(cx: &mut gpui::VisualTestContext, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(16));
    }
}

#[test]
fn toast_total_lifetime_includes_fade_in_and_out() {
    let ttl = Duration::from_secs(6);
    assert_eq!(
        toast_total_lifetime(ttl),
        ttl + Duration::from_millis(TOAST_FADE_IN_MS + TOAST_FADE_OUT_MS)
    );
}

#[test]
fn reconcile_status_multi_selection_prunes_missing_paths_and_anchors() {
    let a = PathBuf::from("a.txt");
    let b = PathBuf::from("b.txt");
    let c = PathBuf::from("c.txt");

    let status = RepoStatus {
        staged: vec![],
        unstaged: vec![FileStatus {
            path: a.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }],
    };

    let mut selection = StatusMultiSelection {
        untracked: vec![],
        untracked_anchor: None,
        unstaged: vec![a.clone(), b.clone()],
        unstaged_anchor: Some(b),
        staged: vec![c.clone()],
        staged_anchor: Some(c),
    };

    reconcile_status_multi_selection(&mut selection, &status);

    assert_eq!(selection.unstaged, vec![a]);
    assert!(selection.unstaged_anchor.is_none());
    assert!(selection.staged.is_empty());
    assert!(selection.staged_anchor.is_none());
}

#[test]
fn remote_rows_groups_and_sorts() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "b".to_string(),
            target: CommitId("b0".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "a".to_string(),
            target: CommitId("a0".into()),
        },
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "main".to_string(),
            target: CommitId("c0".into()),
        },
    ]));

    let rows = GitCometView::remote_rows(&repo);
    assert_eq!(
        rows,
        vec![
            RemoteRow::Header("origin".to_string()),
            RemoteRow::Branch {
                remote: "origin".to_string(),
                name: "a".to_string()
            },
            RemoteRow::Branch {
                remote: "origin".to_string(),
                name: "b".to_string()
            },
            RemoteRow::Header("upstream".to_string()),
            RemoteRow::Branch {
                remote: "upstream".to_string(),
                name: "main".to_string()
            },
        ]
    );
}

#[test]
fn remote_headers_include_remotes_with_no_branches() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.remotes = Loadable::Ready(Arc::new(vec![
        Remote {
            name: "origin".to_string(),
            url: Some("https://example.com/origin.git".to_string()),
        },
        Remote {
            name: "upstream".to_string(),
            url: Some("https://example.com/upstream.git".to_string()),
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let mut headers = rows
        .iter()
        .filter_map(|r| match r {
            BranchSidebarRow::RemoteHeader { name } => Some(name.as_ref().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    headers.sort_unstable();
    headers.dedup();

    assert!(
        headers.contains(&"origin".to_string()),
        "expected origin remote header"
    );
    assert!(
        headers.contains(&"upstream".to_string()),
        "expected upstream remote header"
    );
}

#[test]
fn remote_upstream_branch_is_marked() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.head_branch = Loadable::Ready("main".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        }),
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let upstream_row = rows.iter().find(|r| {
        matches!(
            r,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                is_upstream: true,
                ..
            } if name.as_ref() == "origin/main"
        )
    });
    assert!(
        upstream_row.is_some(),
        "expected origin/main to be marked as upstream"
    );
}

#[test]
fn remote_section_includes_tracked_upstream_without_remote_tracking_ref() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.head_branch = Loadable::Ready("feature".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        }),
        divergence: None,
    }]));
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "origin".to_string(),
        url: Some("https://example.com/origin.git".to_string()),
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let tracked_row = rows.iter().find(|r| {
        matches!(
            r,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                is_upstream: true,
                ..
            } if name.as_ref() == "origin/feature"
        )
    });
    assert!(
        tracked_row.is_some(),
        "expected tracked upstream branch to be listed under Remote section"
    );
}

#[test]
fn worktree_tooltip_includes_branch_name() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("main-worktree"),
        },
    );

    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: PathBuf::from("linked-worktree"),
        head: None,
        branch: Some("feature/tooltip".to_string()),
        detached: false,
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let row = rows
        .iter()
        .find_map(|row| match row {
            BranchSidebarRow::WorktreeItem { tooltip, .. } => Some(tooltip.as_ref().to_owned()),
            _ => None,
        })
        .expect("expected worktree row");

    assert_eq!(row, "feature/tooltip  linked-worktree");
}

#[test]
fn resize_edge_detects_edges_and_corners() {
    let window_size = size(px(100.0), px(100.0));
    let tiling = Tiling::default();
    let inset = px(10.0);

    assert_eq!(
        resize_edge(point(px(0.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::TopLeft)
    );
    assert_eq!(
        resize_edge(point(px(99.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::TopRight)
    );
    assert_eq!(
        resize_edge(point(px(0.0), px(99.0)), inset, window_size, tiling),
        Some(ResizeEdge::BottomLeft)
    );
    assert_eq!(
        resize_edge(point(px(99.0), px(99.0)), inset, window_size, tiling),
        Some(ResizeEdge::BottomRight)
    );

    assert_eq!(
        resize_edge(point(px(50.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::Top)
    );
    assert_eq!(
        resize_edge(point(px(50.0), px(99.0)), inset, window_size, tiling),
        Some(ResizeEdge::Bottom)
    );
    assert_eq!(
        resize_edge(point(px(0.0), px(50.0)), inset, window_size, tiling),
        Some(ResizeEdge::Left)
    );
    assert_eq!(
        resize_edge(point(px(99.0), px(50.0)), inset, window_size, tiling),
        Some(ResizeEdge::Right)
    );

    assert_eq!(
        resize_edge(point(px(50.0), px(50.0)), inset, window_size, tiling),
        None
    );
}

#[test]
fn resize_edge_respects_tiling() {
    let window_size = size(px(100.0), px(100.0));
    let inset = px(10.0);
    let tiling = Tiling {
        top: true,
        left: false,
        right: false,
        bottom: false,
    };

    assert_eq!(
        resize_edge(point(px(0.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::Left)
    );
    assert_eq!(
        resize_edge(point(px(50.0), px(0.0)), inset, window_size, tiling),
        None
    );
    assert_eq!(
        resize_edge(point(px(0.0), px(50.0)), inset, window_size, tiling),
        Some(ResizeEdge::Left)
    );
}

#[test]
fn cursor_style_matches_resize_edge() {
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::Left),
        CursorStyle::ResizeLeftRight
    );
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::Top),
        CursorStyle::ResizeUpDown
    );
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::TopLeft),
        CursorStyle::ResizeUpLeftDownRight
    );
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::TopRight),
        CursorStyle::ResizeUpRightDownLeft
    );
}

#[test]
fn is_markdown_path_detects_common_extensions() {
    use std::path::Path;
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_markdown_path(Path::new("doc.markdown")));
    assert!(is_markdown_path(Path::new("notes.mdown")));
    assert!(is_markdown_path(Path::new("CHANGES.mkd")));
    assert!(is_markdown_path(Path::new("file.mkdn")));
    assert!(is_markdown_path(Path::new("file.mdwn")));
    assert!(is_markdown_path(Path::new("UPPER.MD")));
}

#[test]
fn is_markdown_path_rejects_non_markdown() {
    use std::path::Path;
    assert!(!is_markdown_path(Path::new("file.txt")));
    assert!(!is_markdown_path(Path::new("file.rs")));
    assert!(!is_markdown_path(Path::new("file")));
}

#[test]
fn should_bypass_text_file_preview_for_path_detects_supported_image_types() {
    use std::path::Path;

    for path in [
        "image.png",
        "image.JPEG",
        "image.gif",
        "image.webp",
        "image.bmp",
        "image.ico",
        "image.svg",
        "image.tif",
        "image.tiff",
    ] {
        assert!(
            should_bypass_text_file_preview_for_path(Path::new(path)),
            "expected {path} to bypass text file preview"
        );
    }

    for path in ["image.heic", "README.md", "notes.txt", "image"] {
        assert!(
            !should_bypass_text_file_preview_for_path(Path::new(path)),
            "did not expect {path} to bypass text file preview"
        );
    }
}

#[test]
fn preview_path_rendered_kind_detects_supported_preview_kinds() {
    use std::path::Path;

    assert_eq!(
        preview_path_rendered_kind(Path::new("diagram.svg")),
        Some(RenderedPreviewKind::Svg)
    );
    assert_eq!(
        preview_path_rendered_kind(Path::new("README.md")),
        Some(RenderedPreviewKind::Markdown)
    );
    assert_eq!(preview_path_rendered_kind(Path::new("notes.txt")), None);
}

#[test]
fn diff_target_rendered_preview_kind_reads_diff_target_paths() {
    let svg_target = DiffTarget::WorkingTree {
        path: PathBuf::from("diagram.svg"),
        area: DiffArea::Unstaged,
    };
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&svg_target)),
        Some(RenderedPreviewKind::Svg)
    );

    let markdown_target = DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: Some(PathBuf::from("README.md")),
    };
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&markdown_target)),
        Some(RenderedPreviewKind::Markdown)
    );

    let no_path_target = DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: None,
    };
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&no_path_target)),
        None
    );
}

#[test]
fn main_diff_rendered_preview_toggle_kind_matches_supported_modes() {
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(true, false, Some(RenderedPreviewKind::Svg),),
        Some(RenderedPreviewKind::Svg)
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(true, false, Some(RenderedPreviewKind::Markdown),),
        Some(RenderedPreviewKind::Markdown)
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(false, true, Some(RenderedPreviewKind::Markdown),),
        Some(RenderedPreviewKind::Markdown)
    );
}

#[test]
fn rendered_preview_modes_track_each_kind_independently() {
    let mut modes = RenderedPreviewModes::default();

    assert_eq!(
        modes.get(RenderedPreviewKind::Svg),
        RenderedPreviewMode::Rendered
    );
    assert_eq!(
        modes.get(RenderedPreviewKind::Markdown),
        RenderedPreviewMode::Rendered
    );

    modes.set(RenderedPreviewKind::Svg, RenderedPreviewMode::Source);
    modes.set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);

    assert_eq!(
        modes.get(RenderedPreviewKind::Svg),
        RenderedPreviewMode::Source
    );
    assert_eq!(
        modes.get(RenderedPreviewKind::Markdown),
        RenderedPreviewMode::Source
    );
}

#[test]
fn conflict_resolver_preview_mode_defaults_to_text() {
    assert_eq!(
        ConflictResolverPreviewMode::default(),
        ConflictResolverPreviewMode::Text
    );
}

fn focused_bootstrap(
    repo_path: PathBuf,
    conflicted_file_path: PathBuf,
) -> FocusedMergetoolBootstrap {
    FocusedMergetoolBootstrap::from_view_config(FocusedMergetoolViewConfig {
        repo_path,
        conflicted_file_path,
        labels: FocusedMergetoolLabels {
            local: "LOCAL".to_string(),
            remote: "REMOTE".to_string(),
            base: "BASE".to_string(),
        },
    })
}

fn open_repo_state_with_workdir(workdir: &str) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: normalize_bootstrap_repo_path(PathBuf::from(workdir)),
        },
    );
    repo.open = Loadable::Ready(());
    repo
}

#[test]
fn focused_mergetool_target_path_prefers_repo_relative_path() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let target = focused_mergetool_target_path(&repo, &repo.join("src/conflict.txt"));
    assert_eq!(target, PathBuf::from("src/conflict.txt"));
}

#[test]
fn focused_mergetool_bootstrap_requests_open_repo_when_missing() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let state = AppState::default();

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::OpenRepo(repo))
    );
}

#[test]
fn focused_mergetool_bootstrap_selects_worktree_diff_target() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let mut state = AppState::default();
    state.active_repo = Some(RepoId(1));
    state.repos.push(open_repo_state_with_workdir(
        repo.to_str().expect("test path should be unicode"),
    ));

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::SelectConflictDiff {
            repo_id: RepoId(1),
            path: PathBuf::from("src/conflict.txt"),
        })
    );
}

#[test]
fn focused_mergetool_bootstrap_loads_conflict_file_after_diff_target() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let mut state = AppState::default();
    state.active_repo = Some(RepoId(1));
    let mut repo_state =
        open_repo_state_with_workdir(repo.to_str().expect("test path should be unicode"));
    repo_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        area: DiffArea::Unstaged,
        path: PathBuf::from("src/conflict.txt"),
    });
    state.repos.push(repo_state);

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::LoadConflictFile {
            repo_id: RepoId(1),
            path: PathBuf::from("src/conflict.txt"),
        })
    );
}

#[test]
fn focused_mergetool_bootstrap_completes_after_conflict_file_target_set() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let mut state = AppState::default();
    state.active_repo = Some(RepoId(1));
    let mut repo_state =
        open_repo_state_with_workdir(repo.to_str().expect("test path should be unicode"));
    repo_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        area: DiffArea::Unstaged,
        path: PathBuf::from("src/conflict.txt"),
    });
    repo_state.conflict_state.conflict_file_path = Some(PathBuf::from("src/conflict.txt"));
    repo_state.conflict_state.conflict_file = Loadable::Loading;
    state.repos.push(repo_state);

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::Complete)
    );
}

#[test]
fn focused_mergetool_mode_hides_full_chrome() {
    assert!(renders_full_chrome(GitCometViewMode::Normal));
    assert!(!renders_full_chrome(GitCometViewMode::FocusedMergetool));
}

#[test]
fn ease_out_cubic_hits_expected_anchor_points() {
    assert_eq!(GitCometView::ease_out_cubic(0.0), 0.0);
    assert_eq!(GitCometView::ease_out_cubic(1.0), 1.0);
    assert!((GitCometView::ease_out_cubic(0.5) - 0.875).abs() < 1e-6);
}

#[test]
fn ease_out_cubic_is_monotonic_in_unit_interval() {
    let a = GitCometView::ease_out_cubic(0.2);
    let b = GitCometView::ease_out_cubic(0.6);
    let c = GitCometView::ease_out_cubic(0.9);
    assert!(a < b);
    assert!(b < c);
}

#[gpui::test]
fn sidebar_expand_after_collapse_does_not_reenter_root_update(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(false, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|_window, app| {
        assert!(!view.read(app).sidebar_collapsed);
    });
}

#[test]
fn generic_error_banner_is_hidden_when_auth_prompt_is_active() {
    assert!(GitCometView::should_render_generic_error_banner(false));
    assert!(!GitCometView::should_render_generic_error_banner(true));
}

#[test]
fn auth_prompt_banner_colors_use_accent_palette() {
    let theme = AppTheme::zed_one_light();
    let (bg, border) = GitCometView::auth_prompt_banner_colors(theme);

    assert_eq!(bg, with_alpha(theme.colors.accent, 0.15));
    assert_eq!(border, with_alpha(theme.colors.accent, 0.3));
}
