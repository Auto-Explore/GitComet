#[test]
fn group_paths_at_remote_root_collects_all_directories_without_an_empty_key() {
    let paths = group_paths_at_or_below("", ["main", "feat/a", "feat/b/c", "fix/d"].into_iter());
    assert_eq!(
        paths,
        BTreeSet::from(["feat".into(), "feat/b".into(), "fix".into()])
    );
}

#[test]
fn pinned_group_trees_preserve_identity_metadata_and_nested_collapse() {
    let mut repo = populated_repo();
    let branch = |name: &str| Branch {
        name: name.into(),
        target: commit_id("a"),
        upstream: None,
        divergence: Some(UpstreamDivergence {
            ahead: 3,
            behind: 2,
        }),
    };
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main"),
        branch("feat/a"),
        branch("feat/nested/b"),
        branch("features/x"),
    ]));
    repo.head_branch = Loadable::Ready("feat/a".into());
    let remote = |remote: &str, name: &str| RemoteBranch {
        remote: remote.into(),
        name: name.into(),
        target: commit_id("b"),
    };
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        remote("team/origin", "feat/a"),
        remote("team/origin", "feat/nested/b"),
        remote("team", "origin/feat/wrong-remote"),
        remote("team/origin", "features/x"),
    ]));
    let local_key = local_group_storage_key("feat");
    let remote_key = remote_group_storage_key("team/origin", "feat");
    let pins = BTreeSet::from([
        local_key.clone(),
        remote_key.clone(),
        branch_pin_storage_key(BranchSection::Local, "main"),
        local_group_storage_key("gone"),
    ]);
    let rows = expanded_pinned_rows(&repo, &pins, &BTreeSet::new());
    assert!(
        rows.iter().any(|row| matches!(row,
            BranchSidebarRow::GroupHeader { label, depth: 0, .. } if label == "team/origin/feat/"))
    );
    assert!(rows.iter().any(|row| matches!(row,
            BranchSidebarRow::Branch { name, depth: 1, is_head: true,
                divergence_ahead: Some(ahead), divergence_behind: Some(behind), .. }
                if name == "feat/a" && ahead.get() == 3 && behind.get() == 2)));
    assert!(rows.iter().any(|row| matches!(row,
            BranchSidebarRow::Branch { target, depth: 2, .. }
                if target == &BranchMenuTarget::remote("team/origin", "feat/nested/b"))));
    let names = |rows: &[BranchSidebarRow]| {
        rows.iter()
            .filter_map(|row| match row {
                BranchSidebarRow::Branch { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        names(&rows),
        BTreeSet::from([
            "main".into(),
            "feat/a".into(),
            "feat/nested/b".into(),
            "team/origin/feat/a".into(),
            "team/origin/feat/nested/b".into(),
        ])
    );

    let nested_closed = BTreeSet::from([local_group_storage_key("feat/nested")]);
    let rows = expanded_pinned_rows(&repo, &pins, &nested_closed);
    assert!(!names(&rows).contains("feat/nested/b"));
    assert!(names(&rows).contains("team/origin/feat/nested/b"));
    let roots_closed = BTreeSet::from([local_key.clone(), remote_key.clone()]);
    let rows = expanded_pinned_rows(&repo, &pins, &roots_closed);
    assert_eq!(rows.len(), 3, "closed groups keep one root row each");
    let roots = pinned_root_rows(&repo, &pins, "");
    assert_eq!(roots.len(), 3, "members are not counted as individual pins");
    for (section, key) in roots.iter().filter_map(pinned_row_storage_key) {
        assert!(pins.contains(&key));
        assert!(pinned_branch_renders(&repo, &key, section, ""));
    }
    let filtered = pinned_root_rows(&repo, &pins, "nested/B");
    assert_eq!(filtered.len(), 2);
    assert!(pinned_root_rows(&repo, &pins, "absent").is_empty());
    assert_eq!(
        expanded_pinned_rows(&repo, &pins, &BTreeSet::new()).len(),
        9
    );
}

/// A group's descendants come from the member branch names, and the
/// trailing segment of each name is the branch, not a group.
#[test]
fn group_paths_collects_the_group_and_every_directory_under_it() {
    let names = ["feat/a", "feat/b/c", "feat/b/d/e", "main"];
    let paths = group_paths_at_or_below("feat", names.into_iter());

    let expected: BTreeSet<String> = ["feat", "feat/b", "feat/b/d"]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    assert_eq!(paths, expected);
}

/// `starts_with` on the bare prefix would sweep `features/` into `feat/`.
#[test]
fn group_paths_ignores_a_sibling_sharing_a_name_prefix() {
    let paths = group_paths_at_or_below("feat", ["features/x", "feat/a"].into_iter());
    assert_eq!(
        paths,
        ["feat".to_string()].into_iter().collect::<BTreeSet<_>>()
    );
}

/// The invoked group is always present, so its own row toggles even when
/// the branch list cannot be read.
#[test]
fn group_paths_always_includes_the_invoked_group() {
    let paths = group_paths_at_or_below("feat", std::iter::empty());
    assert!(paths.contains("feat"));
}

#[test]
fn set_collapse_state_drives_a_group_key_both_ways() {
    let mut items = BTreeSet::new();
    let key = local_group_storage_key("feat");

    set_collapse_state(&mut items, &key, true);
    assert!(is_collapsed(&items, &key));
    // Idempotent: setting the state it already has must not flip it.
    set_collapse_state(&mut items, &key, true);
    assert!(is_collapsed(&items, &key));

    set_collapse_state(&mut items, &key, false);
    assert!(!is_collapsed(&items, &key));
}

/// Sections that default to collapsed store the inverted `expanded:` key, so
/// a naive insert/remove would write the wrong one.
#[test]
fn set_collapse_state_respects_the_inverted_storage_of_default_collapsed_sections() {
    let mut items = BTreeSet::new();
    let key = submodules_section_storage_key();
    assert!(
        is_collapsed(&items, key),
        "precondition: collapsed by default"
    );

    set_collapse_state(&mut items, key, false);
    assert!(!is_collapsed(&items, key));

    set_collapse_state(&mut items, key, true);
    assert!(is_collapsed(&items, key));
}

#[test]
fn branch_pin_key_round_trips_both_sections() {
    for section in [BranchSection::Local, BranchSection::Remote] {
        let key = branch_pin_storage_key(section, "feat/a");
        assert_eq!(parse_branch_pin_key(&key), Some((section, "feat/a")));
    }
    // An unknown prefix is ignored rather than mis-rendered, so a stale key
    // from an older session cannot claim a section.
    assert_eq!(parse_branch_pin_key("garbage"), None);
}

fn annex_repo() -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(7),
        RepoSpec {
            workdir: PathBuf::from("/tmp/annex"),
        },
    );
    let branch = |name: &str| Branch {
        name: name.to_string(),
        target: commit_id("aaaaaaaa"),
        upstream: None,
        divergence: None,
    };
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main"),
        branch("git-annex"),
        branch("synced/main"),
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit_id("aaaaaaaa"),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "git-annex".to_string(),
            target: commit_id("bbbbbbbb"),
        },
    ]));
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.annex.uuid = Some("u-here".into());
    support.annex.has_annex_branch = true;
    support.annex.numcopies = Some(2);
    support.annex.repositories = vec![
        gitcomet_core::large_files::AnnexRepository {
            uuid: "00000000-0000-0000-0000-000000000001".into(),
            description: "web".into(),
            remote_name: None,
            special_type: None,
            special_name: None,
            trust: gitcomet_core::large_files::AnnexTrust::Semitrusted,
            here: false,
        },
        gitcomet_core::large_files::AnnexRepository {
            uuid: "u-here".into(),
            description: "laptop".into(),
            remote_name: None,
            special_type: None,
            special_name: None,
            trust: gitcomet_core::large_files::AnnexTrust::Semitrusted,
            here: true,
        },
        gitcomet_core::large_files::AnnexRepository {
            uuid: "u-backup".into(),
            description: String::new(),
            remote_name: Some("backup".into()),
            special_type: Some("directory".into()),
            special_name: None,
            trust: gitcomet_core::large_files::AnnexTrust::Untrusted,
            here: false,
        },
    ];
    repo.large_file_support = Loadable::Ready(Arc::new(support));
    repo
}

