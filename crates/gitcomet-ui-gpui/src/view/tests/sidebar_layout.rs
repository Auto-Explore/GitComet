//! Sidebar and pane layout: branch trees, resize grips, collapse, locate.

use super::*;

#[test]
fn selected_sidebar_branch_colors_come_from_theme_interaction_tokens() {
    let mut theme = AppTheme::gitcomet_dark();
    let selected_background = gpui::rgba(0x12345678);
    let selected_foreground = gpui::rgba(0xabcdefee);
    theme.colors.interaction.selected_background = selected_background;
    theme.colors.interaction.selected_foreground = selected_foreground;

    assert_eq!(selected_branch_row_bg(theme), selected_background);
    assert_eq!(selected_branch_label_color(theme), selected_foreground);
}

#[test]
fn next_pane_resize_drag_width_recomputes_bounds_when_window_changes() {
    let state = PaneResizeState::new(
        PaneResizeHandle::Sidebar,
        px(0.0),
        px(280.0),
        px(420.0),
        px(1280.0),
        false,
        false,
    );
    let current_x = px(320.0);
    let total_w = px(900.0);
    let width = next_pane_resize_drag_width(&state, current_x, total_w, false, false);
    let (min_width, max_width) = pane_resize_drag_width_bounds(
        PaneResizeHandle::Sidebar,
        px(280.0),
        px(420.0),
        total_w,
        false,
        false,
    );
    let expected = (px(280.0) + current_x).max(min_width).min(max_width);

    assert_eq!(width, expected);
}

#[test]
fn diff_split_column_widths_from_available_clamps_to_min_widths() {
    let (left, right) = diff_split_column_widths_from_available(px(556.0), px(160.0), 0.95);

    assert_eq!(left, px(396.0));
    assert_eq!(right, px(160.0));
}

#[test]
fn diff_split_column_widths_from_available_falls_back_to_even_split_when_narrow() {
    let (left, right) = diff_split_column_widths_from_available(px(300.0), px(160.0), 0.95);

    assert_eq!(left, px(150.0));
    assert_eq!(right, px(150.0));
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
fn remote_section_excludes_upstream_without_remote_tracking_ref() {
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
    assert!(
        rows.iter().all(|row| {
            !matches!(
                row,
                BranchSidebarRow::Branch {
                    section: BranchSection::Remote,
                    name,
                    ..
                } if name.as_ref() == "origin/feature"
            )
        }),
        "a configured upstream must not synthesize a missing remote branch"
    );
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
        recorded_head: CommitId("beadfeed".into()),
        checked_out_head: Some(CommitId("beadfeed".into())),
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
        recorded_head: CommitId("beadfeed".into()),
        checked_out_head: Some(CommitId("beadfeed".into())),
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

#[gpui::test]
fn merge_view_temporarily_collapses_and_restores_sidebar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/conflict.rs",
        FileStatusKind::Conflicted,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(view.read(app).sidebar_collapsed));

    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/normal.rs",
        FileStatusKind::Modified,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(!view.read(app).sidebar_collapsed));

    cx.update(|_window, app| {
        view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx));
    });
    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/conflict.rs",
        FileStatusKind::Conflicted,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(view.read(app).sidebar_collapsed));

    cx.update(|_window, app| {
        view.update(app, |this, cx| this.set_sidebar_collapsed(false, cx));
    });
    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/normal.rs",
        FileStatusKind::Modified,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(view.read(app).sidebar_collapsed));
}

#[gpui::test]
fn sidebar_resize_handle_straddles_the_content_card_edge(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(1))));
    sync_view_snapshot(cx, &view);

    let sidebar = cx
        .debug_bounds("sidebar_pane")
        .expect("expected the sidebar pane");
    let handle = cx
        .debug_bounds("pane_resize_sidebar")
        .expect("expected the sidebar resize handle");

    // The same rule the details handle follows: the grab strip is centered on
    // the boundary it drags, so its grip lands on the rule rather than beside
    // it. Without this the strip hangs entirely inside the content card.
    assert_eq!(
        handle.center().x,
        sidebar.right(),
        "sidebar resize handle must straddle the sidebar/card boundary"
    );
}

#[gpui::test]
fn pane_resize_grips_paint_on_hover_in_the_application_layout(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(1))));
    sync_view_snapshot(cx, &view);
    cx.simulate_resize(gpui::size(px(1400.0), px(900.0)));
    test_support::redraw(cx);

    for selector in ["pane_resize_sidebar", "pane_resize_details"] {
        let handle = cx
            .debug_bounds(selector)
            .expect("resize strip should be present");
        assert!(handle.size.width > px(0.0) && handle.size.height > px(44.0));
        cx.simulate_mouse_move(handle.center(), None, gpui::Modifiers::default());
        for pressed in [false, true] {
            if pressed {
                cx.simulate_mouse_down(
                    handle.center(),
                    gpui::MouseButton::Left,
                    gpui::Modifiers::default(),
                );
            }
            cx.update(|window, app| {
                let _ = window.draw(app);
                let theme = view.read(app).theme;
                let tint = if pressed {
                    theme.colors.accent.foreground
                } else {
                    with_alpha(theme.colors.foreground.primary, if theme.is_dark { 0.34 } else { 0.30 })
                };
                let scale = window.scale_factor();
                let quad = window.painted_quads().into_iter()
                    .find(|quad| {
                        let bounds = quad.bounds;
                        quad.background == tint.into()
                            && bounds.size.width.0 > 0.0 && bounds.size.height.0 > 0.0
                            && (bounds.center().x.0 - f32::from(handle.center().x) * scale).abs() < 1.0
                            && (bounds.center().y.0 - f32::from(handle.center().y) * scale).abs() < 1.0
                    }).unwrap_or_else(|| panic!("{selector}: no grip centered in {handle:?}, pressed={pressed}"));
                let max_radius = quad.bounds.size.width.0.min(quad.bounds.size.height.0) / 2.0;
                for radius in [quad.corner_radii.top_left, quad.corner_radii.top_right, quad.corner_radii.bottom_left, quad.corner_radii.bottom_right] {
                    assert!(radius.0 <= max_radius + 0.5,
                        "{selector}: radius {radius:?} exceeds grip bounds {:?} and makes the grip invisible", quad.bounds);
                }
            });
        }
        cx.simulate_mouse_up(
            handle.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
    }
}

