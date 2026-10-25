use super::*;
use crate::model::RepoState;
use gitcomet_core::domain::{DiffArea, RepoSpec};

fn target(path: &str) -> DiffTarget {
    DiffTarget::working_tree(PathBuf::from(path), DiffArea::Unstaged)
}

fn thread_state_with_target(
    repo_id: RepoId,
    selected: DiffTarget,
    target_rev: u64,
) -> Arc<RwLock<Arc<AppState>>> {
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/selected-diff-guard-test"),
        },
    );
    repo.diff_state.diff_target = Some(selected);
    repo.diff_state.diff_target_rev = target_rev;

    let mut state = AppState::default();
    state.repos.push(repo);
    Arc::new(RwLock::new(Arc::new(state)))
}

#[test]
fn selected_diff_guard_accepts_current_target_and_revision() {
    let repo_id = RepoId(1);
    let selected = target("src/lib.rs");
    let guard = SelectedDiffLoadGuard::new(
        thread_state_with_target(repo_id, selected.clone(), 7),
        repo_id,
        selected,
        7,
    );

    assert!(guard.is_current());
}

#[test]
fn selected_diff_guard_rejects_stale_target_or_revision() {
    let repo_id = RepoId(1);
    let selected = target("src/lib.rs");
    let thread_state = thread_state_with_target(repo_id, selected.clone(), 7);

    let stale_revision =
        SelectedDiffLoadGuard::new(Arc::clone(&thread_state), repo_id, selected.clone(), 6);
    assert!(!stale_revision.is_current());

    let stale_target = SelectedDiffLoadGuard::new(thread_state, repo_id, target("src/main.rs"), 7);
    assert!(!stale_target.is_current());
}

fn finish_filesystem_rename(thread_state: &Arc<RwLock<Arc<AppState>>>) -> Vec<crate::msg::Effect> {
    let mut state = (**thread_state.read().unwrap()).clone();
    let root = &state.repos[0].spec.workdir;
    let changes = vec![gitcomet_core::filesystem::PathChange {
        old: Some(root.join("src")),
        new: Some(root.join("renamed")),
    }];
    let effects = crate::store::reducer::reduce(
        &mut Default::default(),
        &std::sync::atomic::AtomicU64::new(2),
        &mut state,
        Msg::FilesystemPathsChanged(changes),
    );
    *thread_state.write().unwrap() = Arc::new(state);
    effects
}

#[test]
fn filesystem_changes_preserve_pending_loads_for_unchanged_diff_targets() {
    use gitcomet_core::domain::CommitId;
    for selected in [
        DiffTarget::commit_range(CommitId("abc".into()), Some(CommitId("def".into())), None),
        DiffTarget::commit(CommitId("abc".into()), "src/lib.rs".into()),
        DiffTarget::commit_range(
            CommitId("abc".into()),
            Some(CommitId("def".into())),
            Some("src/lib.rs".into()),
        ),
        DiffTarget::working_tree("src/lib.rs".into(), DiffArea::Staged),
        target("unrelated.rs"),
    ] {
        let state = thread_state_with_target(RepoId(1), selected.clone(), 7);
        let guard = SelectedDiffLoadGuard::new(state.clone(), RepoId(1), selected, 7);
        finish_filesystem_rename(&state);
        assert!(
            guard.is_current(),
            "an unaffected pending read must still be delivered"
        );
    }
}

#[test]
fn filesystem_moves_replace_pending_diff_loads_for_the_retargeted_path() {
    use crate::model::Loadable;
    use crate::msg::{Effect, InternalMsg};
    let selected = target("src/lib.rs");
    let state = thread_state_with_target(RepoId(1), selected.clone(), 7);
    let guard = SelectedDiffLoadGuard::new(state.clone(), RepoId(1), selected, 7);
    let effects = finish_filesystem_rename(&state);
    assert!(!guard.is_current());
    let expected = target("renamed/lib.rs");
    assert!(effects.iter().any(|effect| matches!(effect,
            Effect::LoadDiff { target, .. } if target == &expected)));
    let mut next = (**state.read().unwrap()).clone();
    assert!(matches!(next.repos[0].diff_state.diff, Loadable::Loading));
    crate::store::reducer::reduce(
        &mut Default::default(),
        &std::sync::atomic::AtomicU64::new(2),
        &mut next,
        Msg::Internal(InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target: expected.clone(),
            result: Ok(gitcomet_core::domain::Diff {
                target: expected,
                lines: vec![],
            }),
        }),
    );
    assert!(matches!(next.repos[0].diff_state.diff, Loadable::Ready(_)));
}

