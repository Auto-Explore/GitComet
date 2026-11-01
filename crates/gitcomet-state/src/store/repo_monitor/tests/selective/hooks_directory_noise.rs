//! Deterministic replay of the Windows CI hooks-directory event; real Gix policy.
use super::*;

struct Fixture {
    _temp: tempfile::TempDir,
    _seed: tempfile::TempDir,
    root: PathBuf,
    child_git: PathBuf,
    rules: TestRules,
}
fn fixture() -> Fixture {
    let (temp, root) = repository();
    let (seed_temp, seed) = repository();
    let subpath = "Assets/Standard Assets/CharacterBuilder";
    run_git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            seed.to_str().unwrap(),
            subpath,
        ],
    );
    run_git(&root, &["commit", "-am", "hooks-noise fixture submodule"]);
    let child_git = normalized(&resolve_git_dir(&root.join(subpath)).unwrap());
    assert_eq!(
        child_git,
        root.join(".git/modules/Assets/Standard Assets/CharacterBuilder")
    );
    fs::create_dir_all(child_git.join("hooks")).unwrap();
    let rules = load_gitignore_rules(&root);
    assert!(!rules.failed);
    assert!(rules.state.inputs.backend_discovered);
    assert!(!rules.state.inputs.info.discovery_incomplete);
    assert!(
        rules.state.snapshot().git_roots.contains(&child_git),
        "actual Gix discovery must recognize CI child Git directory"
    );
    Fixture {
        _temp: temp,
        _seed: seed_temp,
        root,
        child_git,
        rules,
    }
}
fn evaluate(fixture: &mut Fixture, event: &notify::Event) -> (Triage, EventEffect) {
    let policy = fixture.rules.state.snapshot();
    let triage = triage(&policy, event);
    let effect = summarize(&policy, &mut fixture.rules.state.rules, event);
    println!(
        "HOOKS_NOISE event={event:?} parent_known={} triage={triage:?} effect={effect:?}",
        event
            .paths
            .first()
            .and_then(|p| p.parent())
            .is_some_and(|p| policy.git_roots.contains(p))
    );
    (triage, effect)
}
fn modify(path: PathBuf) -> notify::Event {
    notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path)
}
fn require_git_change(fixture: &mut Fixture, path: PathBuf) {
    let (triage, effect) = evaluate(fixture, &modify(path));
    assert_eq!(triage, Triage::Relevant);
    assert!(effect.change.is_some_and(|change| change.git_state));
}
#[test]
fn ci_recorded_nested_hooks_modify_is_quiet() {
    let mut fixture = fixture();
    let hooks = fixture.child_git.join("hooks");
    let metadata = fs::symlink_metadata(&hooks).unwrap();
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        assert_eq!(
            metadata.file_attributes() & 0x400,
            0,
            "counterexample requires a plain non-reparse directory"
        );
    }
    let event = modify(hooks);
    let (triage, effect) = evaluate(&mut fixture, &event);
    assert_eq!(
        triage,
        Triage::Drop,
        "CI-recorded hooks directory echo must be dropped"
    );
    assert_eq!(effect.change, None);
    assert!(!effect.policy_dirty && !effect.index_dirty && effect.dir_added.is_empty());
}
#[test]
fn guard_missing_file_unknown_and_named_modules_stay_relevant() {
    let mut fixture = fixture();
    let hooks = fixture.child_git.join("hooks");
    let saved = fixture.child_git.join("hooks-original");
    fs::rename(&hooks, &saved).unwrap();
    require_git_change(&mut fixture, hooks.clone()); // Missing metadata stays conservative.
    fs::write(&hooks, "ordinary file at hooks boundary").unwrap();
    require_git_change(&mut fixture, hooks.clone());
    fs::remove_file(&hooks).unwrap();
    fs::rename(&saved, &hooks).unwrap();
    for path in [
        fixture.child_git.join("unknown-directory"),
        fixture.root.join(".git/modules/unknown/hooks"),
    ] {
        fs::create_dir_all(&path).unwrap();
        require_git_change(&mut fixture, path);
    }
    let descendant = hooks.join("custom-hook");
    fs::write(&descendant, "hook file edit stays relevant").unwrap();
    require_git_change(&mut fixture, descendant);
    let named_module = fixture.root.join(".git/modules/hooks");
    run_git(
        &fixture.root,
        &["init", "--bare", named_module.to_str().unwrap()],
    );
    fixture.rules.reload(&fixture.root);
    assert!(
        fixture
            .rules
            .state
            .snapshot()
            .git_roots
            .contains(&named_module)
    );
    for path in [
        named_module.join("HEAD"),
        named_module.join("refs/heads/hooks/topic"),
        named_module.join("unknown-metadata"),
        fixture.child_git.join("hooks-not-private"),
    ] {
        require_git_change(&mut fixture, path);
    }
}
#[test]
fn guard_mixed_real_paths_remain_relevant() {
    let mut fixture = fixture();
    let event = modify(fixture.child_git.join("hooks"))
        .add_path(fixture.child_git.join("HEAD"))
        .add_path(fixture.root.join(".git/index"))
        .add_path(fixture.root.join("source.txt"));
    let (triage, effect) = evaluate(&mut fixture, &event);
    assert_eq!(triage, Triage::Relevant);
    let change = effect.change.unwrap();
    assert!(change.git_state && change.index && change.worktree);
    assert_eq!(
        change.paths,
        crate::msg::ChangedPaths::known(vec![PathBuf::from("source.txt")])
    );
}
#[test]
fn guard_structural_and_rescan_events_remain_relevant() {
    let mut fixture = fixture();
    for path in [
        fixture.root.join(".git"),
        fixture.child_git.clone(),
        fixture.child_git.join("hooks"),
    ] {
        for kind in [
            EventKind::Create(CreateKind::Folder),
            EventKind::Remove(RemoveKind::Folder),
            EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Any)),
        ] {
            let event = notify::Event::new(kind).add_path(path.clone());
            let (triage, effect) = evaluate(&mut fixture, &event);
            assert_eq!(triage, Triage::Relevant);
            assert!(effect.change.is_some());
        }
        let event = modify(path).set_flag(notify::event::Flag::Rescan);
        let (triage, effect) = evaluate(&mut fixture, &event);
        assert_eq!(triage, Triage::Rescan);
        assert_eq!(effect.change, Some(RepoExternalChange::all()));
    }
    for path in [
        fixture.root.join(".git/HEAD"),
        fixture.root.join(".git/refs/heads/main"),
        fixture.root.join(".git/unknown-metadata"),
    ] {
        require_git_change(&mut fixture, path);
    }
}
#[test]
fn guard_shallow_planner_still_discovers_hooks() {
    let mut fixture = fixture();
    let nested = fixture.child_git.join("hooks/nested");
    fs::create_dir_all(&nested).unwrap();
    let policy = fixture.rules.state.snapshot();
    let mut plan = WatchPlan::default();
    let mut registered = Vec::new();
    plan.walk(
        [fixture.child_git.clone()],
        false,
        &policy,
        &mut fixture.rules.state.rules,
        &mut fixture.rules.state.inputs,
        MAX_WORKTREE_WATCH_DIRS,
        |path| {
            registered.push(path.to_path_buf());
            Ok(())
        },
    );
    assert!(registered.contains(&fixture.child_git.join("hooks")));
    assert!(registered.contains(&nested));
    assert!(plan.dirs.contains(&nested));
    println!("HOOKS_SHALLOW_PLANNER positive_hooks_and_nested_registration=true");
}
#[test]
fn guard_linked_and_reparse_hooks_stay_relevant() {
    let mut fixture = fixture();
    let hooks = fixture.child_git.join("hooks");
    let target = fixture.child_git.join("hooks-target");
    fs::rename(&hooks, &target).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let result = std::process::Command::new("cmd")
            .current_dir(&fixture.child_git)
            .args(["/d", "/c", "mklink", "/J", "hooks", "hooks-target"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "actual junction prerequisite failed: {result:?}"
        );
        let metadata = fs::symlink_metadata(&hooks).unwrap();
        assert_ne!(metadata.file_attributes() & 0x400, 0);
        require_git_change(&mut fixture, hooks.clone());
        fs::remove_dir(&hooks).unwrap();
        println!("HOOKS_JUNCTION_REPARSE exercised=true remains_relevant=true");
    }
    if gitcomet_core::test_support::symlink::directory(&target, &hooks) {
        require_git_change(&mut fixture, hooks.clone());
        #[cfg(windows)]
        fs::remove_dir(&hooks).unwrap();
        #[cfg(unix)]
        fs::remove_file(&hooks).unwrap();
        println!("HOOKS_DIRECTORY_SYMLINK exercised=true remains_relevant=true");
    } else {
        eprintln!(
            "HOOKS_DIRECTORY_SYMLINK unsupported-unexercised; actual junction proof remains separate"
        );
    }
    fs::rename(&target, &hooks).unwrap();
}