#[gpui::test]
fn collapsed_files_popover_uses_branch_style_rows_and_scrolls(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        (0..40)
            .map(|ix| FileEntry {
                name: format!("file_{ix}.txt"),
                path: Arc::new(PathBuf::from(format!("file_{ix}.txt"))),
                kind: FileEntryKind::File,
                depth: 0,
                ignored: false,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Files, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    let panel = cx
        .debug_bounds("collapsed_sidebar_popover")
        .expect("expected collapsed Files popover");
    assert!(
        cx.debug_bounds("file_browser_scroll_container").is_some(),
        "collapsed Files shares the virtualized file list"
    );
    let scroll =
        cx.update(|_window, app| view.read(app).sidebar_pane.read(app).list_scroll_for_test());
    assert!(
        scroll.max_offset().y > px(0.0),
        "collapsed popover scrollbar must observe overflowing rows"
    );
    assert!(
        components::Scrollbar::thumb_visible_for_test(&scroll, panel.size.height),
        "collapsed popover must render a scrollbar thumb for overflowing rows"
    );
    let surface = cx.debug_bounds("file_browser_scroll_container").unwrap();
    let search_toggle = cx.debug_bounds("collapsed_popover_filter_toggle").unwrap();
    let before = scroll.offset();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: surface.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    test_support::redraw(cx);
    assert!(scroll.offset().y < before.y);
    assert_eq!(
        cx.debug_bounds("collapsed_popover_filter_toggle").unwrap(),
        search_toggle
    );
}

#[gpui::test]
fn collapsed_branch_popover_search_keeps_its_section_scope(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store_for_view, events, None, window, cx)
    });

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/alpha".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    state.repos[0].remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "feature/beta".to_string(),
        target: CommitId("deadbeef".into()),
    }]));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    assert!(
        cx.debug_bounds("sidebar_branches_search").is_none(),
        "the popover filter must stay hidden until its header toggle is used"
    );
    let toggle = cx
        .debug_bounds("collapsed_popover_filter_toggle")
        .expect("expected a filter toggle in the branch popover header");
    let section_menu = cx
        .debug_bounds("collapsed_popover_section_menu")
        .expect("expected a section menu button in the branch popover header");
    let panel = cx
        .debug_bounds("collapsed_sidebar_popover")
        .expect("expected the collapsed branch popover");
    assert!(
        section_menu.left() >= toggle.right() && section_menu.right() <= panel.right(),
        "the header's two buttons must sit side by side inside the panel \
         (filter={toggle:?}, menu={section_menu:?}, panel={panel:?})"
    );

    cx.simulate_mouse_move(toggle.center(), None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        toggle.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        toggle.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);

    let filter_bar = cx
        .debug_bounds("sidebar_branches_search")
        .expect("expected the toggle to reveal the popover filter");
    // The branch sits under a `feature/` group header, so it is not row zero.
    let first_row = ["branch_row_1_0", "branch_row_1_1", "branch_row_1_2"]
        .into_iter()
        .find_map(|selector| cx.debug_bounds(selector))
        .expect("expected the popover to render branch rows");
    assert!(
        filter_bar.bottom() <= first_row.top(),
        "the filter box must sit above every branch row \
         (filter={filter_bar:?}, first row={first_row:?})"
    );
    assert!(
        filter_bar.top() > toggle.top(),
        "the filter box must sit below the popover header"
    );

    cx.simulate_keystrokes("b e t a");
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("branch_row_1_1").is_none(),
        "a Local search must not show a branch that only exists on Remote"
    );
    let query = cx.update(|_window, app| {
        view.read(app)
            .sidebar_pane
            .read(app)
            .branch_filter_query
            .clone()
    });
    assert_eq!(
        query, "beta",
        "keystrokes must reach the popover filter box"
    );
}

#[gpui::test]
fn collapsed_worktrees_popover_offers_its_section_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    // An empty section is the worst case: it has no rows to right-click, so
    // without the panel's own handler the click falls through to the history
    // canvas underneath (whose listener is window-level, not hitbox-gated).
    state.repos[0].worktrees = Loadable::Ready(Arc::new(vec![]));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Worktrees, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    let panel = cx
        .debug_bounds("collapsed_sidebar_popover")
        .expect("expected the collapsed Worktrees popover");
    assert!(
        cx.debug_bounds("collapsed_popover_section_menu").is_some(),
        "the popover header must expose the section's menu button"
    );

    // Low in the panel, below the header and the empty state.
    let empty_point = gpui::point(panel.center().x, panel.bottom() - px(24.0));
    cx.simulate_mouse_move(empty_point, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        empty_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        empty_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert_eq!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::worktree(
                RepoId(1),
                WorktreePopoverKind::SectionMenu
            )),
            "right-clicking the popover must open the worktrees section menu"
        );
        assert_eq!(
            view.read(app).sidebar_collapsed_popover,
            Some(CollapsedSidebarSection::Worktrees),
            "the popover must stay open behind its own context menu"
        );
    });
}

#[gpui::test]
fn collapsed_files_popover_offers_the_files_settings_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    store.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(1))));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Files, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    let button = cx
        .debug_bounds("collapsed_popover_section_menu")
        .expect("the Files popover header must expose its settings menu");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert_eq!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::ExplorerSettingsMenu { repo_id: RepoId(1) }),
        );
        assert_eq!(
            view.read(app).sidebar_collapsed_popover,
            Some(CollapsedSidebarSection::Files),
            "the rail popover must stay open behind its menu"
        );
    });
}

#[test]
fn pane_collapse_ease_is_a_well_formed_easing_curve() {
    // Endpoints are pinned.
    assert_eq!(GitCometView::pane_collapse_ease(0.0), 0.0);
    assert_eq!(GitCometView::pane_collapse_ease(1.0), 1.0);

    // Out-of-range inputs clamp to the endpoints.
    assert_eq!(GitCometView::pane_collapse_ease(-0.5), 0.0);
    assert_eq!(GitCometView::pane_collapse_ease(1.5), 1.0);

    // Monotonically non-decreasing across the domain.
    let mut prev = 0.0;
    for i in 0..=100 {
        let t = i as f32 / 100.0;
        let y = GitCometView::pane_collapse_ease(t);
        assert!(
            y >= prev - 1e-4,
            "easing should be monotonic: y({t}) = {y} < previous {prev}"
        );
        assert!(
            (0.0..=1.0).contains(&y),
            "easing stays in [0, 1]: y({t}) = {y}"
        );
        prev = y;
    }

    // Fast-out, slow-in: past the halfway mark well before the halfway time.
    assert!(GitCometView::pane_collapse_ease(0.5) > 0.5);
}

#[test]
fn cubic_bezier_matches_a_linear_curve_for_the_identity_control_points() {
    // cubic-bezier(1/3, 1/3, 2/3, 2/3) is the straight line y = x.
    for i in 0..=20 {
        let t = i as f32 / 20.0;
        let y = GitCometView::cubic_bezier(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, t);
        assert!((y - t).abs() < 1e-3, "linear bezier: y({t}) = {y}");
    }
}

