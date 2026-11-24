use super::*;
use gitcomet_core::services::{GitRepository, RepositoryWatchInfo};

struct Backend(Option<RepositoryWatchInfo>);
impl GitBackend for Backend {
    fn open(&self, _: &Path) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
        unreachable!("metadata comparison must not open a repository")
    }
    fn repository_watch_info(
        &self,
        _: &Path,
    ) -> gitcomet_core::services::Result<Option<RepositoryWatchInfo>> {
        Ok(self.0.clone())
    }
}

fn fixture() -> (tempfile::TempDir, Backend) {
    let dir = tempfile::tempdir().unwrap();
    let git_dir = dir.path().join(".git");
    fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(git_dir.join("refs/heads/main"), "1111\n").unwrap();
    let backend = Backend(Some(RepositoryWatchInfo {
        git_dirs: vec![git_dir],
        ..Default::default()
    }));
    (dir, backend)
}

fn assert_working_changes_only(change: RepoExternalChange) {
    assert!(!change.git_state && !change.tags);
    assert!(change.worktree && change.index);
    assert!(change.large_file_support && change.text_attributes);
}

#[test]
fn unchanged_focus_and_worktree_edits_do_not_reload_history() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    assert!(refresh.change(dir.path(), &backend).git_state);
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    fs::write(
        dir.path().join("changed.txt"),
        "external edit without a watcher event",
    )
    .unwrap();
    assert_working_changes_only(refresh.change(dir.path(), &backend));
}

#[test]
fn missed_git_metadata_changes_restore_full_refresh() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    for name in [
        "HEAD",
        "refs/heads/main",
        "refs/heads/nested/topic",
        "refs/tags/v1",
        "packed-refs",
        "reftable/tables.list",
        "shallow",
        "config",
        "MERGE_HEAD",
        "rebase-merge/head-name",
        "sequencer/todo",
        "logs/HEAD",
    ] {
        let path = dir.path().join(".git").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("new value for {name}\n")).unwrap();
        let change = refresh.change(dir.path(), &backend);
        assert!(change.git_state && change.tags, "missed change: {name}");
        assert_working_changes_only(refresh.change(dir.path(), &backend));
        fs::remove_file(&path).unwrap();
        assert!(
            refresh.change(dir.path(), &backend).git_state,
            "missed deletion: {name}"
        );
    }
}

#[test]
fn fetch_head_rewrites_keep_history_and_tags() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    fs::write(
        dir.path().join(".git/FETCH_HEAD"),
        "1111\t\tbranch 'main'\n",
    )
    .unwrap();
    assert_working_changes_only(refresh.change(dir.path(), &backend));
}

#[test]
fn common_directory_and_external_config_inputs_are_checked() {
    let (dir, mut backend) = fixture();
    let external = tempfile::tempdir().unwrap();
    let common = external.path().join("common");
    fs::create_dir_all(common.join("refs/heads")).unwrap();
    let config = external.path().join("included-config");
    let info = backend.0.as_mut().unwrap();
    info.git_dirs.push(common.clone());
    info.ignore_inputs.push(config.clone());
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    fs::write(common.join("refs/heads/main"), "2222\n").unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
    fs::write(&config, "[branch.main]\n remote = origin\n").unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
    assert_working_changes_only(refresh.change(dir.path(), &backend));
}

#[test]
fn index_only_changes_keep_history_and_tags_but_still_refresh_status() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    fs::write(dir.path().join(".git/index"), "new staged contents").unwrap();
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    fs::remove_file(dir.path().join(".git/index")).unwrap();
    assert_working_changes_only(refresh.change(dir.path(), &backend));
}

#[test]
fn duplicate_control_inputs_are_read_and_budgeted_once() {
    let (dir, mut backend) = fixture();
    let config = dir.path().join(".git/config");
    fs::write(&config, vec![b'#'; 3 * 1024 * 1024]).unwrap();
    backend
        .0
        .as_mut()
        .unwrap()
        .ignore_inputs
        .push(config.clone());
    let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
    let snapshot = MetadataSnapshot::capture(dir.path(), &inputs).unwrap();
    // The same config is an explicit discovered input and a Git control.
    // Counting both would exceed the 4 MiB budget and disable narrowing.
    assert!(snapshot.bytes < 3 * 1024 * 1024 + 1024);
    assert_eq!(
        snapshot.entries[&config]
            .as_ref()
            .unwrap()
            .contents
            .as_ref()
            .unwrap()
            .len(),
        3 * 1024 * 1024
    );
}

