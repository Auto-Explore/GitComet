use super::*;
use gitcomet_core::domain::{
    Branch, CommitId, Remote, RemoteBranch, RepoSpec, StashEntry, Submodule, SubmoduleStatus,
    Upstream, Worktree,
};
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

fn wait_until(description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
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
fn restore_session_mode_does_not_seed_empty_session_from_initial_repository() {
    assert!(!should_seed_initial_repository_from_session(
        GitCometViewMode::Normal,
        Some(Path::new("/repo")),
        InitialRepositoryLaunchMode::RestoreSession,
        false,
    ));
}

#[test]
fn restore_session_mode_keeps_initial_repository_when_session_has_saved_repos() {
    assert!(should_seed_initial_repository_from_session(
        GitCometViewMode::Normal,
        Some(Path::new("/repo")),
        InitialRepositoryLaunchMode::RestoreSession,
        true,
    ));
}

#[test]
fn explicit_initial_repository_mode_seeds_empty_session() {
    assert!(should_seed_initial_repository_from_session(
        GitCometViewMode::Normal,
        Some(Path::new("/repo")),
        InitialRepositoryLaunchMode::OpenExplicitly,
        false,
    ));
}

#[test]
fn splash_backdrop_embedded_png_decodes() {
    assert_eq!(
        super::splash::load_splash_backdrop_image().format(),
        gpui::ImageFormat::Png,
        "expected splash backdrop image to decode from embedded PNG bytes"
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
        unstaged_anchor_index: None,
        staged: vec![c.clone()],
        staged_anchor: Some(c),
        staged_anchor_index: None,
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
            BranchSidebarRow::RemoteHeader { name, .. } => Some(name.as_ref().to_owned()),
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
fn branch_sidebar_branch_label_uses_leaf_segment() {
    assert_eq!(
        branch_sidebar::branch_sidebar_branch_label("origin/feature/topic"),
        "topic"
    );
    assert_eq!(
        branch_sidebar::branch_sidebar_branch_label("feature"),
        "feature"
    );
}

#[test]
fn branch_sidebar_keeps_leaf_before_children_when_branch_is_also_group() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "feature".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature/topic".to_string(),
            target: CommitId("feedface".into()),
            upstream: None,
            divergence: None,
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let feature_group_index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                BranchSidebarRow::GroupHeader { label, depth, .. }
                    if label.as_ref() == "feature/" && *depth == 0
            )
        })
        .expect("expected feature group header");
    let feature_leaf_index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch { name, depth, .. }
                    if name.as_ref() == "feature" && *depth == 1
            )
        })
        .expect("expected feature branch row");
    let feature_child_index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch { name, depth, .. }
                    if name.as_ref() == "feature/topic" && *depth == 1
            )
        })
        .expect("expected feature/topic branch row");

    assert!(feature_group_index < feature_leaf_index);
    assert!(feature_leaf_index < feature_child_index);
}