fn branch_labels(rows: &[BranchSidebarRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn annex_section_lists_repositories_without_builtin_remotes() {
    let repo = annex_repo();
    let rows = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "");
    let header = rows.iter().find_map(|row| match row {
        BranchSidebarRow::AnnexHeader { summary, .. } => Some(summary.clone()),
        _ => None,
    });
    assert_eq!(header, Some(Some("numcopies 2".into())));
    let items: Vec<(String, String, bool, bool)> = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::AnnexRepositoryItem {
                name,
                detail,
                here,
                untrusted,
                ..
            } => Some((name.to_string(), detail.to_string(), *here, *untrusted)),
            _ => None,
        })
        .collect();
    assert_eq!(
        items,
        [
            (
                "laptop".into(),
                "this clone · semitrusted".into(),
                true,
                false
            ),
            (
                "[backup]".into(),
                "directory · untrusted".into(),
                false,
                true
            ),
        ]
    );

    let rows_for =
        |repo: &RepoState| branch_sidebar_rows(repo, &BTreeSet::new(), &BTreeSet::new(), "");
    let mut uninitialized = annex_repo();
    if let Loadable::Ready(support) = &mut uninitialized.large_file_support {
        Arc::make_mut(support).annex.uuid = None;
    }
    let rows = branch_sidebar_rows(&uninitialized, &BTreeSet::new(), &BTreeSet::new(), "");
    assert!(rows.iter().any(|row| matches!(
        row,
        BranchSidebarRow::AnnexPlaceholder { can_init: true, .. }
    )));

    let is_restage_row = |row: &BranchSidebarRow| {
        matches!(
            row,
            BranchSidebarRow::AnnexPlaceholder {
                can_restage: true,
                ..
            }
        )
    };
    assert!(!rows_for(&annex_repo()).iter().any(is_restage_row));
    let mut stale = annex_repo();
    if let Loadable::Ready(support) = &mut stale.large_file_support {
        Arc::make_mut(support).annex.restage_pending = true;
    }
    let rows = rows_for(&stale);
    assert!(
        rows.iter().any(is_restage_row),
        "interrupted get offers a refresh"
    );
    assert!(
        rows.iter()
            .any(|row| matches!(row, BranchSidebarRow::AnnexRepositoryItem { .. })),
        "repositories stay listed below the refresh row"
    );
    let (fresh_source, _) = branch_sidebar_source_fingerprint(&annex_repo(), None);
    let (stale_source, _) = branch_sidebar_source_fingerprint(&stale, None);
    assert_ne!(fresh_source, stale_source, "cached rows must not be reused");

    let mut plain = annex_repo();
    plain.large_file_support = Loadable::Ready(Arc::new(Default::default()));
    let rows = branch_sidebar_rows(&plain, &BTreeSet::new(), &BTreeSet::new(), "");
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::AnnexHeader { .. }))
    );
}