#[test]
fn aliased_control_inputs_share_the_content_budget_and_keep_discovery_paths() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    let config = dir.path().join("config");
    let alias = dir.path().join("nested/../config");
    fs::write(&config, vec![b'#'; 3 * 1024 * 1024]).unwrap();
    assert_ne!(config, alias);
    assert_eq!(
        config.canonicalize().unwrap(),
        alias.canonicalize().unwrap()
    );

    let mut snapshot = MetadataSnapshot::default();
    snapshot.add_discovery(&config).unwrap();
    snapshot.add_discovery(&alias).unwrap();
    assert_eq!(snapshot.bytes, 3 * 1024 * 1024);
    for path in [&config, &alias] {
        assert!(snapshot.discovery.contains_key(path));
        assert_eq!(
            snapshot.entries[path]
                .as_ref()
                .unwrap()
                .contents
                .as_ref()
                .unwrap()
                .len(),
            3 * 1024 * 1024
        );
    }
    let previous = snapshot;
    fs::write(&config, "changed").unwrap();
    let mut current = MetadataSnapshot::default();
    current.add_discovery(&config).unwrap();
    current.add_discovery(&alias).unwrap();
    assert!(!previous.discovery_still_matches(&current));
}

fn check_directory_link_inputs(create_link: impl Fn(&Path, &Path) -> bool) {
    let (dir, mut backend) = fixture();
    let external = tempfile::tempdir().unwrap();
    // Recreate an ancestor alias such as macOS /var -> /private/var on
    // every platform, independently of the host's temporary directory.
    let external_root = normalized(&external.path().canonicalize().unwrap());
    let physical_parent = external_root.join("real-parent");
    fs::create_dir(&physical_parent).unwrap();
    let parent_link = external_root.join("parent-link");
    if !create_link(&physical_parent, &parent_link) {
        return;
    }
    let first = physical_parent.join("target-a");
    let second = physical_parent.join("target-b");
    let link = parent_link.join("linked");
    for target in [&first, &second] {
        fs::create_dir(target).unwrap();
        fs::write(target.join("config"), "same configuration").unwrap();
        // An ancestor link must not expand into unrelated directory data.
        fs::write(
            target.join("unrelated"),
            vec![b'x'; MAX_CONTROL_FILE_BYTES as usize + 1],
        )
        .unwrap();
    }
    if !create_link(&first, &link) {
        return;
    }
    let mut recursive = MetadataSnapshot::default();
    assert!(recursive.add(&link, true, true).is_err());
    assert!(
        recursive.entries.is_empty(),
        "recursive links must fail before walking their targets"
    );
    backend
        .0
        .as_mut()
        .unwrap()
        .ignore_inputs
        .extend([link.join("config"), link.join("missing")]);
    let capture = || {
        let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
        MetadataSnapshot::capture(dir.path(), &inputs).unwrap()
    };
    let snapshot = capture();
    assert!(snapshot.discovery.contains_key(&parent_link));
    for input in [link.join("config"), link.join("missing")] {
        assert!(snapshot.discovery.contains_key(&input));
    }
    // Discovery resolves ancestor links before recording a nested link.
    // Resolve its parent only: canonicalizing the final link would name
    // target-a and lose the link identity whose retargeting we are testing.
    let physical_link =
        normalized(&link.parent().unwrap().canonicalize().unwrap()).join(link.file_name().unwrap());
    assert!(snapshot.discovery.contains_key(&physical_link));
    assert!(
        snapshot
            .discovery
            .contains_key(&normalized(&first.canonicalize().unwrap()))
    );
    assert!(!snapshot.entries.contains_key(&first.join("unrelated")));
    assert!(!snapshot.entries.contains_key(&link.join("unrelated")));
    let mut refresh = ActivationRefresh::default();
    refresh.complete(Some(snapshot), ActivationContext::ready_for_test());
    assert_working_changes_only(
        refresh.complete(Some(capture()), ActivationContext::ready_for_test()),
    );
    fs::write(first.join("config"), "edited configuration").unwrap();
    let edited = capture();
    assert!(refresh.needs_discovery(Ok(&edited)));
    assert!(
        refresh
            .complete(Some(edited), ActivationContext::ready_for_test())
            .git_state
    );
    fs::write(first.join("config"), "same configuration").unwrap();
    refresh.complete(Some(capture()), ActivationContext::ready_for_test());
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
    assert!(create_link(&second, &link));
    let retargeted = capture();
    assert!(
        refresh.needs_discovery(Ok(&retargeted)),
        "equal file bytes must not hide link retargeting"
    );
    assert!(
        refresh
            .complete(Some(retargeted), ActivationContext::ready_for_test())
            .git_state
    );
    assert_working_changes_only(
        refresh.complete(Some(capture()), ActivationContext::ready_for_test()),
    );
}