#[gpui::test]
fn locate_open_file_switches_to_files_and_expands_its_folders(cx: &mut gpui::TestAppContext) {
    // The action is reachable from a shortcut, the app menu and the palette, so
    // it has to work with the sidebar on Branches and the folders collapsed.
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let nested = PathBuf::from("src/inner/deep.rs");
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: FileEntryKind::Directory,
            depth: 0,
            ignored: false,
        },
        FileEntry {
            name: "inner".to_string(),
            path: Arc::new(PathBuf::from("src/inner")),
            kind: FileEntryKind::Directory,
            depth: 1,
            ignored: false,
        },
        FileEntry {
            name: "deep.rs".to_string(),
            path: Arc::new(nested.clone()),
            kind: FileEntryKind::File,
            depth: 2,
            ignored: false,
        },
    ]));
    state.repos[0].file_browser.bump_rev();
    state.repos[0].diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
        nested.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
    state.repos[0].diff_state.content_preview = true;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| this.locate_open_file_in_explorer(cx));
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let state = view.read(app).store.snapshot();
        assert_eq!(
            state.sidebar_mode,
            gitcomet_state::model::SidebarMode::Files,
            "locating has to bring the tree it scrolls into view"
        );
        let expanded = &state.repos[0].file_browser.expanded_dirs;
        assert!(expanded.contains(&Arc::new(PathBuf::from("src"))));
        assert!(expanded.contains(&Arc::new(PathBuf::from("src/inner"))));
    });
}

#[gpui::test]
fn sidebar_tabs_grow_with_density_at_each_ui_scale(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(1))));
    sync_view_snapshot(cx, &view);
    cx.update(|_, app| view.update(app, |view, cx| view.set_sidebar_collapsed(false, cx)));
    cx.simulate_resize(gpui::size(px(1400.0), px(900.0)));

    for scale in [100, 150] {
        let mut previous: Option<[gpui::Size<Pixels>; 2]> = None;
        for density in [
            crate::appearance::UiDensity::Compact,
            crate::appearance::UiDensity::Comfortable,
            crate::appearance::UiDensity::Spacious,
        ] {
            cx.update(|_, app| {
                app.set_global(crate::appearance::Appearance {
                    density,
                    ..Default::default()
                });
                ui_scale::set_default(app, scale);
                view.update(app, |view, cx| {
                    view.notify_font_preferences_changed(cx);
                    // Real scale changes resize the panel too. Measure the
                    // natural tab widths with room for both header actions;
                    // the minimum-width search test covers constrained tabs.
                    test_support::set_sidebar_width_for_test(
                        view,
                        px(320.0 * scale as f32 / 100.0),
                        cx,
                    );
                });
            });
            test_support::redraw(cx);
            let sizes = ["sidebar_tab_branches", "sidebar_tab_files"]
                .map(|selector| cx.debug_bounds(selector).unwrap().size);
            if let Some(previous) = previous {
                for (current, previous) in sizes.iter().zip(previous) {
                    assert!(
                        current.width > previous.width,
                        "tab width must grow at {density:?}"
                    );
                    assert!(
                        current.height > previous.height,
                        "tab height must grow at {density:?}"
                    );
                }
            }
            previous = Some(sizes);
        }
    }
}

#[gpui::test]
fn each_sidebar_tab_keeps_its_own_locate_button_present(cx: &mut gpui::TestAppContext) {
    // The trailing action stays put as its data becomes available; switching
    // tabs swaps it for the action belonging to that tree.
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    assert!(
        cx.debug_bounds("sidebar_locate_open_file").is_none(),
        "the file-locate action belongs only to Files"
    );
    assert!(
        cx.debug_bounds("sidebar_locate_active_branch").is_some(),
        "Branches keeps its disabled locate action before HEAD is available"
    );

    // Files, still with no file open: present, and disabled rather than absent.
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    assert!(
        cx.debug_bounds("sidebar_locate_open_file").is_some(),
        "the locate button belongs to the Files tab, open file or not"
    );
    assert!(
        cx.debug_bounds("sidebar_locate_active_branch").is_none(),
        "the branch-locate action belongs only to Branches"
    );

    state.repos[0].diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("src/main.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
    state.repos[0].diff_state.content_preview = true;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    assert!(cx.debug_bounds("sidebar_locate_open_file").is_some());
}

#[gpui::test]
fn active_branch_locate_button_expands_scrolls_and_selects_like_its_row(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let repo_id = RepoId(1);
    let active_name = "zzzz/deep/topic";
    let active_tip = CommitId("active-branch-tip".into());
    let branch = |name: String, target: CommitId| Branch {
        name,
        target,
        upstream: None,
        divergence: None,
    };
    let mut branches = (0..80)
        .map(|ix| {
            branch(
                format!("group-{ix:03}/topic"),
                CommitId(format!("tip-{ix:03}").into()),
            )
        })
        .collect::<Vec<_>>();
    branches.push(branch(active_name.to_string(), active_tip));
    branches.push(branch(
        "zzzz/deep/other".to_string(),
        CommitId("other-tip".into()),
    ));

    let mut state = view_state_with_active_ready_repo(repo_id);
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    state.repos[0].head_branch = Loadable::Ready(active_name.to_string());
    state.repos[0].branches = Loadable::Ready(Arc::new(branches));
    state.repos[0].branches_rev = 1;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    cx.update(|_window, app| {
        sidebar_pane.update(app, |pane, _cx| {
            pane.set_branch_filter_query_for_test("does-not-match-head");
            pane.set_collapsed_keys_for_test(&[
                branch_sidebar::local_section_storage_key(),
                "group:local:zzzz",
                "group:local:zzzz/deep",
                "group:local:release",
            ]);
        });
    });
    test_support::redraw(cx);

    let button_center = cx
        .debug_bounds("sidebar_locate_active_branch")
        .expect("expected the Branches locate action")
        .center();
    cx.simulate_mouse_move(button_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some(format!(
            "Show and select the active local branch: {active_name}"
        ))
    );

    click_debug_selector(cx, "sidebar_locate_active_branch");

    let target_ix = cx.update(|_window, app| {
        sidebar_pane.update(app, |pane, _cx| {
            assert!(pane.branch_filter_query.is_empty());
            assert_eq!(
                pane.selected_branch(),
                Some(&SelectedBranch {
                    repo_id,
                    target: BranchMenuTarget::local(active_name),
                })
            );

            let collapsed = pane.collapsed_items_for_test();
            for expanded in [
                branch_sidebar::local_section_storage_key(),
                "group:local:zzzz",
                "group:local:zzzz/deep",
            ] {
                assert!(!collapsed.contains(expanded), "{expanded} stayed collapsed");
            }
            assert!(
                collapsed.contains("group:local:release"),
                "unrelated groups should retain their state"
            );

            let presentation = pane
                .branch_sidebar_presentation_cached()
                .expect("expected the expanded branch presentation");
            presentation
                .rows
                .iter()
                .rposition(|row| {
                    matches!(
                        row,
                        BranchSidebarRow::Branch {
                            name,
                            section: BranchSection::Local,
                            ..
                        } if name.as_ref() == active_name
                    )
                })
                .expect("expected the active branch row after expansion")
        })
    });
    test_support::redraw(cx);
    let target_selector: &'static str =
        Box::leak(format!("branch_row_{}_{}", repo_id.0, target_ix).into_boxed_str());
    assert!(
        cx.debug_bounds(target_selector).is_some(),
        "the locate action should scroll the distant active branch row into the rendered viewport"
    );

    // Drawing the programmatic scroll starts the branch scrollbar's auto-hide
    // task. Remove that scrollbar from the element tree so its state drops and
    // cancels the task on this test's thread, before another GPUI test installs
    // a different test scheduler.
    let mut teardown_state = store.snapshot().as_ref().clone();
    teardown_state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    store.replace_snapshot_for_test(Arc::new(teardown_state));
    sync_view_snapshot(cx, &view);
    cx.run_until_parked();
}

/// The two sidebar lists swap in place, so a row in one must be exactly as tall
/// as a row in the other -- at either density.
#[gpui::test]
fn the_file_explorer_and_the_branch_tree_share_one_row_height(cx: &mut gpui::TestAppContext) {
    use crate::appearance::{Appearance, UiDensity};
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let base = {
        let mut state = view_state_with_active_ready_repo(RepoId(1));
        state.repos[0].head_branch = Loadable::Ready("main".to_string());
        state.repos[0].branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Branch {
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        }]));
        state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![FileEntry {
            name: "a.rs".to_string(),
            path: Arc::new(PathBuf::from("a.rs")),
            kind: FileEntryKind::File,
            depth: 0,
            ignored: false,
        }]));
        state.repos[0].file_browser.bump_rev();
        state
    };

    let mut height_of = |mode, selectors: &[&'static str], density| {
        cx.update(|_window, app| {
            app.set_global(Appearance {
                density,
                ..Appearance::default()
            });
        });
        let mut state = base.clone();
        state.sidebar_mode = mode;
        store.replace_snapshot_for_test(Arc::new(state));
        sync_view_snapshot(cx, &view);
        cx.update(|_window, app| {
            view.update(app, |this, cx| this.notify_font_preferences_changed(cx));
        });
        cx.run_until_parked();
        selectors
            .iter()
            .find_map(|selector| cx.debug_bounds(selector))
            .unwrap_or_else(|| panic!("missing {selectors:?} in {mode:?} at {density:?}"))
            .size
            .height
    };

    for density in UiDensity::ALL {
        let file_row = height_of(
            gitcomet_state::model::SidebarMode::Files,
            &["file_browser_row_0"],
            density,
        );
        // The branch may sit under a section header, so it is not always row zero.
        let branch_row = height_of(
            gitcomet_state::model::SidebarMode::Branches,
            &["branch_row_1_0", "branch_row_1_1", "branch_row_1_2"],
            density,
        );

        assert_eq!(
            file_row, branch_row,
            "a file row and a branch row must match at {density:?} density"
        );
    }
}