#[test]
fn branch_sidebar_sorts_unsorted_local_branches() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "feature/topic".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "zeta".to_string(),
            target: CommitId("feedface".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature".to_string(),
            target: CommitId("cafebabe".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "alpha".to_string(),
            target: CommitId("8badf00d".into()),
            upstream: None,
            divergence: None,
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch { name, .. } => Some(name.as_ref().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["feature", "feature/topic", "alpha", "zeta"]);
}

#[test]
fn branch_sidebar_sorts_unsorted_remote_branches() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "zeta/topic".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "feature/topic".to_string(),
            target: CommitId("feedface".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "alpha".to_string(),
            target: CommitId("cafebabe".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "feature".to_string(),
            target: CommitId("8badf00d".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "alpha".to_string(),
            target: CommitId("decafbad".into()),
        },
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "main".to_string(),
            target: CommitId("facefeed".into()),
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } => Some(name.as_ref().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "origin/feature",
            "origin/feature/topic",
            "origin/alpha",
            "upstream/zeta/topic",
            "upstream/main",
        ]
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

    let expanded_key = branch_sidebar::expanded_default_section_storage_key(
        branch_sidebar::worktrees_section_storage_key(),
    )
    .expect("worktrees should support explicit expansion");
    let rows = GitCometView::branch_sidebar_rows_with_collapsed(&repo, &[expanded_key.as_str()]);
    let row = rows
        .iter()
        .find_map(|row| match row {
            BranchSidebarRow::WorktreeItem {
                path,
                branch,
                detached,
                ..
            } => Some(
                branch_sidebar::branch_sidebar_worktree_label(
                    branch.as_ref().map(SharedString::as_ref),
                    *detached,
                    &path.to_string_lossy(),
                )
                .as_ref()
                .to_owned(),
            ),
            _ => None,
        })
        .expect("expected worktree row");

    assert_eq!(row, "feature/tooltip  linked-worktree");
}

#[test]
fn branch_sidebar_defaults_secondary_sections_to_collapsed() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: PathBuf::from("linked-worktree"),
        head: None,
        branch: Some("main".to_string()),
        detached: false,
    }]));
    repo.submodules = Loadable::Ready(Arc::new(vec![Submodule {
        path: PathBuf::from("vendor/lib"),
        head: CommitId("beadfeed".into()),
        status: SubmoduleStatus::UpToDate,
    }]));
    repo.stashes = Loadable::Ready(Arc::new(vec![StashEntry {
        index: 0,
        id: CommitId("c0ffee".into()),
        message: "stash message".into(),
        created_at: None,
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);

    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::WorktreesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected Worktrees to start collapsed"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SubmodulesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected Submodules to start collapsed"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::StashHeader {
                collapsed: true,
                ..
            }
        )),
        "expected Stash to start collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::WorktreeItem { .. })),
        "expected Worktrees rows to stay hidden until expanded"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::SubmoduleItem { .. })),
        "expected Submodules rows to stay hidden until expanded"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::StashItem { .. })),
        "expected Stash rows to stay hidden until expanded"
    );
}

#[test]
fn branch_sidebar_starts_with_local_and_remote_branch_sections() {
    let repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    let rows = GitCometView::branch_sidebar_rows(&repo);
    assert!(
        matches!(
            rows.first(),
            Some(BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                ..
            })
        ),
        "expected Local Branches header to be the first sidebar row"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                ..
            }
        )),
        "expected Remote branches header to be present"
    );
}

#[test]
fn branch_sidebar_sorts_groups_before_branches_case_insensitively() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "zeta".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "topic/zeta".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "Alpha".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "topic/beta".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "topic/Alpha".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/zeta".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "Main".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/beta".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/Alpha".to_string(),
            target: CommitId("deadbeef".into()),
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let local_names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch {
                section: BranchSection::Local,
                name,
                ..
            } => Some(name.as_ref().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let remote_names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } => Some(name.as_ref().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        local_names,
        vec![
            "topic/Alpha".to_string(),
            "topic/beta".to_string(),
            "topic/zeta".to_string(),
            "Alpha".to_string(),
            "zeta".to_string(),
        ]
    );
    assert_eq!(
        remote_names,
        vec![
            "origin/release/Alpha".to_string(),
            "origin/release/beta".to_string(),
            "origin/release/zeta".to_string(),
            "origin/Main".to_string(),
        ]
    );
}