#[test]
fn directory_link_inputs_track_targets_and_retarget_without_recursive_scans() {
    check_directory_link_inputs(|target, link| {
        gitcomet_core::test_support::symlink::directory(target, link)
    });
}

#[cfg(windows)]
#[test]
fn directory_junction_inputs_track_targets_and_retarget_without_recursive_scans() {
    check_directory_link_inputs(|target, link| {
        // Fixed relative fixture names keep this shell invocation independent
        // of user-profile spelling and require no symlink privilege.
        let target_name = target.file_name().unwrap().to_str().unwrap();
        assert!(matches!(
            target_name,
            "real-parent" | "target-a" | "target-b"
        ));
        let link_name = link.file_name().unwrap().to_str().unwrap();
        assert!(matches!(link_name, "parent-link" | "linked"));
        let output = std::process::Command::new("cmd")
            .current_dir(link.parent().unwrap())
            .args(["/d", "/c", "mklink", "/J", link_name, target_name])
            .output()
            .unwrap();
        assert!(output.status.success(), "junction fixture: {output:?}");
        true
    });
}

#[test]
fn file_link_inputs_track_target_contents_and_intermediate_retargeting() {
    let (dir, mut backend) = fixture();
    let external = tempfile::tempdir().unwrap();
    let first = external.path().join("first");
    let second = external.path().join("second");
    let middle = external.path().join("middle");
    let link = external.path().join("link");
    fs::write(&first, "same").unwrap();
    fs::write(&second, "same").unwrap();
    if !gitcomet_core::test_support::symlink::file(Path::new("first"), &middle)
        || !gitcomet_core::test_support::symlink::file(Path::new("middle"), &link)
    {
        return;
    }
    backend.0.as_mut().unwrap().ignore_inputs.push(link.clone());
    let mut refresh = ActivationRefresh::default();
    assert!(refresh.change(dir.path(), &backend).git_state);
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    fs::write(&first, "edit").unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    fs::write(&first, "same").unwrap();
    refresh.change(dir.path(), &backend);
    fs::remove_file(&middle).unwrap();
    assert!(gitcomet_core::test_support::symlink::file(
        Path::new("second"),
        &middle
    ));
    let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
    let retargeted = ActivationRefresh::prepare(dir.path(), &inputs);
    assert!(retargeted.is_ok());
    assert!(refresh.needs_discovery(retargeted.as_ref()));
    assert!(
        refresh
            .complete(retargeted.ok(), ActivationContext::ready_for_test())
            .git_state
    );
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    // Canonical identity and bytes stay unchanged, but the link's spelling
    // still changes and must invalidate discovery.
    fs::remove_file(&middle).unwrap();
    // Windows stores relative symlink targets without normalizing forward
    // slashes. Build the alternate spelling with the native separator.
    let alternate_target = Path::new(".").join("second");
    assert!(gitcomet_core::test_support::symlink::file(
        &alternate_target,
        &middle
    ));
    assert_eq!(
        fs::read_link(&middle).unwrap().as_os_str(),
        alternate_target.as_os_str()
    );
    assert_eq!(
        middle
            .canonicalize()
            .expect("alternate link target resolves"),
        second.canonicalize().unwrap()
    );
    assert_eq!(
        fs::read(&link).expect("the link chain still reads the same target"),
        b"same"
    );
    let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
    let retargeted = ActivationRefresh::prepare(dir.path(), &inputs);
    assert!(retargeted.is_ok());
    assert!(refresh.needs_discovery(retargeted.as_ref()));
}