#[gpui::test]
fn file_explorer_pins_and_marks_files_with_unsaved_editor_buffers(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        ["a.rs", "b.rs", "c.rs"]
            .into_iter()
            .map(|name| FileEntry {
                name: name.to_string(),
                path: Arc::new(PathBuf::from(name)),
                kind: FileEntryKind::File,
                depth: 0,
                ignored: false,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    assert!(
        cx.debug_bounds("file_browser_unsaved_header").is_none(),
        "with nothing unsaved the section must take no space at all"
    );

    // Stash a dirty buffer for `b.rs` -- the case the section exists for, since
    // a file edited and navigated away from is the one hardest to find again.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_stash.insert(
                    pane.document_identity(RepoId(1), std::path::Path::new("b.rs"))
                        .unwrap(),
                    crate::view::panes::main::StashedFileEdit {
                        text: SharedString::from("edited\n"),
                        text_format: None,
                        source_text_format: None,
                        cursor: 0,
                        text_fingerprint: 1,
                        saved_fingerprint: 2,
                        first_dirty_line: Some(0),
                        disk: Default::default(),
                    },
                );
                pane.sync_unsaved_file_edits_rev(cx);
            });
        });
    });
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("file_browser_unsaved_header").is_some(),
        "an unsaved buffer must pin a section at the top of the explorer"
    );
    let pinned = cx
        .debug_bounds("file_browser_unsaved_1")
        .expect("the unsaved file gets a pinned row");
    assert!(
        cx.debug_bounds("file_browser_unsaved_discard_1").is_some(),
        "the pinned row carries its own discard control"
    );
    // Row 0 is the header and row 1 the file, so the tree starts at row 2: the
    // pinned rows sit above the tree rather than replacing it.
    let first_tree_row = cx
        .debug_bounds("file_browser_row_2")
        .expect("the tree is still listed below the pinned section");
    assert!(
        pinned.top() < first_tree_row.top(),
        "pinned rows come first: pinned at {:?}, tree at {:?}",
        pinned.top(),
        first_tree_row.top()
    );

    // Discarding through the same entry point the row's button uses clears it.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.discard_file_edits_for(RepoId(1), &PathBuf::from("b.rs"), cx);
            });
        });
    });
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("file_browser_unsaved_header").is_none(),
        "discarding the last unsaved buffer removes the section again"
    );
    assert!(
        cx.debug_bounds("file_browser_row_0").is_some(),
        "and the tree closes back up to the top"
    );
}

/// Folder rows carried no context-menu invoker at all until this menu existed,
/// so the right-click handler had nothing to light up and was simply never
/// attached. This drives the real row to catch a regression back to that.
#[gpui::test]
fn right_clicking_a_folder_row_opens_the_folder_context_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: FileEntryKind::Directory,
            depth: 0,
            ignored: false,
        },
        FileEntry {
            name: "a.rs".to_string(),
            path: Arc::new(PathBuf::from("a.rs")),
            kind: FileEntryKind::File,
            depth: 0,
            ignored: false,
        },
    ]));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let folder_row = cx
        .debug_bounds("file_browser_row_0")
        .expect("the folder is the first tree row");
    let center = folder_row.center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("app_popover").is_some(),
        "right-clicking a folder must open a context menu"
    );
    // A folder-only entry: proof this is the folder menu rather than the file
    // menu firing on the wrong row.
    assert!(
        cx.debug_bounds("context_menu_expand_all_under_here")
            .is_some(),
        "expected the folder menu's recursive expand entry"
    );
    assert!(
        cx.debug_bounds("context_menu_copy_absolute_path").is_some(),
        "expected the folder menu's copy entries"
    );
    // The folder row is the only row that pairs a state-mutating `on_click`
    // with a right-button handler, so opening the menu must not also toggle it
    // — otherwise every right-click would collapse the folder under the menu.
    assert!(
        store
            .snapshot()
            .repos
            .iter()
            .all(|repo| repo.file_browser.expanded_dirs.is_empty()),
        "right-clicking a folder must not toggle it"
    );
}