#[test]
fn review_annex_sidebar_displays_raw_description_with_remote_decoration() {
    let mut repo = annex_repo();
    let Loadable::Ready(support) = &mut repo.large_file_support else {
        panic!("support")
    };
    Arc::make_mut(support).annex.repositories[2].description = "Archive".into();
    let rows = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "");
    assert!(rows.iter().any(|row| matches!(row, BranchSidebarRow::AnnexRepositoryItem { name, .. } if name.as_ref() == "Archive [backup]")));
}

#[test]
fn hidden_annex_refs_leave_branch_lists_and_change_the_cache_source() {
    let mut repo = annex_repo();
    let shown = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "");
    let (shown_source, cached) = branch_sidebar_source_fingerprint(&repo, None);
    assert!(branch_labels(&shown).iter().any(|name| name == "git-annex"));

    repo.annex_refs_hidden = true;
    let hidden = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "");
    let labels = branch_labels(&hidden);
    assert!(
        !labels
            .iter()
            .any(|name| name == "git-annex" || name.starts_with("synced")),
        "{labels:?}"
    );
    assert!(labels.iter().any(|name| name == "main"), "{labels:?}");
    assert!(!branch_sidebar_source_matches_cached(&repo, &cached));
    let (hidden_source, _) = branch_sidebar_source_fingerprint(&repo, Some(&cached));
    assert_eq!(
        hidden_source,
        branch_sidebar_source_fingerprint(&repo, None).0
    );
    assert_ne!(
        shown_source, hidden_source,
        "cached rows must not be reused"
    );
}