#[test]
fn unavailable_metadata_snapshot_does_not_request_watcher_rebuild() {
    let (dir, backend) = fixture();
    let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    fs::write(
        dir.path().join(".git/config"),
        vec![b'#'; MAX_CONTROL_FILE_BYTES as usize + 1],
    )
    .unwrap();
    let probe = ActivationRefresh::prepare(dir.path(), &inputs);
    assert!(matches!(probe, Err(SnapshotError::Unavailable)));
    assert!(!refresh.needs_discovery(probe.as_ref()));
    assert!(
        refresh
            .complete(probe.ok(), ActivationContext::ready_for_test())
            .git_state
    );
    fs::remove_file(dir.path().join(".git/config")).unwrap();
    let recovered = ActivationRefresh::prepare(dir.path(), &inputs);
    assert!(recovered.is_ok());
    assert!(
        refresh.needs_discovery(recovered.as_ref()),
        "recovering a readable snapshot must rediscover exact control inputs"
    );
}

#[test]
fn duplicate_visits_upgrade_recursive_and_content_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("child");
    fs::write(&file, "bytes").unwrap();
    let mut snapshot = MetadataSnapshot::default();
    snapshot.add(dir.path(), true, false).unwrap();
    snapshot.add(dir.path(), false, true).unwrap();
    snapshot.add(dir.path(), true, true).unwrap();
    assert_eq!(
        snapshot.entries[&file]
            .as_ref()
            .unwrap()
            .contents
            .as_deref(),
        Some(b"bytes".as_slice())
    );
    assert_eq!(snapshot.bytes, 5);
    snapshot.add(dir.path(), true, true).unwrap();
    snapshot.add(&file, false, true).unwrap();
    assert_eq!(snapshot.bytes, 5);
}

#[test]
fn linked_admin_roots_are_scanned_once_and_new_roots_require_discovery() {
    let (dir, mut backend) = fixture();
    let linked = dir.path().join(".git/worktrees/linked");
    fs::create_dir_all(linked.join("logs/refs/heads")).unwrap();
    fs::write(linked.join("HEAD"), "ref: refs/heads/linked\n").unwrap();
    fs::write(linked.join("index"), "staged files").unwrap();
    fs::write(
        linked.join("logs/refs/heads/linked"),
        "unused branch reflog",
    )
    .unwrap();
    backend.0.as_mut().unwrap().git_dirs.push(linked.clone());
    let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
    let snapshot = MetadataSnapshot::capture(dir.path(), &inputs).unwrap();
    assert!(!snapshot.entries.contains_key(&linked.join("index")));
    assert!(
        !snapshot
            .entries
            .contains_key(&linked.join("logs/refs/heads/linked"))
    );
    assert_eq!(snapshot.bytes, 49); // root HEAD/ref plus linked HEAD, once each
    let mut refresh = ActivationRefresh::default();
    refresh.complete(Some(snapshot), ActivationContext::ready_for_test());
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    fs::write(linked.join("HEAD"), "ref: refs/heads/other\n").unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
    fs::write(linked.join("gitdir"), "moved-checkout/.git\n").unwrap();
    let moved = ActivationRefresh::prepare(dir.path(), &inputs);
    assert!(refresh.needs_discovery(moved.as_ref()));
    assert!(refresh.change(dir.path(), &backend).git_state);
    fs::remove_dir_all(&linked).unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
    fs::create_dir_all(dir.path().join(".git/worktrees/new")).unwrap();
    let current = ActivationRefresh::prepare(dir.path(), &inputs);
    assert!(refresh.needs_discovery(current.as_ref()));
}

#[test]
fn only_the_displayed_head_reflog_invalidates_history() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    let logs = dir.path().join(".git/logs");
    fs::create_dir_all(logs.join("refs/heads")).unwrap();
    fs::write(logs.join("refs/heads/main"), "unused reflog").unwrap();
    assert_working_changes_only(refresh.change(dir.path(), &backend));
    fs::write(logs.join("HEAD"), "displayed reflog").unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
}