/// Clicking a file the editor is holding unsaved text for must land back in the
/// editor, not in the read-only view -- which would show the copy on disk and
/// look like the edits were lost.
#[gpui::test]
fn clicking_a_file_with_unsaved_edits_opens_the_editor(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        ["a.rs", "b.rs"]
            .into_iter()
            .map(|name| FileEntry {
                name: name.to_string(),
                path: Arc::new(PathBuf::from(name)),
                kind: FileEntryKind::File,
                depth: 0,
                ignored: false,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    // A clean tree: clicking a file opens the read-only content view.
    click_debug_selector(cx, "file_browser_row_0");
    pump_until(cx, "file content selection", |_| {
        store.snapshot().repos[0].diff_state.content_preview
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        let repo = &view.read(app).state.repos[0];
        assert!(
            repo.diff_state.content_preview && !repo.diff_state.edit_mode,
            "a file with nothing unsaved opens read-only"
        );
    });

    // Now give `b.rs` an unsaved buffer and click it in the tree.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_stash.insert(
                    pane.document_identity(RepoId(1), std::path::Path::new("b.rs"))
                        .unwrap(),
                    crate::view::panes::main::StashedFileEdit {
                        text: SharedString::from("edited\n"),
                        text_format: None,
                        source_text_format: None,
                        cursor: 0,
                        text_fingerprint: 1,
                        saved_fingerprint: 2,
                        first_dirty_line: Some(0),
                        disk: Default::default(),
                    },
                );
                pane.sync_unsaved_file_edits_rev(cx);
            });
        });
    });
    test_support::redraw(cx);

    // Rows 0 and 1 are now the pinned section, so `b.rs` sits at tree row 3.
    click_debug_selector(cx, "file_browser_row_3");
    pump_until(cx, "unsaved file editor selection", |_| {
        store.snapshot().repos[0].diff_state.edit_mode
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    cx.update(|_window, app| {
        let repo = &view.read(app).state.repos[0];
        assert!(
            repo.diff_state.edit_mode,
            "a file with unsaved edits opens straight into the editor"
        );
    });

    // And the pinned row itself does the same, from a read-only starting point.
    cx.update(|_window, app| {
        let mut state = (*view.read(app).state).clone();
        state.repos[0].diff_state.edit_mode = false;
        state.repos[0].diff_state.content_preview = true;
        store.replace_snapshot_for_test(Arc::new(state));
    });
    sync_view_snapshot(cx, &view);
    click_debug_selector(cx, "file_browser_unsaved_1");
    pump_until(cx, "pinned file editor selection", |_| {
        store.snapshot().repos[0].diff_state.edit_mode
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    cx.update(|_window, app| {
        assert!(
            view.read(app).state.repos[0].diff_state.edit_mode,
            "the pinned row opens the editor too"
        );
    });
}

#[gpui::test]
fn sidebar_worktree_badges_share_one_right_edge_near_the_pane_edge(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    let branch = |name: &str| gitcomet_core::domain::Branch {
        name: name.to_string(),
        target: CommitId("1111111111111111".into()),
        upstream: None,
        divergence: None,
    };
    let worktree = |path: &str, branch: &str| gitcomet_core::domain::Worktree {
        path: PathBuf::from(path),
        head: None,
        branch: Some(branch.to_string()),
        detached: false,
    };
    // Names and badge labels of deliberately different widths: the badges are
    // pushed against the trailing edge, so none of that may reach their right
    // edge.
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![
        branch("alpha"),
        branch("beta"),
        branch("gamma-with-a-much-longer-name"),
    ]));
    state.repos[0].branches_rev = 1;
    state.repos[0].worktrees = Loadable::Ready(Arc::new(vec![
        worktree("/tmp/wt-alpha", "alpha"),
        worktree("/tmp/wt-beta-considerably-longer", "beta"),
        worktree("/tmp/g", "gamma-with-a-much-longer-name"),
    ]));
    state.repos[0].worktrees_rev = 1;
    state.repos[0].branch_sidebar_rev = 1;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let sidebar = cx
        .debug_bounds("sidebar_pane")
        .expect("expected the sidebar pane");
    let badges: Vec<_> = (0..12usize)
        .filter_map(|ix| {
            let selector: &'static str =
                Box::leak(format!("branch_worktree_badge_{ix}").into_boxed_str());
            cx.debug_bounds(selector)
        })
        .collect();
    assert!(
        badges.len() >= 3,
        "expected a worktree badge on each branch that has one, got {}",
        badges.len()
    );

    let first_right = badges[0].right();
    for badge in &badges {
        assert_eq!(
            badge.right(),
            first_right,
            "worktree badges must share one right edge regardless of label width"
        );
    }

    // What is left between the badges and the pane edge is the reserved `⋮`
    // slot, the gap before it, and the row-highlight inset — nothing else.
    let trailing_gap = sidebar.right() - first_right;
    assert!(
        trailing_gap <= px(30.0),
        "worktree badges should sit close to the pane's right edge, got {trailing_gap:?}"
    );
}

/// Branch group rows carried no context-menu invoker and no right-click handler
/// at all until this menu existed. This drives the real row to catch a
/// regression back to that.
#[gpui::test]
fn right_clicking_a_branch_group_row_opens_the_group_context_menu(cx: &mut gpui::TestAppContext) {
    // Measures Compact layout; a fresh session now defaults to Comfortable.
    cx.update(crate::appearance::pin_compact_for_test);
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    let branch = |name: &str| gitcomet_core::domain::Branch {
        name: name.to_string(),
        target: gitcomet_core::domain::CommitId("aaaaaaaaaaaa".into()),
        upstream: None,
        divergence: None,
    };
    state.repos[0].head_branch = Loadable::Ready("main".to_string());
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![
        branch("main"),
        branch("feat/a"),
        branch("feat/b"),
    ]));
    state.repos[0].branches_rev = 1;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let group_row = cx
        .debug_bounds("branch_group_0")
        .or_else(|| cx.debug_bounds("branch_group_1"))
        .or_else(|| cx.debug_bounds("branch_group_2"))
        .expect("the feat/ group renders a row");
    assert_eq!(
        group_row.size.height,
        cx.update(|_window, app| {
            let root = view.read(app);
            rows::sidebar::sidebar_list_row_height(root.theme, root.ui_scale_percent)
        }),
        "branch hierarchy rows must follow the selected density"
    );
    let center = group_row.center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("app_popover").is_some(),
        "right-clicking a branch group must open a context menu"
    );
    // A group-only entry: proof this is the group menu rather than the section
    // or branch menu firing on the wrong row.
    assert!(
        cx.debug_bounds("context_menu_expand_all_under_here")
            .is_some(),
        "expected the group menu's recursive expand entry"
    );

    // The group row pairs a collapse-toggling `on_click` with the new
    // right-button handler, so opening the menu must not also collapse the
    // group under it.
    let collapsed_after = cx.update(|_window, app| {
        view.read(app)
            .sidebar_pane
            .read(app)
            .collapsed_items_for_test()
    });
    assert!(
        collapsed_after.is_empty(),
        "right-clicking a branch group must not toggle it, got {collapsed_after:?}"
    );
}