#[test]
fn annex_support_invalidates_both_sidebar_cache_reuse_paths() {
    let mut repo = annex_repo();
    repo.large_file_support = Loadable::NotLoaded;
    let (mut source, mut cached) = branch_sidebar_source_fingerprint(&repo, None);
    for support in [
        Loadable::Loading,
        annex_repo().large_file_support,
        {
            let mut support = match annex_repo().large_file_support {
                Loadable::Ready(support) => (*support).clone(),
                _ => unreachable!(),
            };
            support.annex.uuid = Some("changed-uuid".into());
            support.annex.restage_pending = true;
            Loadable::Ready(Arc::new(support))
        },
        Loadable::NotLoaded,
    ] {
        repo.large_file_support = support;
        repo.branch_sidebar_rev += 1;
        assert!(!branch_sidebar_source_matches_cached(&repo, &cached));
        let (next, parts) = branch_sidebar_source_fingerprint(&repo, Some(&cached));
        assert_ne!(source, next);
        assert_eq!(next, branch_sidebar_source_fingerprint(&repo, None).0);
        assert_eq!(parts.local_hash, cached.local_hash);
        assert_eq!(parts.remote_hash, cached.remote_hash);
        assert!(branch_sidebar_source_matches_cached(&repo, &parts));
        (source, cached) = (next, parts);
    }
}

#[test]
fn annex_support_revision_invalidates_an_in_place_update() {
    let mut repo = annex_repo();
    let (old, cached) = branch_sidebar_source_fingerprint(&repo, None);
    let Loadable::Ready(support) = &mut repo.large_file_support else {
        unreachable!()
    };
    Arc::make_mut(support).annex.restage_pending = true;
    repo.large_file_support_rev += 1;
    assert!(!branch_sidebar_source_matches_cached(&repo, &cached));
    let (updated, _) = branch_sidebar_source_fingerprint(&repo, Some(&cached));
    assert_ne!(old, updated);
    assert_eq!(updated, branch_sidebar_source_fingerprint(&repo, None).0);
}

#[test]
fn checked_out_annex_bookkeeping_branch_stays_visible() {
    for head in ["git-annex", "synced/main"] {
        let mut repo = annex_repo();
        repo.annex_refs_hidden = true;
        repo.head_branch = Loadable::Ready(head.into());
        let rows = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "");
        let labels = branch_labels(&rows);
        assert!(labels.iter().any(|name| name == head), "{head}: {labels:?}");
        let other = if head == "git-annex" {
            "synced/main"
        } else {
            "git-annex"
        };
        assert!(
            !labels.iter().any(|name| name == other),
            "{head}: {labels:?}"
        );
        assert!(!labels.iter().any(|name| name == "origin/git-annex"));
    }
}

#[test]
fn pinned_remote_branch_resolution_does_not_guess_a_slash_boundary() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let pin = branch_pin_storage_key(BranchSection::Remote, "team/alice/main");
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "team/alice".to_string(),
        name: "main".to_string(),
        target: commit_id("aaaaaaaa"),
    }]));

    assert!(pinned_branch_renders(
        &repo,
        &pin,
        BranchSection::Remote,
        ""
    ));

    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "team/alice".to_string(),
            name: "main".to_string(),
            target: commit_id("aaaaaaaa"),
        },
        RemoteBranch {
            remote: "team".to_string(),
            name: "alice/main".to_string(),
            target: commit_id("bbbbbbbb"),
        },
    ]));
    assert!(
        !pinned_branch_renders(&repo, &pin, BranchSection::Remote, ""),
        "a legacy flat pin must not be attached to either ambiguous identity"
    );
}
use super::*;
use gitcomet_core::domain::{
    Branch, CommitId, FileStatus, FileStatusKind, Remote, RemoteBranch, RepoSpec, RepoStatus,
    StashEntry, Submodule, SubmoduleStatus, Upstream, Worktree,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

fn commit_id(id: &str) -> CommitId {
    CommitId(id.into())
}

fn populated_repo() -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: commit_id("aaaaaaaa"),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        }),
        divergence: None,
    }]));
    repo.branches_rev = 1;
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "origin".to_string(),
        url: Some("https://example.com/origin.git".to_string()),
        default_branch: None,
    }]));
    repo.remotes_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit_id("aaaaaaaa"),
    }]));
    repo.remote_branches_rev = 1;
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        Worktree {
            path: PathBuf::from("/tmp/repo"),
            head: Some(commit_id("aaaaaaaa")),
            branch: Some("main".to_string()),
            detached: false,
        },
        Worktree {
            path: PathBuf::from("/tmp/repo-linked"),
            head: Some(commit_id("bbbbbbbb")),
            branch: Some("feature/topic".to_string()),
            detached: false,
        },
    ]));
    repo.worktrees_rev = 1;
    repo.submodules = Loadable::Ready(Arc::new(vec![Submodule {
        path: PathBuf::from("vendor/lib"),
        recorded_head: commit_id("cccccccc"),
        checked_out_head: Some(commit_id("cccccccc")),
        status: SubmoduleStatus::UpToDate,
    }]));
    repo.submodules_rev = 1;
    repo.stashes = Loadable::Ready(Arc::new(vec![StashEntry {
        index: 0,
        id: commit_id("dddddddd"),
        message: "stash message".into(),
        created_at: None,
    }]));
    repo.stashes_rev = 1;
    repo
}

