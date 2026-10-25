use gitcomet_core::domain::{CommitId, RepoSpec};
use gitcomet_core::ref_comparison::{RefComparisonCounts, RefIntegration};
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use gitcomet_state::model::{AppState, LoadKind, Loadable, RepoId, RepoState};
use gitcomet_state::msg::Msg;
use gitcomet_state::ref_comparisons::{RefComparisonsMsg, RefComparisonsState};
use gitcomet_state::store::AppStore;
use serde_json::{Value, json};
use std::io::{Error, ErrorKind, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const PROBE_CHILD: &str = "GITCOMET_COMPARISON_PROBE_CHILD";
const FULL_TIMEOUT: Duration = Duration::from_secs(1200);

fn emit(value: Value) {
    println!("{value}");
    std::io::stdout()
        .flush()
        .expect("flush comparison measurement");
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let null = if cfg!(windows) { "nul" } else { "/dev/null" };
    let git_environment_isolated = std::env::var("GIT_CONFIG_NOSYSTEM").as_deref() == Ok("1")
        && std::env::var("GIT_CONFIG_GLOBAL").as_deref() == Ok(null)
        && std::env::vars_os().all(|(name, _)| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            !name.starts_with("GIT_")
                || matches!(name.as_str(), "GIT_CONFIG_NOSYSTEM" | "GIT_CONFIG_GLOBAL")
        });
    if std::env::var_os(PROBE_CHILD).is_none() || !git_environment_isolated {
        let isolated = tempfile::tempdir()?;
        let mut command = Command::new(std::env::current_exe()?);
        for (name, _) in std::env::vars_os() {
            if name
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(name);
            }
        }
        let status = command
            .args(std::env::args_os().skip(1))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", null)
            .env(PROBE_CHILD, "1")
            .env("GITCOMET_DISABLE_SESSION_PERSIST", "1")
            .env("GITCOMET_PROFILE_ROOT", isolated.path())
            .env_remove("GITCOMET_SESSION_FILE")
            .env("XDG_CONFIG_HOME", isolated.path().join("config"))
            .env("XDG_STATE_HOME", isolated.path().join("state"))
            .env("XDG_DATA_HOME", isolated.path().join("data"))
            .env("APPDATA", isolated.path().join("config"))
            .env("LOCALAPPDATA", isolated.path().join("state"))
            .status()?;
        drop(isolated);
        std::process::exit(status.code().unwrap_or(1));
    }
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "usage: comparison-probe REPO [--deep]",
        )
    })?;
    let deep = match (args.next(), args.next()) {
        (None, None) => false,
        (Some(flag), None) if flag == "--deep" => true,
        _ => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "usage: comparison-probe REPO [--deep]",
            )
            .into());
        }
    };
    let path = PathBuf::from(path).canonicalize()?;
    let listing = Instant::now();
    let backend = Arc::new(GixBackend);
    let reader = backend.open(&path)?;
    let cancellation = CancellationToken::new();
    let memory = reader
        .ref_comparison_memory()
        .ok_or_else(|| Error::other("backend has no comparison memory authority"))?;
    let id = RepoId(1);
    let mut repo = RepoState::new_opening(
        id,
        RepoSpec {
            workdir: path.clone(),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready(reader.current_branch(&cancellation)?);
    repo.detached_head_commit = if matches!(&repo.head_branch, Loadable::Ready(name) if name == "HEAD")
    {
        reader.head_commit_id()?
    } else {
        None
    };
    repo.branches = Loadable::Ready(Arc::new(reader.list_branches(&cancellation)?));
    repo.remotes = Loadable::Ready(Arc::new(reader.list_remotes(&cancellation)?));
    repo.remote_branches = Loadable::Ready(Arc::new(reader.list_remote_branches(&cancellation)?));
    repo.worktrees = Loadable::Ready(Arc::new(reader.list_worktrees(&cancellation)?));
    repo.ref_comparisons = RefComparisonsState::for_benchmarks(memory.clone());
    let mut visible = Vec::new();
    if let Some(head) = repo.head_commit_id() {
        visible.push(head);
    }
    for branch in repo.branches.ready().expect("captured branches").iter() {
        if visible.len() == 32 {
            break;
        }
        if !visible.contains(&branch.target) {
            visible.push(branch.target.clone());
        }
    }
    emit(
        json!({ "event": "inputs", "milliseconds": listing.elapsed().as_secs_f64() * 1000.0,
        "local_branches": repo.branches.ready().map_or(0, |rows| rows.len()),
        "worktrees": repo.worktrees.ready().map_or(0, |rows| rows.len()), "deep": deep }),
    );
    let (store, events) = AppStore::new_test(backend);
    store.disable_repo_monitors_for_test();
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(id),
        ..AppState::test_default()
    }));
    store.insert_repo_for_test(id, reader);
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(30))?;
    let started = Instant::now();
    store.dispatch(Msg::RefComparisons(RefComparisonsMsg::Prioritize {
        repo_id: id,
        subjects: visible.clone(),
    }));
    store.dispatch(Msg::RefComparisons(if deep {
        RefComparisonsMsg::Deeper { repo_id: id }
    } else {
        RefComparisonsMsg::Ensure { repo_id: id }
    }));
    let mut first_visible = None;
    let mut first_exact = None;
    let mut progress_at = Instant::now();
    let cold_settled;
    loop {
        while events.try_recv().is_ok() {}
        let snapshot = store.snapshot();
        let repo = &snapshot.repos[0];
        let comparisons = &repo.ref_comparisons;
        let mut exact = 0;
        let mut confirmed = 0;
        let mut pending = 0;
        let mut unavailable_counts = 0;
        let mut not_confirmed = 0;
        for subject in &comparisons.projections.subjects {
            match comparisons.decorations.get(subject) {
                Some(result) => {
                    exact +=
                        usize::from(matches!(result.counts, RefComparisonCounts::Exact { .. }));
                    confirmed +=
                        usize::from(matches!(result.integration, RefIntegration::Confirmed(_)));
                    unavailable_counts +=
                        usize::from(matches!(result.counts, RefComparisonCounts::Unavailable(_)));
                    not_confirmed += usize::from(matches!(
                        result.integration,
                        RefIntegration::NotConfirmed(_)
                    ));
                    pending += usize::from(
                        result.counts == RefComparisonCounts::Pending
                            || result.integration == RefIntegration::Pending,
                    );
                }
                None => pending += 1,
            }
        }
        for subject in &visible {
            if let Some(result) = comparisons.decorations.get(subject) {
                if result.counts != RefComparisonCounts::Pending {
                    first_visible.get_or_insert(started.elapsed());
                }
                if matches!(result.counts, RefComparisonCounts::Exact { .. }) {
                    first_exact.get_or_insert(started.elapsed());
                }
            }
        }
        let target_error = if let Loadable::Error(error) = &comparisons.target {
            Some(error.clone())
        } else {
            None
        };
        let done = target_error.is_some()
            || comparisons.deferred.is_some()
            || (matches!(comparisons.target, Loadable::Ready(_)) && pending == 0);
        if done || progress_at.elapsed() >= Duration::from_secs(5) {
            emit(
                json!({ "event": if done { "cold_complete" } else { "progress" },
                "milliseconds": started.elapsed().as_secs_f64() * 1000.0,
                "first_visible_ms": first_visible.map(|duration| duration.as_secs_f64() * 1000.0),
                "first_exact_ms": first_exact.map(|duration| duration.as_secs_f64() * 1000.0),
                "subjects": comparisons.projections.subjects.len(), "exact": exact, "confirmed": confirmed, "pending": pending,
                "unavailable_counts": unavailable_counts, "not_confirmed": not_confirmed,
                "jobs": comparisons.metrics.jobs, "completed_graph_rows": comparisons.metrics.graph_reads,
                "cache_hits": comparisons.metrics.cache_hits, "publications": comparisons.metrics.publications,
                "accounted_bytes": memory.used_bytes(), "accounted_peak_bytes": memory.peak_bytes(),
                "supervised_git_peak_bytes": memory.native_peak_bytes(),
                "deferred": comparisons.deferred.map(|reason| reason.description()), "target_error": target_error }),
            );
            progress_at = Instant::now();
        }
        if done {
            cold_settled = matches!(comparisons.target, Loadable::Ready(_))
                && comparisons.deferred.is_none()
                && pending == 0;
            break;
        }
        if started.elapsed() >= FULL_TIMEOUT {
            return Err(Error::new(
                ErrorKind::TimedOut,
                "comparison full-completion probe timed out",
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let cold = store.snapshot();
    let projections = cold.repos[0].ref_comparisons.projections.clone();
    let metrics = cold.repos[0].ref_comparisons.metrics;
    let mut oracles = 0;
    if let Loadable::Ready(target) = &cold.repos[0].ref_comparisons.target {
        for subject in projections
            .subjects
            .iter()
            .step_by((projections.subjects.len() / 8).max(1))
            .take(8)
        {
            if let Some(result) = cold.repos[0].ref_comparisons.decorations.get(subject)
                && let RefComparisonCounts::Exact { ahead, behind } = result.counts
            {
                validate_counts(&path, subject, &target.oid, ahead, behind)?;
                oracles += 1;
            }
        }
    }
    emit(json!({ "event": "oracles", "validated": oracles }));
    drop(cold);
    if cold_settled {
        let warm = Instant::now();
        store.dispatch(Msg::RefreshBranches { repo_id: id });
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(30))?;
        loop {
            while events.try_recv().is_ok() {}
            let snapshot = store.snapshot();
            let repo = &snapshot.repos[0];
            let loading = [
                LoadKind::Branches,
                LoadKind::RemoteBranches,
                LoadKind::Remotes,
            ]
            .iter()
            .any(|kind| repo.loads_in_flight.is_in_flight(*kind));
            if !loading && !matches!(repo.ref_comparisons.target, Loadable::Loading) {
                emit(
                    json!({ "event": if matches!(repo.branches, Loadable::Ready(_)) && matches!(repo.ref_comparisons.target, Loadable::Ready(_)) { "warm_branch_complete" } else { "warm_branch_unavailable" }, "scope": "local_branches", "milliseconds": warm.elapsed().as_secs_f64() * 1000.0,
                "new_jobs": repo.ref_comparisons.metrics.jobs.saturating_sub(metrics.jobs),
                "new_graph_rows": repo.ref_comparisons.metrics.graph_reads.saturating_sub(metrics.graph_reads),
                "new_publications": repo.ref_comparisons.metrics.publications.saturating_sub(metrics.publications),
                "projections_reused": Arc::ptr_eq(&projections, &repo.ref_comparisons.projections) }),
                );
                break;
            }
            if warm.elapsed() >= Duration::from_secs(30) {
                return Err(
                    Error::new(ErrorKind::TimedOut, "warm comparison refresh timed out").into(),
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    } else {
        emit(
            json!({"event": "warm_skipped", "scope": "local_branches", "reason": "The cold comparison did not settle with an available target."}),
        );
    }
    drop(projections);
    let release = Instant::now();
    store.dispatch(Msg::MoveRepoOut { repo_id: id });
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(30))?;
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(45))?;
    drop(store);
    while memory.used_bytes() != 0 && release.elapsed() < Duration::from_secs(45) {
        std::thread::sleep(Duration::from_millis(5));
    }
    emit(
        json!({ "event": "released", "milliseconds": release.elapsed().as_secs_f64() * 1000.0, "accounted_bytes": memory.used_bytes(), "accounted_peak_bytes": memory.peak_bytes() }),
    );
    if memory.used_bytes() != 0 {
        return Err(Error::other("comparison memory remained owned after closure").into());
    }
    Ok(())
}

fn validate_counts(
    path: &std::path::Path,
    subject: &CommitId,
    target: &CommitId,
    ahead: u64,
    behind: u64,
) -> Result<()> {
    let output = gitcomet_core::process::git_command()
        .arg("-C")
        .arg(path)
        .args(["--no-optional-locks", "rev-list", "--left-right", "--count"])
        .arg(format!("{subject}...{target}"))
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .output()?;
    if !output.status.success() {
        return Err(Error::other("independent immutable-OID count oracle failed").into());
    }
    let text = String::from_utf8(output.stdout)?;
    let counts: Vec<u64> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    if counts != [ahead, behind] {
        return Err(Error::other(format!(
            "independent counts differ for {subject}: {counts:?} versus {ahead}/{behind}"
        ))
        .into());
    }
    Ok(())
}