#[gpui::test]
fn native_file_drop_moves_before_acknowledgement_and_can_be_undone(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("repository");
    let destination = repository.join("target");
    std::fs::create_dir_all(&destination).unwrap();
    let source = directory.path().join("desktop.txt");
    std::fs::write(&source, b"saved desktop contents").unwrap();
    let target = destination.join("desktop.txt");
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].spec.workdir = repository;
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![FileEntry {
        name: "target".into(),
        path: Arc::new(PathBuf::from("target")),
        kind: FileEntryKind::Directory,
        depth: 0,
        ignored: false,
    }]));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    let position = cx.debug_bounds("file_browser_row_0").unwrap().center();
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Entered {
            position,
            paths: gpui::ExternalPaths([source.clone()].into_iter().collect()),
        },
    );
    let completions = Arc::new(Mutex::new(Vec::new()));
    let completed = completions.clone();
    let old = source.clone();
    let new = target.clone();
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::SubmitWithTransfer {
            position,
            transfer: gpui::FileDropTransfer {
                operation: gpui::FileTransferOperation::Move,
                // Native URI-list file managers let the receiving application
                // move files, even though the wire protocol has a Move result.
                source_owns_move: false,
                completion: gpui::FilePaste::new(move |operation| {
                    assert!(!old.exists(), "acknowledge only after source removal");
                    assert_eq!(std::fs::read(&new).unwrap(), b"saved desktop contents");
                    completed.lock().unwrap().push(operation);
                }),
            },
        },
    );
    pump_until(cx, "native move result", |_| {
        !store.snapshot().filesystem.completed.is_empty()
    });
    // Deterministic UI tests apply store snapshots explicitly; the live app's
    // store poller normally delivers the result that releases the native lease.
    sync_view_snapshot(cx, &view);
    assert_eq!(
        *completions.lock().unwrap(),
        [Some(gpui::FileTransferOperation::Move)]
    );
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.submit_filesystem_operation(
                gitcomet_core::filesystem::Request::new(gitcomet_core::filesystem::Operation::Undo),
                None,
                window,
                cx,
            );
        });
    });
    pump_until(cx, "undo native move", |_| {
        source.exists() && !target.exists()
    });
    assert_eq!(std::fs::read(source).unwrap(), b"saved desktop contents");
}

#[gpui::test]
fn explorer_selection_keyboard_cut_and_document_navigation_are_focus_scoped(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(directory.path().join(name), "saved").unwrap();
    }
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].spec.workdir = directory.path().to_path_buf();
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        ["a.txt", "b.txt", "c.txt"]
            .into_iter()
            .map(|name| FileEntry {
                name: name.into(),
                path: Arc::new(PathBuf::from(name)),
                kind: FileEntryKind::File,
                depth: 0,
                ignored: false,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    let click = |cx: &mut gpui::VisualTestContext, index: usize, modifiers, button| {
        let point = cx
            .debug_bounds(
                [
                    "file_browser_row_0",
                    "file_browser_row_1",
                    "file_browser_row_2",
                ][index],
            )
            .unwrap()
            .center();
        cx.simulate_mouse_down(point, button, modifiers);
        cx.simulate_mouse_up(point, button, modifiers);
    };
    let primary = if cfg!(target_os = "macos") {
        gpui::Modifiers {
            platform: true,
            ..Default::default()
        }
    } else {
        gpui::Modifiers {
            control: true,
            ..Default::default()
        }
    };
    click(cx, 0, primary, gpui::MouseButton::Left);
    pump_until(cx, "first selection", |_| {
        store.snapshot().repos[0].file_browser.selection.paths.len() == 1
    });
    sync_view_snapshot(cx, &view);
    click(cx, 2, primary, gpui::MouseButton::Left);
    pump_until(cx, "toggle selection", |_| {
        store.snapshot().repos[0].file_browser.selection.paths.len() == 2
    });
    sync_view_snapshot(cx, &view);
    click(
        cx,
        1,
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        gpui::MouseButton::Left,
    );
    pump_until(cx, "range selection", |_| {
        store.snapshot().repos[0]
            .file_browser
            .selection
            .paths
            .contains(Path::new("b.txt"))
    });
    sync_view_snapshot(cx, &view);
    let selected = store.snapshot().repos[0]
        .file_browser
        .selection
        .paths
        .clone();
    assert_eq!(
        selected,
        [PathBuf::from("b.txt"), PathBuf::from("c.txt")]
            .into_iter()
            .collect()
    );
    click(cx, 2, gpui::Modifiers::default(), gpui::MouseButton::Right);
    test_support::redraw(cx);
    assert_eq!(
        store.snapshot().repos[0].file_browser.selection.paths,
        selected
    );
    cx.simulate_keystrokes("escape");
    sync_view_snapshot(cx, &view);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-home"
    } else {
        "ctrl-home"
    });
    pump_until(cx, "focused row", |_| {
        store.snapshot().repos[0]
            .file_browser
            .selection
            .focused
            .as_deref()
            == Some(Path::new("a.txt"))
    });
    assert_eq!(
        store.snapshot().repos[0].file_browser.selection.paths,
        selected
    );
    sync_view_snapshot(cx, &view);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    });
    pump_until(cx, "select all", |_| {
        store.snapshot().repos[0].file_browser.selection.paths.len() == 3
    });
    sync_view_snapshot(cx, &view);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-x"
    } else {
        "ctrl-x"
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.update(app, |_, cx| {
            let files = crate::clipboard::read_files(cx).unwrap();
            assert_eq!(
                files.intent,
                gitcomet_core::filesystem::TransferIntent::Move
            );
            assert_eq!(files.paths.len(), 3);
        })
    });
    cx.simulate_keystrokes("escape");
    cx.update(|_, app| {
        view.update(app, |_, cx| {
            assert_eq!(
                crate::clipboard::read_files(cx).unwrap().intent,
                gitcomet_core::filesystem::TransferIntent::Copy
            )
        })
    });
    // Repository navigation must restore the canvas even when its tab was
    // already active underneath Documents.
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.documents_active = true;
            assert!(view.activate_repo_path(directory.path(), cx));
            assert!(!view.documents_active);
        })
    });
    sync_view_snapshot(cx, &view);
    click(cx, 0, gpui::Modifiers::default(), gpui::MouseButton::Left);
    cx.update(|_, app| assert!(!view.read(app).documents_active));
    cx.update(|_, app| {
        view.update(app, |_, cx| {
            crate::clipboard::write_text(
                cx,
                String::new(),
                crate::clipboard::CopySource::ContextMenu,
            )
        })
    });
}

