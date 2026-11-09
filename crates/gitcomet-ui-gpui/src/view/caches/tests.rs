use super::*;
use std::path::PathBuf;
use std::time::SystemTime;

fn commit_id(id: &str) -> CommitId {
    CommitId(id.into())
}

fn commit(id: &str, parents: &[&str], summary: &str) -> Commit {
    Commit {
        id: commit_id(id),
        parent_ids: parents.iter().map(|parent| commit_id(parent)).collect(),
        summary: summary.into(),
        author: "author".into(),
        time: SystemTime::UNIX_EPOCH,
    }
}

#[test]
fn history_branch_text_cache_precomputes_head_and_remote_labels() {
    let commit_a = commit_id("a");
    let commit_b = commit_id("b");
    let branches = vec![
        Branch {
            name: "main".to_string(),
            target: commit_a.clone(),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature".to_string(),
            target: commit_a.clone(),
            upstream: None,
            divergence: None,
        },
    ];
    let remote_branches = vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit_a.clone(),
        },
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "topic".to_string(),
            target: commit_b.clone(),
        },
    ];

    let (branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
        &branches,
        &remote_branches,
        Some("main"),
        Some(commit_a.as_ref()),
    );

    assert_eq!(
        branch_text_by_target
            .get(commit_a.as_ref())
            .map(HistoryTextVm::as_ref),
        Some("feature, origin/main")
    );
    assert_eq!(
        branch_text_by_target
            .get(commit_b.as_ref())
            .map(HistoryTextVm::as_ref),
        Some("upstream/topic")
    );
    assert_eq!(
        head_branches_text.as_ref().map(HistoryTextVm::as_ref),
        Some("HEAD → main, feature, origin/main")
    );
}

#[test]
fn history_branch_text_dedups_and_orders_duplicate_names() {
    let commit = commit_id("a");
    let branches = vec![
        Branch {
            name: "topic".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "apple".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "topic".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
    ];
    let remote_branches = vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "zzz".to_string(),
            target: commit.clone(),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        },
    ];

    let (branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
        &branches,
        &remote_branches,
        Some("topic"),
        Some(commit.as_ref()),
    );

    assert_eq!(
        branch_text_by_target
            .get(commit.as_ref())
            .map(HistoryTextVm::as_ref),
        Some("apple, origin/main, origin/zzz")
    );
    assert_eq!(
        head_branches_text.as_ref().map(HistoryTextVm::as_ref),
        Some("HEAD → topic, apple, origin/main, origin/zzz")
    );
}

#[test]
fn history_branch_chips_merge_same_named_local_and_remote_refs_on_one_commit() {
    let commit = commit_id("a");
    let branches = vec![Branch {
        name: "main".to_string(),
        target: commit.clone(),
        upstream: None,
        divergence: None,
    }];
    let remote_branches = vec![
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        },
    ];

    let (chips_by_target, head_chips) = build_history_branch_chips_by_target(
        &branches,
        &remote_branches,
        Some("main"),
        Some(commit.as_ref()),
    );

    let normal = chips_by_target
        .get(commit.as_ref())
        .expect("expected compact branch chip");
    assert_eq!(normal.len(), 1);
    assert_eq!(normal[0].text.as_ref(), "main");
    assert!(matches!(
        normal[0].kind,
        HistoryBranchChipKind::Branch { is_head: false, .. }
    ));

    let head = head_chips.expect("expected head chips");
    assert_eq!(head.len(), 1);
    assert_eq!(head[0].text.as_ref(), "main");
    let HistoryBranchChipKind::Branch { is_head, targets } = &head[0].kind else {
        panic!("expected a branch chip");
    };
    assert!(*is_head);
    assert_eq!(
        targets.as_ref(),
        [
            BranchMenuTarget::local("main"),
            BranchMenuTarget::remote("origin", "main"),
            BranchMenuTarget::remote("upstream", "main"),
        ]
    );
}