#[test]
fn branch_sidebar_collapses_branch_sections_without_hiding_other_sections() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: PathBuf::from("linked-worktree"),
        head: None,
        branch: Some("main".to_string()),
        detached: false,
    }]));
    repo.submodules = Loadable::Ready(Arc::new(vec![Submodule {
        path: PathBuf::from("vendor/lib"),
        head: CommitId("beadfeed".into()),
        status: SubmoduleStatus::UpToDate,
    }]));
    repo.stashes = Loadable::Ready(Arc::new(vec![StashEntry {
        index: 0,
        id: CommitId("c0ffee".into()),
        message: "stash message".into(),
        created_at: None,
    }]));

    let rows = GitCometView::branch_sidebar_rows_with_collapsed(
        &repo,
        &[
            branch_sidebar::local_section_storage_key(),
            branch_sidebar::remote_section_storage_key(),
            branch_sidebar::worktrees_section_storage_key(),
            branch_sidebar::submodules_section_storage_key(),
            branch_sidebar::stash_section_storage_key(),
        ],
    );

    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Local Branches header"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Remote branches header"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::Branch { .. })),
        "expected branch rows to be hidden when Local and Remote sections are collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::RemoteHeader { .. })),
        "expected remote headers to be hidden when Remote branches is collapsed"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::WorktreesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Worktrees header"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SubmodulesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Submodules header"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::StashHeader {
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Stash header"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::WorktreeItem { .. })),
        "expected worktree rows to be hidden when Worktrees is collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::SubmoduleItem { .. })),
        "expected submodule rows to be hidden when Submodules is collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::StashItem { .. })),
        "expected stash rows to be hidden when Stash is collapsed"
    );
}

#[test]
fn branch_sidebar_collapses_local_branch_groups() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "feature".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature/one".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature/two".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
    ]));

    let feature_group_key = branch_sidebar::local_group_storage_key("feature");
    let rows =
        GitCometView::branch_sidebar_rows_with_collapsed(&repo, &[feature_group_key.as_str()]);

    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::GroupHeader {
                label,
                collapsed: true,
                ..
            } if label.as_ref() == "feature/"
        )
    }));
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::Branch { name, .. } if name.as_ref() == "main"
        )
    }));
    for hidden in ["feature", "feature/one", "feature/two"] {
        assert!(
            !rows.iter().any(|row| {
                matches!(
                    row,
                    BranchSidebarRow::Branch { name, .. } if name.as_ref() == hidden
                )
            }),
            "expected {hidden} to be hidden by collapsed feature/ group"
        );
    }
}

#[test]
fn branch_sidebar_collapses_local_section_without_hiding_remote_rows() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows_with_collapsed(
        &repo,
        &[branch_sidebar::local_section_storage_key()],
    );

    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                collapsed: true,
                ..
            }
        )
    }));
    assert!(
        !rows.iter().any(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch {
                    section: BranchSection::Local,
                    ..
                }
            )
        }),
        "expected local branches to be hidden when Local section is collapsed"
    );
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::RemoteHeader { name, .. } if name.as_ref() == "origin"
        )
    }));
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } if name.as_ref() == "origin/main"
        )
    }));
}

#[test]
fn branch_sidebar_collapses_remote_section_and_remote_groups() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/one".to_string(),
            target: CommitId("deadbeef".into()),
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows_with_collapsed(
        &repo,
        &[branch_sidebar::remote_section_storage_key()],
    );
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                collapsed: true,
                ..
            }
        )
    }));
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::RemoteHeader { .. })),
        "expected remote rows to be hidden when Remote section is collapsed"
    );

    let origin_key = branch_sidebar::remote_header_storage_key("origin");
    let rows = GitCometView::branch_sidebar_rows_with_collapsed(&repo, &[origin_key.as_str()]);
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::RemoteHeader {
                name,
                collapsed: true,
                ..
            } if name.as_ref() == "origin"
        )
    }));
    assert!(
        !rows.iter().any(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch {
                    section: BranchSection::Remote,
                    ..
                }
            )
        }),
        "expected origin branches to be hidden when the remote group is collapsed"
    );
}