#[test]
fn incomplete_or_unsupported_discovery_never_skips_full_refresh() {
    let (dir, mut backend) = fixture();
    backend.0.as_mut().unwrap().discovery_incomplete = true;
    for backend in [backend, Backend(None)] {
        let mut refresh = ActivationRefresh::default();
        for _ in 0..2 {
            assert!(refresh.change(dir.path(), &backend).git_state);
        }
    }
}

#[test]
fn metadata_errors_discard_the_baseline_and_recover_conservatively() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    // A regular file where info is expected makes deeper inputs
    // unreadable and must not be cached as an unchanged error.
    fs::write(dir.path().join(".git/info"), "invalid info directory").unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
    assert!(refresh.change(dir.path(), &backend).git_state);
    fs::remove_file(dir.path().join(".git/info")).unwrap();
    fs::create_dir(dir.path().join(".git/info")).unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
    assert_working_changes_only(refresh.change(dir.path(), &backend));
}

#[test]
fn failed_or_cancelled_refresh_is_retried_with_identical_metadata() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    let mut context = ActivationContext::ready_for_test();
    assert!(
        refresh
            .check(dir.path(), &backend, context.clone())
            .git_state
    );
    assert_working_changes_only(refresh.check(dir.path(), &backend, context.clone()));
    // Even a failed load that retains Ready data increments diagnostics.
    context.diagnostics_seq += 1;
    assert!(
        refresh
            .check(dir.path(), &backend, context.clone())
            .git_state
    );
    assert_working_changes_only(refresh.check(dir.path(), &backend, context.clone()));
    context.epoch = context.epoch.next();
    assert!(
        refresh
            .check(dir.path(), &backend, context.clone())
            .git_state
    );
    context.ready = false;
    for _ in 0..2 {
        assert!(
            refresh
                .check(dir.path(), &backend, context.clone())
                .git_state
        );
    }
}

#[test]
fn displayed_state_changes_invalidate_an_old_matching_disk_snapshot() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    let mut context = ActivationContext::ready_for_test();
    refresh.check(dir.path(), &backend, context.clone());
    assert_working_changes_only(refresh.check(dir.path(), &backend, context.clone()));
    // The watcher loaded B after activation at A; an unseen reset put disk
    // back at A. An unchanged disk snapshot must not keep displaying B.
    context.revisions[1] += 1;
    assert!(
        refresh
            .check(dir.path(), &backend, context.clone())
            .git_state
    );
    assert_working_changes_only(refresh.check(dir.path(), &backend, context));
}

#[test]
fn ref_content_changes_are_detected_with_preserved_size_and_mtime() {
    let (dir, backend) = fixture();
    let mut refresh = ActivationRefresh::default();
    refresh.change(dir.path(), &backend);
    let path = dir.path().join(".git/refs/heads/main");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "2222\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(refresh.change(dir.path(), &backend).git_state);
}

#[test]
fn delayed_activation_reply_revalidates_current_state_and_repository_lifetime() {
    let mut repo = crate::model::RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    let context = ActivationContext::for_repo(&repo);
    let checked = || CheckedActivation {
        repo_id: repo.id,
        context: context.clone(),
        change: RepoExternalChange {
            git_state: false,
            tags: false,
            ..RepoExternalChange::all()
        },
    };
    assert!(!checked().into_refresh(&repo).unwrap().change.git_state);
    // The UI accepted a different history result while metadata was read.
    let pending = checked();
    repo.history_state.log_rev += 1;
    assert!(pending.into_refresh(&repo).unwrap().change.git_state);
    let reopened = crate::model::RepoState::new_opening(repo.id, repo.spec.clone());
    let pending = CheckedActivation {
        repo_id: repo.id,
        context,
        change: RepoExternalChange::all(),
    };
    assert!(pending.into_refresh(&reopened).is_none());
}