#[test]
fn history_branch_chips_do_not_merge_same_name_across_commits() {
    let local_commit = commit_id("local");
    let remote_commit = commit_id("remote");
    let branches = vec![Branch {
        name: "topic".to_string(),
        target: local_commit.clone(),
        upstream: None,
        divergence: None,
    }];
    let remote_branches = vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "topic".to_string(),
        target: remote_commit.clone(),
    }];

    let (chips_by_target, _) =
        build_history_branch_chips_by_target(&branches, &remote_branches, None, None);
    let local_chip = &chips_by_target[local_commit.as_ref()][0];
    let remote_chip = &chips_by_target[remote_commit.as_ref()][0];
    assert_eq!(local_chip.text.as_ref(), "topic");
    assert_eq!(remote_chip.text.as_ref(), "topic");
    assert!(matches!(
        &local_chip.kind,
        HistoryBranchChipKind::Branch { targets, .. }
            if targets.as_ref() == [BranchMenuTarget::local("topic")]
    ));
    assert!(matches!(
        &remote_chip.kind,
        HistoryBranchChipKind::Branch { targets, .. }
            if targets.as_ref() == [BranchMenuTarget::remote("origin", "topic")]
    ));
}

#[test]
fn history_branch_chips_keep_slash_named_local_ref_distinct_from_remote_ref() {
    let commit = commit_id("a");
    let branches = vec![Branch {
        name: "origin/main".to_string(),
        target: commit.clone(),
        upstream: None,
        divergence: None,
    }];
    let remote_branches = vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit.clone(),
    }];

    let (chips_by_target, _) =
        build_history_branch_chips_by_target(&branches, &remote_branches, None, None);
    let chips = &chips_by_target[commit.as_ref()];
    assert_eq!(
        chips
            .iter()
            .map(|chip| chip.text.as_ref())
            .collect::<Vec<_>>(),
        ["main", "origin/main"]
    );
}

#[test]
fn history_branch_chips_keep_detached_head_in_its_own_chip() {
    let commit = commit_id("a");
    let branches = vec![Branch {
        name: "main".to_string(),
        target: commit.clone(),
        upstream: None,
        divergence: None,
    }];
    let remote_branches = vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit.clone(),
    }];

    let (_, head_chips) = build_history_branch_chips_by_target(
        &branches,
        &remote_branches,
        Some("HEAD"),
        Some(commit.as_ref()),
    );
    let head_chips = head_chips.expect("expected detached head chips");
    assert_eq!(head_chips.len(), 2);
    assert_eq!(head_chips[0].text.as_ref(), "HEAD");
    assert!(matches!(
        head_chips[0].kind,
        HistoryBranchChipKind::DetachedHead
    ));
    assert_eq!(head_chips[1].text.as_ref(), "main");
    assert!(matches!(
        &head_chips[1].kind,
        HistoryBranchChipKind::Branch { targets, .. } if targets.len() == 2
    ));
}

#[test]
fn history_tag_names_cache_dedups_once_per_target() {
    let commit_a = commit_id("a");
    let tags = vec![
        Tag {
            name: "v2.0.0".to_string(),
            target: commit_a.clone(),
        },
        Tag {
            name: "v1.0.0".to_string(),
            target: commit_a.clone(),
        },
        Tag {
            name: "v1.0.0".to_string(),
            target: commit_a.clone(),
        },
    ];

    let tag_names_by_target = build_history_tag_names_by_target(&tags);
    let tag_names = tag_names_by_target
        .get(commit_a.as_ref())
        .expect("tag names should be cached for the target");
    let tag_names = tag_names
        .iter()
        .map(HistoryTextVm::as_ref)
        .collect::<Vec<_>>();

    assert_eq!(tag_names, vec!["v1.0.0", "v2.0.0"]);
}