#[test]
fn branch_sidebar_exposes_stable_collapse_keys_for_persistence() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/one".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "release/one".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);

    let local_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::SectionHeader {
            section: BranchSection::Local,
            collapse_key,
            ..
        } => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(local_key, Some(branch_sidebar::local_section_storage_key()));

    let remote_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::SectionHeader {
            section: BranchSection::Remote,
            collapse_key,
            ..
        } => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        remote_key,
        Some(branch_sidebar::remote_section_storage_key())
    );

    let origin_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::RemoteHeader {
            name, collapse_key, ..
        } if name.as_ref() == "origin" => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        origin_key,
        Some(branch_sidebar::remote_header_storage_key("origin").as_str())
    );

    let local_group_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::GroupHeader {
            label,
            collapse_key,
            ..
        } if label.as_ref() == "feature/" => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        local_group_key,
        Some(branch_sidebar::local_group_storage_key("feature").as_str())
    );

    let remote_group_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::GroupHeader {
            label,
            collapse_key,
            ..
        } if label.as_ref() == "release/" => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        remote_group_key,
        Some(branch_sidebar::remote_group_storage_key("origin", "release").as_str())
    );
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
fn repository_entry_interstitial_helpers_distinguish_loading_and_splash() {
    assert!(repository_entry_interstitial_active(
        GitCometViewMode::Normal,
        false
    ));
    assert!(should_show_startup_repository_loading_screen(
        GitCometViewMode::Normal,
        false,
        true
    ));
    assert!(!should_show_splash_screen(
        GitCometViewMode::Normal,
        false,
        true
    ));
    assert!(should_show_splash_screen(
        GitCometViewMode::Normal,
        false,
        false
    ));
    assert!(!repository_entry_interstitial_active(
        GitCometViewMode::Normal,
        true
    ));
    assert!(titlebar_workspace_actions_enabled(
        GitCometViewMode::FocusedMergetool,
        false
    ));
    assert!(!titlebar_workspace_actions_enabled(
        GitCometViewMode::Normal,
        false
    ));
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

#[gpui::test]
fn splash_screen_renders_when_no_repositories_are_open(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
        let _ = window.draw(app);
    });

    cx.debug_bounds("repository_entry_screen")
        .expect("expected repository entry splash screen");
    cx.debug_bounds("splash_headline")
        .expect("expected splash headline");
    cx.debug_bounds("splash_open_repo_action")
        .expect("expected splash open repository button");
    cx.debug_bounds("splash_clone_repo_action")
        .expect("expected splash clone repository button");

    #[cfg(not(target_os = "macos"))]
    assert!(
        cx.debug_bounds("app_menu").is_none(),
        "expected app menu button to be hidden on the splash screen"
    );

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
    assert!(splash_active, "expected splash screen to be active");
}

#[gpui::test]
fn splash_backdrop_renders_native_layers(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
        let _ = window.draw(app);
    });

    cx.debug_bounds("splash_backdrop_native")
        .expect("expected native splash backdrop root");
    cx.debug_bounds("splash_backdrop_image")
        .expect("expected SVG-backed splash image layer");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).splash_backdrop_image.format(),
            gpui::ImageFormat::Png,
            "expected splash backdrop to be preloaded before the first draw"
        );
    });
    assert!(
        cx.debug_bounds("splash_backdrop_glow_layer").is_none(),
        "expected legacy procedural glow layer to be removed"
    );
    assert!(
        cx.debug_bounds("splash_backdrop_star_layer").is_none(),
        "expected animated star overlay to be removed"
    );
    assert!(
        cx.debug_bounds("splash_backdrop_center").is_none(),
        "expected legacy centered backdrop container to be removed"
    );

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
    assert!(splash_active, "expected splash screen to remain active");
}

#[gpui::test]
fn splash_screen_buttons_publish_expected_tooltips(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
        let _ = window.draw(app);
    });

    let open_center = cx
        .debug_bounds("splash_open_repo_action")
        .expect("expected splash open repository button")
        .center();
    cx.simulate_mouse_move(open_center, None, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .tooltip_text_for_test(app)
                .map(|text| text.to_string()),
            Some("Open repository".to_string())
        );
    });

    let clone_center = cx
        .debug_bounds("splash_clone_repo_action")
        .expect("expected splash clone repository button")
        .center();
    cx.simulate_mouse_move(clone_center, None, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .tooltip_text_for_test(app)
                .map(|text| text.to_string()),
            Some("Clone repository".to_string())
        );
    });
}