#[test]
fn activation_only_skips_a_settled_empty_single_checkout_scan() {
    use crate::model::{LoadKind, Loadable, RepoState};
    use gitcomet_core::domain::{RepoSpec, Worktree, WorktreeDirtySummary};

    let make_repo = || {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("repo"),
            },
        );
        repo.set_open(Loadable::Ready(()));
        repo.set_worktrees(Loadable::Ready(vec![Worktree {
            bare: false,
            path: repo.spec.workdir.clone(),
            head: None,
            branch: Some("main".into()),
            detached: false,
        }]));
        repo.set_worktree_dirty(Loadable::Ready(Vec::new()));
        repo
    };
    let decide = |repo: &RepoState| {
        CheckedActivation {
            repo_id: repo.id,
            context: ActivationContext::for_repo(repo),
            change: RepoExternalChange {
                git_state: false,
                tags: false,
                ..RepoExternalChange::all()
            },
        }
        .into_refresh(repo)
        .unwrap()
    };
    assert!(!decide(&make_repo()).refresh_worktree_dirty);
    type Invalidate = fn(&mut RepoState);
    let cases: [(&str, Invalidate); 8] = [
        ("unknown list", |repo| {
            repo.set_worktrees(Loadable::NotLoaded)
        }),
        ("failed list", |repo| {
            repo.set_worktrees(Loadable::Error("unavailable".into()))
        }),
        ("unknown dirty scan", |repo| {
            repo.set_worktree_dirty(Loadable::NotLoaded)
        }),
        ("failed dirty scan", |repo| {
            repo.set_worktree_dirty(Loadable::Error("unavailable".into()))
        }),
        ("retained dirty rows", |repo| {
            repo.set_worktree_dirty(Loadable::Ready(vec![WorktreeDirtySummary::default()]))
        }),
        ("list in flight", |repo| {
            repo.loads_in_flight.request(LoadKind::Worktrees);
        }),
        ("scan in flight", |repo| {
            repo.loads_in_flight
                .request_worktree_dirty(crate::model::WorktreeDirtyScope::All);
        }),
        ("local action", |repo| repo.local_actions_in_flight = 1),
    ];
    for (case, invalidate) in cases {
        let mut repo = make_repo();
        invalidate(&mut repo);
        assert!(decide(&repo).refresh_worktree_dirty, "{case}");
    }
    let mut repo = make_repo();
    let mut list = repo.worktrees.ready().unwrap().as_ref().clone();
    let mut linked = list[0].clone();
    linked.path = PathBuf::from("linked");
    list.push(linked.clone());
    repo.set_worktrees(Loadable::Ready(list));
    assert!(decide(&repo).refresh_worktree_dirty, "linked checkout");
    repo.set_worktrees(Loadable::Ready(vec![linked]));
    assert!(
        decide(&repo).refresh_worktree_dirty,
        "sole path is not the current checkout"
    );
}

#[test]
fn equal_successful_refreshes_preserve_the_production_activation_context() {
    use crate::model::Loadable;
    let mut repo = crate::model::RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    let page = Arc::new(gitcomet_core::domain::LogPage {
        commits: Vec::new(),
        next_cursor: None,
    });
    let apply = |repo: &mut crate::model::RepoState| {
        repo.set_head_branch(Loadable::Ready("main".into()));
        repo.set_log(Loadable::Ready(page.clone()));
        repo.set_branches(Loadable::Ready(Vec::new()));
        repo.set_remote_branches(Loadable::Ready(Vec::new()));
        repo.set_tags(Loadable::Ready(Vec::new()));
        repo.set_remotes(Loadable::Ready(Vec::new()));
        repo.set_stashes(Loadable::Ready(Vec::new()));
        repo.set_reflog(Loadable::Ready(Vec::new()));
        repo.set_worktrees(Loadable::Ready(Vec::new()));
        repo.set_upstream_divergence(Loadable::Ready(None));
        repo.set_rebase_in_progress(Loadable::Ready(false));
        repo.set_sequencer_state(Loadable::Ready(Default::default()));
        repo.set_merge_commit_message(Loadable::Ready(None));
    };
    apply(&mut repo);
    let before = ActivationContext::for_repo(&repo);
    assert!(before.ready);
    apply(&mut repo);
    assert_eq!(before, ActivationContext::for_repo(&repo));
}