#[test]
fn history_ref_items_preserve_display_order_and_ref_types_for_attached_head() {
    let commit = commit_id("a");
    let branches = vec![
        Branch {
            name: "main".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        },
    ];
    let remote_branches = vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit.clone(),
    }];
    let tags = vec![
        Tag {
            name: "v2.0.0".to_string(),
            target: commit.clone(),
        },
        Tag {
            name: "v1.0.0".to_string(),
            target: commit.clone(),
        },
    ];

    let (_, head_branch_items) = build_history_branch_ref_items_by_target(
        &branches,
        &remote_branches,
        Some("main"),
        Some(commit.as_ref()),
    );
    let tag_names_by_target = build_history_tag_names_by_target(&tags);
    let tag_names = tag_names_by_target
        .get(commit.as_ref())
        .expect("expected tag names");
    let ref_items = history_ref_items_from_displayed_refs(
        tag_names,
        head_branch_items.expect("expected head branch items"),
    );

    let display = ref_items
        .iter()
        .map(|item| item.text.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        display,
        vec!["v1.0.0", "v2.0.0", "main", "feature", "origin/main"]
    );
    assert!(matches!(
        ref_items[0].kind,
        HistoryRefListItemKind::Tag { ref name } if name == "v1.0.0"
    ));
    assert!(matches!(
        ref_items[2].kind,
        HistoryRefListItemKind::AttachedHead { ref branch } if branch == "main"
    ));
    assert!(matches!(
        ref_items[3].kind,
        HistoryRefListItemKind::LocalBranch { ref name } if name == "feature"
    ));
    assert!(matches!(
        ref_items[4].kind,
        HistoryRefListItemKind::RemoteBranch { ref name, .. } if name == "origin/main"
    ));

    let hidden_tags = Arc::<[HistoryTextVm]>::from([]);
    let ref_items_without_tags = history_ref_items_from_displayed_refs(
        &hidden_tags,
        ref_items.iter().skip(2).cloned().collect::<Vec<_>>().into(),
    );
    let display = ref_items_without_tags
        .iter()
        .map(|item| item.text.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(display, vec!["main", "feature", "origin/main"]);
}

#[test]
fn history_ref_items_preserve_detached_head_as_informational() {
    let commit = commit_id("a");
    let branches = vec![Branch {
        name: "main".to_string(),
        target: commit.clone(),
        upstream: None,
        divergence: None,
    }];
    let remote_branches = vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit.clone(),
    }];

    let (_, head_branch_items) = build_history_branch_ref_items_by_target(
        &branches,
        &remote_branches,
        Some("HEAD"),
        Some(commit.as_ref()),
    );
    let items = head_branch_items.expect("expected detached head item");
    let display = items
        .iter()
        .map(|item| item.text.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(display, vec!["HEAD", "main", "origin/main"]);
    assert!(matches!(
        items[0].kind,
        HistoryRefListItemKind::DetachedHead
    ));
    assert!(matches!(
        items[1].kind,
        HistoryRefListItemKind::LocalBranch { ref name } if name == "main"
    ));
    assert!(matches!(
        items[2].kind,
        HistoryRefListItemKind::RemoteBranch { ref name, .. } if name == "origin/main"
    ));
}

#[test]
fn history_ref_items_keep_local_and_remote_refs_with_same_display_text() {
    let commit = commit_id("a");
    let branches = vec![Branch {
        name: "origin/main".to_string(),
        target: commit.clone(),
        upstream: None,
        divergence: None,
    }];
    let remote_branches = vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit.clone(),
    }];

    let (text_by_target, _) =
        build_history_branch_text_by_target(&branches, &remote_branches, None, None);
    assert_eq!(
        text_by_target
            .get(commit.as_ref())
            .expect("expected compact branch text")
            .as_ref(),
        "origin/main",
        "history row text should still dedupe duplicate display labels"
    );

    let (items_by_target, _) =
        build_history_branch_ref_items_by_target(&branches, &remote_branches, None, None);
    let items = items_by_target
        .get(commit.as_ref())
        .expect("expected hover branch refs");
    let display = items
        .iter()
        .map(|item| item.text.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(display, vec!["origin/main", "origin/main"]);
    assert!(matches!(
        items[0].kind,
        HistoryRefListItemKind::LocalBranch { ref name } if name == "origin/main"
    ));
    assert!(matches!(
        items[1].kind,
        HistoryRefListItemKind::RemoteBranch { ref name, .. } if name == "origin/main"
    ));
}