#[test]
fn pinned_branches_keep_their_tree_copies_without_extra_headers() {
    let repo = populated_repo();
    let pinned = BTreeSet::from([
        branch_pin_storage_key(BranchSection::Local, "main"),
        branch_pin_storage_key(BranchSection::Remote, "origin/main"),
    ]);
    let pins = expanded_pinned_rows(&repo, &pinned, &BTreeSet::new());
    assert_eq!(pins.len(), 2);
    let tree = expanded_sidebar_rows(&repo, &BTreeSet::new(), "");
    for pin in pins {
        let BranchSidebarRow::Branch { target, .. } = pin else {
            panic!("expected a branch pin")
        };
        assert!(tree.iter().any(|row| matches!(row, BranchSidebarRow::Branch { target: candidate, .. } if *candidate == target)));
    }
}

#[test]
fn pins_for_missing_branches_produce_no_rows() {
    let repo = populated_repo();
    let pinned = BTreeSet::from([branch_pin_storage_key(
        BranchSection::Local,
        "branch-that-was-deleted",
    )]);
    assert!(expanded_pinned_rows(&repo, &pinned, &BTreeSet::new()).is_empty());
}

#[test]
fn branch_filter_matches_case_insensitively_and_force_expands_collapsed_sections() {
    let repo = populated_repo();
    let has_local_main = |rows: &[BranchSidebarRow]| {
        rows.iter().any(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch {
                    name,
                    section: BranchSection::Local,
                    ..
                } if name.as_ref() == "main"
            )
        })
    };

    // A collapsed local section is force-expanded while filtering, and an
    // uppercase query still matches the lowercase branch name.
    let collapsed = BTreeSet::from([local_section_storage_key().to_string()]);
    let rows = branch_sidebar_rows(&repo, &collapsed, &BTreeSet::new(), "MAIN");
    assert!(
        has_local_main(&rows),
        "a matching filter should force-expand the collapsed local section and show the branch"
    );

    // A non-matching query hides the branch entirely.
    let rows = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "no-such-branch");
    assert!(
        !has_local_main(&rows),
        "a non-matching filter should hide the local branch"
    );
}

#[test]
fn source_fingerprint_ignores_status_only_changes() {
    let mut repo = populated_repo();
    let (before_fingerprint, before_parts) = branch_sidebar_source_fingerprint(&repo, None);

    repo.status = Loadable::Ready(Arc::new(RepoStatus {
        staged: std::sync::Arc::new(vec![]),
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: PathBuf::from("src/lib.rs"),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
    }));

    let (after_fingerprint, after_parts) =
        branch_sidebar_source_fingerprint(&repo, Some(&before_parts));

    assert_eq!(after_fingerprint, before_fingerprint);
    assert_eq!(after_parts, before_parts);
}

#[test]
fn source_fingerprint_reuses_unchanged_partitions_when_worktrees_change() {
    let mut repo = populated_repo();
    let (before_fingerprint, before_parts) = branch_sidebar_source_fingerprint(&repo, None);

    repo.worktrees = Loadable::Ready(Arc::new(vec![
        Worktree {
            path: PathBuf::from("/tmp/repo"),
            head: Some(commit_id("aaaaaaaa")),
            branch: Some("main".to_string()),
            detached: false,
        },
        Worktree {
            path: PathBuf::from("/tmp/repo-linked"),
            head: Some(commit_id("eeeeeeee")),
            branch: None,
            detached: true,
        },
    ]));
    repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1);

    let (after_fingerprint, after_parts) =
        branch_sidebar_source_fingerprint(&repo, Some(&before_parts));

    assert_ne!(after_fingerprint, before_fingerprint);
    assert_eq!(after_parts.local_hash, before_parts.local_hash);
    assert_eq!(after_parts.remote_hash, before_parts.remote_hash);
    assert_ne!(after_parts.worktree_hash, before_parts.worktree_hash);
    assert_eq!(after_parts.submodule_hash, before_parts.submodule_hash);
    assert_eq!(after_parts.stash_hash, before_parts.stash_hash);
}