#[gpui::test]
fn explorer_ctrl_x_after_a_plain_click_cuts_the_clicked_file(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    // The production keymap, including the TextInput chords a stray focus would hit.
    install_app_shortcuts_for_test(cx, backend);
    cx.update(|_, app| crate::app::bind_text_input_keys_for_test(app));
    let directory = tempfile::tempdir().unwrap();
    // Backend workdirs are canonical; Windows cuts publish canonical paths.
    let workdir = canonicalize_or_original(directory.path().to_path_buf());
    for name in ["a.txt", "b.txt"] {
        std::fs::write(workdir.join(name), "saved").unwrap();
    }
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].spec.workdir = workdir.clone();
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        ["a.txt", "b.txt"]
            .into_iter()
            .map(|name| FileEntry {
                name: name.into(),
                path: Arc::new(PathBuf::from(name)),
                kind: FileEntryKind::File,
                depth: 0,
                ignored: false,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    let entries = state.repos[0].file_browser.entries.clone();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    // Opening the file reloads the listing, which TestBackend leaves empty.
    let reseed = |cx: &mut gpui::VisualTestContext| {
        let mut state = (*store.snapshot()).clone();
        state.repos[0].file_browser.entries = entries.clone();
        state.repos[0].file_browser.bump_rev();
        store.replace_snapshot_for_test(Arc::new(state));
        sync_view_snapshot(cx, &view);
    };
    let cut = if cfg!(target_os = "macos") {
        "cmd-x"
    } else {
        "ctrl-x"
    };
    // The second round clicks the row that the first one opened and selected.
    for round in 0..2 {
        let row = cx.debug_bounds("file_browser_row_0").unwrap().center();
        cx.simulate_mouse_down(row, gpui::MouseButton::Left, Default::default());
        cx.simulate_mouse_up(row, gpui::MouseButton::Left, Default::default());
        pump_until(cx, "file selection", |_| {
            store.snapshot().repos[0]
                .file_browser
                .selection
                .paths
                .contains(Path::new("a.txt"))
        });
        cx.run_until_parked();
        reseed(cx);
        cx.update(|window, app| {
            let focused = window.focused(app).is_some();
            assert!(
                view.read(app).sidebar_pane.read(app).explorer_has_focus_for_test(window),
                "round {round}: the clicked tree must keep the keyboard (anything focused: {focused})"
            );
        });
        cx.simulate_keystrokes(cut);
        cx.run_until_parked();
        cx.update(|_, app| {
            view.update(app, |_, cx| {
                let files =
                    crate::clipboard::read_files(cx).expect("the cut reached the clipboard");
                assert_eq!(
                    files.intent,
                    gitcomet_core::filesystem::TransferIntent::Move
                );
                assert_eq!(files.paths, vec![workdir.join("a.txt")]);
            })
        });
        test_support::redraw(cx);
        assert!(
            cx.debug_bounds("explorer_cut_marker_0").is_some(),
            "round {round}: the cut row shows its marker"
        );
        // Escape cancels the cut before the next round.
        cx.simulate_keystrokes("escape");
        sync_view_snapshot(cx, &view);
    }
}

#[gpui::test]
fn explorer_chevron_click_keeps_the_selection_and_keyboard_paste_uses_focused_folder(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("source.txt"), "clipboard content").unwrap();
    std::fs::create_dir(directory.path().join("destination")).unwrap();
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].spec.workdir = directory.path().to_path_buf();
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        FileEntry {
            name: "destination".into(),
            path: Arc::new("destination".into()),
            kind: FileEntryKind::Directory,
            depth: 0,
            ignored: false,
        },
        FileEntry {
            name: "source.txt".into(),
            path: Arc::new("source.txt".into()),
            kind: FileEntryKind::File,
            depth: 0,
            ignored: false,
        },
    ]));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    let primary = gpui::Modifiers {
        control: !cfg!(target_os = "macos"),
        platform: cfg!(target_os = "macos"),
        ..Default::default()
    };
    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str, modifiers| {
        let position = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_mouse_down(position, gpui::MouseButton::Left, modifiers);
        cx.simulate_mouse_up(position, gpui::MouseButton::Left, modifiers);
    };
    click(cx, "file_browser_row_1", primary);
    pump_until(cx, "file selection", |_| {
        store.snapshot().repos[0]
            .file_browser
            .selection
            .paths
            .contains(Path::new("source.txt"))
    });
    sync_view_snapshot(cx, &view);
    let selected = store.snapshot().repos[0]
        .file_browser
        .selection
        .paths
        .clone();
    click(cx, "explorer_chevron_0", Default::default());
    pump_until(cx, "folder expanded", |_| {
        store.snapshot().repos[0]
            .file_browser
            .expanded_dirs
            .contains(&PathBuf::from("destination"))
    });
    sync_view_snapshot(cx, &view);
    assert_eq!(
        store.snapshot().repos[0].file_browser.selection.paths,
        selected
    );
    assert_eq!(
        store.snapshot().repos[0]
            .file_browser
            .selection
            .focused
            .as_deref(),
        Some(Path::new("destination"))
    );
    // A modified click adds the folder to the selection without toggling it.
    click(cx, "file_browser_row_0", primary);
    pump_until(cx, "folder selected", |_| {
        store.snapshot().repos[0].file_browser.selection.paths.len() == 2
    });
    assert!(
        store.snapshot().repos[0]
            .file_browser
            .expanded_dirs
            .contains(&PathBuf::from("destination"))
    );
    sync_view_snapshot(cx, &view);
    click(cx, "file_browser_row_0", primary);
    pump_until(cx, "folder deselected", |_| {
        store.snapshot().repos[0].file_browser.selection.paths == selected
    });
    sync_view_snapshot(cx, &view);
    click(cx, "explorer_chevron_0", Default::default());
    pump_until(cx, "folder collapsed", |_| {
        !store.snapshot().repos[0]
            .file_browser
            .expanded_dirs
            .contains(&PathBuf::from("destination"))
    });
    sync_view_snapshot(cx, &view);
    // Focus remains on the destination. Copy must still use the file selection.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    cx.update(|_, app| {
        assert_eq!(
            view.update(app, |_, cx| crate::clipboard::read_files(cx).unwrap().paths),
            vec![directory.path().join("source.txt")]
        );
    });
    // Paste immediately after the chevron click, before the model can publish
    // its new focus. Keyboard routing must honor the click we just handled.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-end"
    } else {
        "ctrl-end"
    });
    pump_until(cx, "source focused", |_| {
        store.snapshot().repos[0]
            .file_browser
            .selection
            .focused
            .as_deref()
            == Some(Path::new("source.txt"))
    });
    sync_view_snapshot(cx, &view);
    click(cx, "explorer_chevron_0", Default::default());
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    });
    pump_until(cx, "file pasted into focused folder", |_| {
        directory.path().join("destination/source.txt").exists()
    });
    assert_eq!(
        std::fs::read_to_string(directory.path().join("destination/source.txt")).unwrap(),
        "clipboard content"
    );
    assert!(directory.path().join("source.txt").exists());
    cx.update(|_, app| {
        assert_eq!(
            view.update(app, |_, cx| crate::clipboard::read_files(cx).unwrap().paths),
            vec![directory.path().join("source.txt")],
            "copy remains available for repeated paste"
        );
    });
    // Native clipboard content from another process follows the same path.
    let external = tempfile::tempdir().unwrap();
    std::fs::write(external.path().join("external.txt"), "external clipboard").unwrap();
    cx.update(|_, app| {
        app.write_to_clipboard(gpui::ClipboardItem {
            entries: vec![gpui::ClipboardEntry::Files(gpui::FileTransfer {
                paths: gpui::ExternalPaths(
                    [external.path().join("external.txt")].into_iter().collect(),
                ),
                operation: gpui::FileTransferOperation::Copy,
                ownership: 0,
            })],
        })
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    });
    pump_until(cx, "native clipboard file pasted", |_| {
        directory.path().join("destination/external.txt").exists()
    });
    assert!(external.path().join("external.txt").exists());
}