#[test]
fn history_stash_analysis_ignores_stash_ids_absent_from_log() {
    let commits = vec![
        commit("a", &[], "Commit A"),
        commit("b", &["a"], "Commit B"),
    ];
    let stashes = vec![StashEntry {
        index: 0,
        id: commit_id("z"),
        message: "On main: hidden stash".into(),
        created_at: None,
    }];

    let analysis = analyze_history_stashes(&commits, &stashes);

    assert!(analysis.stash_tips.is_empty());
    assert!(analysis.stash_helper_ids.is_empty());
}

#[test]
fn history_stash_analysis_keeps_matching_tip_message_and_helper() {
    let commits = vec![
        commit("base", &[], "Commit base"),
        commit("helper", &["base"], "index on main: helper"),
        commit("tip", &["base", "helper"], "WIP on main: fallback"),
    ];
    let stashes = vec![StashEntry {
        index: 0,
        id: commit_id("tip"),
        message: "On main: listed stash".into(),
        created_at: None,
    }];

    let analysis = analyze_history_stashes(&commits, &stashes);

    assert_eq!(analysis.stash_tips.len(), 1);
    assert_eq!(analysis.stash_tips[0].commit_ix, 2);
    assert_eq!(
        analysis.stash_tips[0].message.map(AsRef::as_ref),
        Some("On main: listed stash")
    );
    assert!(analysis.stash_helper_ids.contains("helper"));
}

#[test]
fn history_when_vm_formats_lazily_and_caches_result() {
    let display_key = HistoryDisplayKey::new(DateTimeFormat::YmdHm, Timezone::Utc, true, false);
    let when = HistoryWhenVm::deferred(SystemTime::UNIX_EPOCH);

    assert!(when.formatted.borrow().is_none());
    let first = when.resolve(display_key);
    let second = when.resolve(display_key);
    assert_eq!(first, second);
    assert!(when.formatted.borrow().is_some());
}

#[test]
fn history_short_sha_vm_formats_lazily_and_caches_result() {
    let short_sha = HistoryShortShaVm::new("0123456789abcdef");

    assert_eq!(short_sha.as_str(), "01234567");
    assert!(short_sha.formatted.borrow().is_none());
    let first = short_sha.resolve();
    let second = short_sha.resolve();
    assert_eq!(first.as_ref(), "01234567");
    assert_eq!(first, second);
    assert!(short_sha.formatted.borrow().is_some());
}

#[test]
fn history_short_sha_vm_preserves_short_ids_without_padding() {
    let short_sha = HistoryShortShaVm::new("abc");

    assert_eq!(short_sha.as_str(), "abc");
    assert_eq!(short_sha.resolve().as_ref(), "abc");
}

#[test]
fn detached_head_history_branch_text_adds_head_label_once() {
    let commit = commit_id("a");
    let branches = vec![Branch {
        name: "main".to_string(),
        target: commit.clone(),
        upstream: None,
        divergence: None,
    }];
    let remote_branches = vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: commit.clone(),
    }];

    let (branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
        &branches,
        &remote_branches,
        Some("HEAD"),
        Some(commit.as_ref()),
    );

    assert_eq!(
        branch_text_by_target
            .get(commit.as_ref())
            .map(HistoryTextVm::as_ref),
        Some("main, origin/main")
    );
    assert_eq!(
        head_branches_text.as_ref().map(HistoryTextVm::as_ref),
        Some("HEAD, main, origin/main")
    );
}