#[gpui::test]
fn closing_last_repository_tab_returns_to_splash_screen(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from(
        "/tmp/repository-entry-screen-test",
    )));
    wait_until("repository tab to be added", || {
        !store_for_assert.snapshot().repos.is_empty()
    });
    pump_for(cx, Duration::from_millis(120));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
    assert!(
        !splash_active,
        "expected splash screen to disappear after opening a repo"
    );

    #[cfg(not(target_os = "macos"))]
    assert!(
        cx.debug_bounds("app_menu").is_some(),
        "expected app menu button to be visible once a repo tab exists"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(
                this.close_active_repo_tab(cx),
                "expected the active repo tab to close"
            );
        });
    });

    wait_until("last repository tab to close", || {
        store_for_assert.snapshot().repos.is_empty()
    });
    pump_for(cx, Duration::from_millis(120));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("repository_entry_screen")
        .expect("expected splash screen after closing the last repo");

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
    assert!(
        splash_active,
        "expected splash screen to return after closing the last repo"
    );
}

#[gpui::test]
fn splash_screen_clears_stale_close_repository_tooltip(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from(
        "/tmp/splash-tooltip-clear-test",
    )));
    wait_until("repository tab to be added", || {
        !store_for_assert.snapshot().repos.is_empty()
    });
    pump_for(cx, Duration::from_millis(120));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let _ = this.tooltip_host.update(cx, |host, cx| {
                host.set_tooltip_text_if_changed(Some("Close repository".into()), cx);
            });
        });
        assert_eq!(
            view.read(app)
                .tooltip_text_for_test(app)
                .map(|text| text.to_string()),
            Some("Close repository".to_string())
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(
                this.close_active_repo_tab(cx),
                "expected the active repo tab to close"
            );
        });
    });

    wait_until("last repository tab to close", || {
        store_for_assert.snapshot().repos.is_empty()
    });
    pump_for(cx, Duration::from_millis(120));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).tooltip_text_for_test(app),
            None,
            "expected splash transition to clear stale repository-close tooltip text"
        );
    });
}

#[test]
fn generic_error_banner_is_hidden_when_auth_prompt_is_active() {
    assert!(GitCometView::should_render_generic_error_banner(false));
    assert!(!GitCometView::should_render_generic_error_banner(true));
}

#[test]
fn auth_prompt_banner_colors_use_accent_palette() {
    let theme = AppTheme::gitcomet_light();
    let (bg, border) = GitCometView::auth_prompt_banner_colors(theme);

    assert_eq!(bg, with_alpha(theme.colors.accent, 0.15));
    assert_eq!(border, with_alpha(theme.colors.accent, 0.3));
}

#[gpui::test]
fn apply_state_snapshot_routes_command_errors_into_store_backed_banner(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let error = "Fetch failed".to_string();
    let mut next = AppState::default();
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.last_error = Some(error.clone());
    repo.command_log
        .push(gitcomet_state::model::CommandLogEntry {
            time: std::time::SystemTime::now(),
            ok: false,
            command: "git fetch".to_string(),
            summary: error.clone(),
            stdout: String::new(),
            stderr: "fatal: test".to_string(),
        });
    next.active_repo = Some(repo_id);
    next.repos.push(repo);
    let next = Arc::new(next);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
        });
    });

    wait_until("store-backed banner error", || {
        let snapshot = store_for_assert.snapshot();
        snapshot
            .banner_error
            .as_ref()
            .is_some_and(|banner| banner.repo_id == Some(repo_id) && banner.message == error)
    });
}