#[gpui::test]
fn explorer_folder_row_click_selects_focuses_and_toggles(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("source.txt"), "clipboard content").unwrap();
    std::fs::create_dir(directory.path().join("destination")).unwrap();
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].spec.workdir = directory.path().to_path_buf();
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        FileEntry {
            name: "destination".into(),
            path: Arc::new("destination".into()),
            kind: FileEntryKind::Directory,
            depth: 0,
            ignored: false,
        },
        FileEntry {
            name: "source.txt".into(),
            path: Arc::new("source.txt".into()),
            kind: FileEntryKind::File,
            depth: 0,
            ignored: false,
        },
    ]));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    let primary = gpui::Modifiers {
        control: !cfg!(target_os = "macos"),
        platform: cfg!(target_os = "macos"),
        ..Default::default()
    };
    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str, modifiers| {
        let position = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_mouse_down(position, gpui::MouseButton::Left, modifiers);
        cx.simulate_mouse_up(position, gpui::MouseButton::Left, modifiers);
    };
    let expanded = |store: &AppStore| {
        store.snapshot().repos[0]
            .file_browser
            .expanded_dirs
            .contains(&PathBuf::from("destination"))
    };
    let only_destination: std::collections::BTreeSet<PathBuf> =
        [PathBuf::from("destination")].into_iter().collect();

    click(cx, "file_browser_row_1", primary);
    pump_until(cx, "file selected", |_| {
        store.snapshot().repos[0].file_browser.selection.paths.len() == 1
    });
    sync_view_snapshot(cx, &view);
    // A plain row click replaces the selection, focuses the folder and opens it.
    click(cx, "file_browser_row_0", Default::default());
    pump_until(cx, "folder expanded", |_| expanded(&store));
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.repos[0].file_browser.selection.paths,
        only_destination
    );
    assert_eq!(
        snapshot.repos[0].file_browser.selection.focused.as_deref(),
        Some(Path::new("destination"))
    );
    sync_view_snapshot(cx, &view);
    // A second click closes it and keeps it selected.
    click(cx, "file_browser_row_0", Default::default());
    pump_until(cx, "folder collapsed", |_| !expanded(&store));
    assert_eq!(
        store.snapshot().repos[0].file_browser.selection.paths,
        only_destination
    );
    sync_view_snapshot(cx, &view);
    // A modified click extends the selection without toggling.
    click(cx, "file_browser_row_1", primary);
    pump_until(cx, "selection extended", |_| {
        store.snapshot().repos[0].file_browser.selection.paths.len() == 2
    });
    assert!(
        !expanded(&store),
        "a modified click must not toggle the folder"
    );
    sync_view_snapshot(cx, &view);
    // The clipboard is independent of the selection the click replaced.
    click(cx, "file_browser_row_1", Default::default());
    pump_until(cx, "file selected alone", |_| {
        store.snapshot().repos[0].file_browser.selection.paths
            == [PathBuf::from("source.txt")]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
    });
    sync_view_snapshot(cx, &view);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    click(cx, "file_browser_row_0", Default::default());
    pump_until(cx, "folder selected", |_| {
        store.snapshot().repos[0].file_browser.selection.paths == only_destination
    });
    sync_view_snapshot(cx, &view);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    });
    pump_until(cx, "file pasted into the clicked folder", |_| {
        directory.path().join("destination/source.txt").exists()
    });
}

#[gpui::test]
fn explorer_external_drop_writes_to_highlighted_folder_in_sidebar_and_popup(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("destination")).unwrap();
    std::fs::write(directory.path().join("destination/keep.txt"), "keep").unwrap();
    for popup in [false, true] {
        let name = if popup { "popup.txt" } else { "sidebar.txt" };
        let source = external.path().join(name);
        std::fs::write(&source, "dropped content").unwrap();
        let mut state = view_state_with_active_ready_repo(RepoId(1));
        state.repos[0].spec.workdir = directory.path().to_path_buf();
        state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
        state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
            FileEntry {
                name: "destination".into(),
                path: Arc::new("destination".into()),
                kind: FileEntryKind::Directory,
                depth: 0,
                ignored: false,
            },
            FileEntry {
                name: "keep.txt".into(),
                path: Arc::new("destination/keep.txt".into()),
                kind: FileEntryKind::File,
                depth: 1,
                ignored: false,
            },
        ]));
        state.repos[0]
            .file_browser
            .expanded_dirs
            .insert(Arc::new(PathBuf::from("destination")));
        state.repos[0].file_browser.bump_rev();
        store.replace_snapshot_for_test(Arc::new(state));
        sync_view_snapshot(cx, &view);
        if popup {
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.set_sidebar_collapsed(true, cx);
                    view.open_sidebar_collapsed_popover(panes::CollapsedSidebarSection::Files, cx);
                })
            });
            for _ in 0..25 {
                cx.executor().advance_clock(Duration::from_millis(16));
                cx.run_until_parked();
                test_support::redraw(cx);
            }
        }
        let position = cx.debug_bounds("file_browser_row_1").unwrap().center();
        dispatch_file_drop(
            cx,
            gpui::FileDropEvent::Entered {
                position,
                paths: gpui::ExternalPaths([source.clone()].into_iter().collect()),
            },
        );
        dispatch_file_drop(cx, gpui::FileDropEvent::Pending { position });
        dispatch_file_drop(cx, gpui::FileDropEvent::Submit { position });
        pump_until(cx, "drop into highlighted destination", |_| {
            directory.path().join("destination").join(name).exists()
        });
        assert_eq!(
            std::fs::read_to_string(directory.path().join("destination").join(name)).unwrap(),
            "dropped content"
        );
        assert!(source.exists(), "external drop copies by default");
    }
}

#[test]
fn reconciliation_releases_vanished_selection_but_preserves_intentional_empty_selection() {
    let status = RepoStatus {
        staged: Default::default(),
        unstaged: Default::default(),
    };
    for use_repo in [false, true] {
        let mut selection = StatusMultiSelection {
            explicit_section: Some(StatusSection::Staged),
            staged: vec!["gone.txt".into()],
            ..Default::default()
        };
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::new(),
            },
        );
        repo.worktree_status = Loadable::Ready(Arc::clone(&status.unstaged));
        repo.staged_status = Loadable::Ready(Arc::clone(&status.staged));
        if use_repo {
            reconcile_status_multi_selection_with_repo(&mut selection, &repo);
        } else {
            reconcile_status_multi_selection(&mut selection, &status);
        }
        assert!(selection.is_empty());
        assert_eq!(selection.explicit_section, None);
        selection.explicit_section = Some(StatusSection::Staged);
        if use_repo {
            reconcile_status_multi_selection_with_repo(&mut selection, &repo);
        } else {
            reconcile_status_multi_selection(&mut selection, &status);
        }
        assert_eq!(selection.explicit_section, Some(StatusSection::Staged));
    }
}