#[test]
fn source_fingerprint_reuses_branch_partition_hashes_when_revs_bump_without_snapshot_change() {
    let mut repo = populated_repo();
    let (before_fingerprint, before_parts) = branch_sidebar_source_fingerprint(&repo, None);

    repo.branches_rev = repo.branches_rev.wrapping_add(1);

    let (after_fingerprint, after_parts) =
        branch_sidebar_source_fingerprint(&repo, Some(&before_parts));

    assert_eq!(after_fingerprint, before_fingerprint);
    assert_ne!(after_parts.local_revs, before_parts.local_revs);
    assert_ne!(after_parts.remote_revs, before_parts.remote_revs);
    assert_eq!(after_parts.local_hash, before_parts.local_hash);
    assert_eq!(after_parts.remote_hash, before_parts.remote_hash);
    assert_eq!(after_parts.worktree_hash, before_parts.worktree_hash);
    assert_eq!(after_parts.submodule_hash, before_parts.submodule_hash);
    assert_eq!(after_parts.stash_hash, before_parts.stash_hash);
}

#[test]
fn remote_rows_dedup_upstream_branches_that_also_exist_as_remote_refs() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.head_branch = Loadable::Ready("feature".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature".to_string(),
        target: commit_id("aaaaaaaa"),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        }),
        divergence: None,
    }]));
    repo.branches_rev = 1;
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "origin".to_string(),
        url: Some("https://example.com/origin.git".to_string()),
        default_branch: None,
    }]));
    repo.remotes_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "feature".to_string(),
        target: commit_id("aaaaaaaa"),
    }]));
    repo.remote_branches_rev = 1;

    let rows = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "");
    let matches = rows
        .iter()
        .filter(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch {
                    section: BranchSection::Remote,
                    name,
                    ..
                } if name.as_ref() == "origin/feature"
            )
        })
        .count();

    assert_eq!(matches, 1, "remote branch rows should be deduplicated");
}

#[test]
fn remote_rows_do_not_recreate_a_missing_upstream_tracking_ref() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "fix/below_EOF_click".to_string(),
        target: commit_id("aaaaaaaa"),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "fix/below_EOF_click".to_string(),
        }),
        divergence: None,
    }]));
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "origin".to_string(),
        url: Some("https://example.com/origin.git".to_string()),
        default_branch: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));

    let rows = branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), "");

    assert!(rows.iter().all(|row| {
        !matches!(
            row,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } if name.as_ref() == "origin/fix/below_EOF_click"
        )
    }));
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::Branch {
                section: BranchSection::Local,
                name,
                ..
            } if name.as_ref() == "fix/below_EOF_click"
        )
    }));
}

#[test]
fn worktree_label_handles_branchless_and_detached_states() {
    assert_eq!(
        branch_sidebar_worktree_label(None, false, "/tmp/repo").as_ref(),
        "/tmp/repo"
    );
    assert_eq!(
        branch_sidebar_worktree_label(None, true, "/tmp/repo").as_ref(),
        "(detached)  /tmp/repo"
    );
}

#[test]
fn expanded_worktrees_show_loading_until_a_list_arrives() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: "/repo".into(),
        },
    );
    for worktrees in [Loadable::NotLoaded, Loadable::Loading] {
        repo.worktrees = worktrees;
        let mut collapsed = BTreeSet::new();
        set_collapse_state(&mut collapsed, worktrees_section_storage_key(), false);
        let rows = branch_sidebar_rows(&repo, &collapsed, &BTreeSet::new(), "");
        assert!(
            rows.iter().any(|row| matches!(row,
                BranchSidebarRow::WorktreePlaceholder { message } if message.as_ref() == "Loading"
            )),
            "an expanded section must explain why there are no rows"
        );
        set_collapse_state(&mut collapsed, worktrees_section_storage_key(), true);
        let rows = branch_sidebar_rows(&repo, &collapsed, &BTreeSet::new(), "");
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, BranchSidebarRow::WorktreePlaceholder { .. }))
        );
    }
}

#[test]
fn branch_tooltip_only_appends_upstream_note_when_requested() {
    assert_eq!(
        branch_sidebar_branch_tooltip("origin/main", false).as_ref(),
        "Branch: origin/main"
    );
    assert_eq!(
        branch_sidebar_branch_tooltip("origin/main", true).as_ref(),
        "Branch: origin/main (upstream for current branch)"
    );
}