#[test]
fn branch_sidebar_cache_lookup_by_source_reuses_rows_and_updates_fingerprint() {
    let repo = RepoState::new_opening(
        RepoId(7),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let (source_fingerprint, source_parts) =
        branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
    let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
    let mut cache = None;

    branch_sidebar_cache_store(
        &mut cache,
        repo.id,
        BranchSidebarFingerprint { cache_rev: 1 },
        source_fingerprint,
        source_parts.clone(),
        Rc::clone(&rows),
    );

    let hit = branch_sidebar_cache_lookup_by_source(
        &mut cache,
        repo.id,
        BranchSidebarFingerprint { cache_rev: 2 },
        source_fingerprint,
        &source_parts,
    )
    .expect("matching source fingerprints should reuse cached rows");

    assert!(Rc::ptr_eq(&hit, &rows));
    let cached = cache
        .as_ref()
        .expect("branch sidebar cache should stay populated");
    assert_eq!(
        cached.fingerprint,
        BranchSidebarFingerprint { cache_rev: 2 }
    );
    assert_eq!(cached.source_fingerprint, source_fingerprint);
    assert_eq!(cached.source_parts, source_parts);
}

#[test]
fn branch_sidebar_cache_lookup_by_cached_source_reuses_rows_when_revs_bump() {
    let mut repo = RepoState::new_opening(
        RepoId(7),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let (source_fingerprint, source_parts) =
        branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
    let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
    let mut cache = None;

    branch_sidebar_cache_store(
        &mut cache,
        repo.id,
        BranchSidebarFingerprint { cache_rev: 1 },
        source_fingerprint,
        source_parts,
        Rc::clone(&rows),
    );

    repo.branches_rev = repo.branches_rev.wrapping_add(1);

    let hit = branch_sidebar_cache_lookup_by_cached_source(
        &mut cache,
        &repo,
        BranchSidebarFingerprint { cache_rev: 2 },
    )
    .expect("unchanged source snapshots should reuse cached rows");

    assert!(Rc::ptr_eq(&hit, &rows));
    let cached = cache
        .as_ref()
        .expect("branch sidebar cache should stay populated");
    assert_eq!(
        cached.fingerprint,
        BranchSidebarFingerprint { cache_rev: 2 }
    );
}

#[test]
fn branch_sidebar_cache_lookup_by_source_rejects_repo_and_source_mismatches() {
    let repo = RepoState::new_opening(
        RepoId(7),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let (source_fingerprint, source_parts) =
        branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
    let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
    let mut cache = None;

    branch_sidebar_cache_store(
        &mut cache,
        repo.id,
        BranchSidebarFingerprint { cache_rev: 1 },
        source_fingerprint,
        source_parts,
        rows,
    );

    let other_repo = RepoState::new_opening(
        RepoId(8),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/tmp/other"),
        },
    );
    let (other_source_fingerprint, other_source_parts) =
        branch_sidebar::branch_sidebar_source_fingerprint(&other_repo, None);

    assert!(
        branch_sidebar_cache_lookup_by_source(
            &mut cache,
            other_repo.id,
            BranchSidebarFingerprint { cache_rev: 2 },
            source_fingerprint,
            &other_source_parts,
        )
        .is_none()
    );
    assert!(
        branch_sidebar_cache_lookup_by_source(
            &mut cache,
            repo.id,
            BranchSidebarFingerprint { cache_rev: 2 },
            other_source_fingerprint,
            &other_source_parts,
        )
        .is_none()
    );
}

#[test]
fn branch_sidebar_cache_lookup_by_source_reuses_rows_after_worktrees_rev_bump() {
    let mut repo = RepoState::new_opening(
        RepoId(7),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let (source_fingerprint, source_parts) =
        branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
    let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
    let mut cache = None;

    branch_sidebar_cache_store(
        &mut cache,
        repo.id,
        BranchSidebarFingerprint { cache_rev: 1 },
        source_fingerprint,
        source_parts.clone(),
        Rc::clone(&rows),
    );

    repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1);

    let hit = branch_sidebar_cache_lookup_by_source(
        &mut cache,
        repo.id,
        BranchSidebarFingerprint { cache_rev: 2 },
        source_fingerprint,
        &source_parts,
    )
    .expect("matching source fingerprints should reuse cached rows after worktrees rev bump");

    assert!(Rc::ptr_eq(&hit, &rows));
}

#[test]
fn branch_sidebar_cache_fingerprint_changes_when_worktrees_rev_bumps() {
    let mut repo = RepoState::new_opening(
        RepoId(7),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );

    let fingerprint_before = BranchSidebarFingerprint::from_repo(&repo);

    repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1);

    let fingerprint_after = BranchSidebarFingerprint::from_repo(&repo);

    assert_ne!(fingerprint_before, fingerprint_after);
}