/// The repo-load trace explains slow or stuck loads, so selected-diff work
/// must record when it is queued, replaced, skipped, started and finished.
/// The trace target is read from the environment once per process, so the
/// scenario runs in a child test process with tracing enabled.
#[test]
fn selected_diff_loads_are_repo_load_traced() {
    const CHILD: &str = "GITCOMET_TEST_SELECTED_DIFF_TRACE";
    if std::env::var_os(CHILD).is_none() {
        let dir = tempfile::tempdir().unwrap();
        let trace_path = dir.path().join("repo-load.log");
        let name = concat!(module_path!(), "::selected_diff_loads_are_repo_load_traced");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name.split_once("::").unwrap().1, "--nocapture"])
            .env(CHILD, "1")
            .env("GITCOMET_REPO_LOAD_TRACE", &trace_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        let trace = std::fs::read_to_string(&trace_path).unwrap_or_default();
        let events: Vec<&str> = trace
            .lines()
            .filter_map(|line| line.split_once("] ").map(|(_, event)| event))
            .filter(|event| event.contains("task=selected_diff_"))
            .collect();
        assert_eq!(
            events,
            [
                "queue_repo_task repo_id=RepoId(1) task=selected_diff_patch",
                "queue_repo_task repo_id=RepoId(1) task=selected_diff_file_text",
                "queue_repo_task repo_id=RepoId(1) task=selected_diff_patch",
                "replace_queued_repo_task repo_id=RepoId(1) task=selected_diff_patch",
                "queue_repo_task repo_id=RepoId(1) task=selected_diff_submodule_summary",
                "repo_task_missing_handle repo_id=RepoId(2) task=selected_diff_file_image",
                "start_repo_task repo_id=RepoId(1) task=selected_diff_patch",
                "finish_repo_task repo_id=RepoId(1) task=selected_diff_patch",
                "skip_repo_task_stale_selection repo_id=RepoId(1) task=selected_diff_file_text",
                "skip_repo_task_cancelled_before_start repo_id=RepoId(1) task=selected_diff_submodule_summary",
            ],
            "full trace:\n{trace}"
        );
        return;
    }

    let repo_id = RepoId(1);
    let selected = target("src/lib.rs");
    let thread_state = thread_state_with_target(repo_id, selected.clone(), 1);
    let executor = TaskExecutor::new(1);
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    executor.spawn(move || {
        let _ = release_rx.recv();
    });
    let slots = super::super::super::executor::SelectedDiffSlots::default();
    let mut repos: RepositoryRuntimes = Default::default();
    repos.attach_repository_for_test(
        repo_id,
        Arc::new(crate::store::tests::DummyRepo::new(
            "/tmp/selected-diff-trace-test",
        )),
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let msg_tx = StoreWorkerSender::for_test_msg_sender(tx);
    let load = |patch, text, summary, image| SelectedDiffLoadOptions {
        load_text_attributes: false,
        load_patch_diff: patch,
        load_file_text: text,
        preview_text_side: None,
        load_submodule_summary: summary,
        load_file_image: image,
    };
    let schedule = |repo_id, msg_tx: &StoreWorkerSender, rev, options| {
        schedule_load_selected_diff(
            &executor,
            &slots,
            &repos,
            Arc::clone(&thread_state),
            msg_tx.clone(),
            repo_id,
            (selected.clone(), rev),
            CancellationToken::new(),
            options,
        );
    };

    schedule(repo_id, &msg_tx, 1, load(true, true, false, false));
    // A refresh while the worker is busy replaces the queued patch load and
    // leaves the queued file-text load stale.
    {
        let mut state = thread_state.write().unwrap();
        Arc::make_mut(&mut state).repos[0]
            .diff_state
            .diff_target_rev = 2;
    }
    schedule(repo_id, &msg_tx, 2, load(true, false, false, false));
    let cancelled = CancellationToken::new();
    let cancelled_tx = msg_tx.with_repo_load_guard(
        repo_id,
        crate::model::RepoLoadEpoch::default(),
        cancelled.clone(),
    );
    schedule(repo_id, &cancelled_tx, 2, load(false, false, true, false));
    cancelled.cancel();
    schedule(RepoId(2), &msg_tx, 2, load(false, false, false, true));

    release_tx.send(()).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    executor.spawn(move || done_tx.send(()).unwrap());
    done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("queued loads finish");
    assert!(matches!(
        rx.try_recv(),
        Ok(Msg::Internal(crate::msg::InternalMsg::DiffLoaded { .. }))
    ));
    assert!(rx.try_recv().is_err(), "skipped loads send nothing");
}
