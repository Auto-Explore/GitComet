//! Read-only backend stage timings for startup and unchanged refreshes.
//! Usage: repository-load-probe REPO [--refreshes N] [--all-branches] [--remote-tags]
//!
//! Build with --release --features benchmarks. Stages run sequentially to
//! isolate their cost; their sum is NOT the application's concurrent launch
//! time. Each JSON line is emitted immediately so a stalled stage is visible.
//! Remote queries are opt-in. Command capture is thread-local, so commands a
//! stage delegates to child threads (remote tag queries) are included in its
//! elapsed time but not its command breakdown.
use gitcomet_core::domain::HistoryMode;
use gitcomet_core::services::{
    CancellationToken, GitBackend, HistoryReadRequest, HistoryReadResult,
};
use gitcomet_git_gix::GixBackend;
use serde_json::{Value, json};
use std::{io::Write as _, path::PathBuf, time::Instant};

fn emit(value: Value) {
    println!("{value}");
    std::io::stdout().flush().expect("flush stage timing");
}

fn measure<T>(
    iteration: usize,
    stage: &str,
    run: impl FnOnce() -> gitcomet_core::services::Result<T>,
    describe: impl FnOnce(&T) -> Value,
) -> Option<T> {
    emit(json!({"event": "start", "iteration": iteration, "stage": stage}));
    let start = Instant::now();
    let ((result, native_status), commands) =
        gitcomet_git_gix::command_trace::capture(|| gitcomet_git_gix::status_trace::capture(run));
    let elapsed = start.elapsed();
    let commands: Vec<_> = commands
        .iter()
        .map(|command| {
            json!({
                "label": command.label,
                "milliseconds": command.elapsed.as_secs_f64() * 1000.0,
                "stages": command.stages.iter().map(|(stage, duration)| {
                    json!({"stage": stage, "milliseconds": duration.as_secs_f64() * 1000.0})
                }).collect::<Vec<_>>()
            })
        })
        .collect();
    let native_status = json!({
        "stages": native_status.stages.iter().map(|stage| {
            json!({"stage": stage.label, "milliseconds": stage.elapsed.as_secs_f64() * 1000.0})
        }).collect::<Vec<_>>(),
        "walks": native_status.walks.iter().map(|walk| {
            let tracked = &walk.outcome.tracked_file_modification;
            json!({
                "windows_metadata_cache": cfg!(windows) && walk.filesystem_cache,
                "thread_limit": walk.thread_limit,
                "entries_to_process": tracked.entries_to_process,
                "entries_processed": tracked.entries_processed,
                "skipped_by_common_prefix": tracked.entries_skipped_by_common_prefix,
                "skipped_by_pathspec": tracked.entries_skipped_by_pathspec,
                "skipped_by_flags": tracked.entries_skipped_by_entry_flags,
                "live_metadata_calls": tracked.symlink_metadata_calls,
                "entries_to_update": tracked.entries_to_update,
                "racy_clean": tracked.racy_clean,
                "worktree_bytes": tracked.worktree_bytes,
                "worktree_files_read": tracked.worktree_files_read,
                "odb_bytes": tracked.odb_bytes,
                "odb_objects_read": tracked.odb_objects_read,
                "untracked_walk": walk.outcome.dirwalk.as_ref().map(|walk| {
                    json!({
                        "read_dir_calls": walk.read_dir_calls,
                        "seen_entries": walk.seen_entries,
                        "returned_entries": walk.returned_entries,
                    })
                }),
            })
        }).collect::<Vec<_>>(),
    });
    let (witness, error) = match &result {
        Ok(value) => (describe(value), None),
        Err(error) => (Value::Null, Some(error.to_string())),
    };
    emit(json!({
        "event": "finish", "iteration": iteration, "stage": stage,
        "milliseconds": elapsed.as_secs_f64() * 1000.0,
        "witness": witness, "error": error, "commands": commands,
        "native_status": native_status,
    }));
    result.ok()
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .expect("repository-load-probe REPO [--refreshes N] [--all-branches] [--remote-tags]"),
    );
    let mut refreshes = 2usize;
    let mut mode = HistoryMode::default();
    let mut remote_tags = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--refreshes") => {
                refreshes = args
                    .next()
                    .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
                    .filter(|count| *count <= 1000)
                    .expect("--refreshes requires a count from 0 to 1000");
            }
            Some("--all-branches") => mode = HistoryMode::AllBranches,
            Some("--remote-tags") => remote_tags = true,
            _ => panic!("unknown option: {arg:?}"),
        }
    }
    emit(json!({
        "event": "configuration", "debug_assertions": cfg!(debug_assertions),
        "refreshes": refreshes, "history_mode": format!("{mode:?}"),
        "remote_tags": remote_tags, "execution": "sequential_backend_stages",
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "available_parallelism": std::thread::available_parallelism().ok().map(usize::from),
        "native_status_profiling": true,
    }));
    let Some(repo) = measure(0, "open", || GixBackend.open(&root), |_| json!("opened")) else {
        std::process::exit(1);
    };
    let cancellation = CancellationToken::new();
    let mut history = HistoryReadRequest::Page {
        limit: 200,
        cursor: None,
        snapshot: None,
    };
    for iteration in 0..=refreshes {
        macro_rules! count {
            ($stage:literal, $call:expr) => {
                measure(iteration, $stage, || $call, |items| json!({"count": items.len()}));
            };
        }
        measure(
            iteration,
            "head",
            || repo.current_branch(&cancellation),
            |_| json!("read"),
        );
        measure(
            iteration,
            "upstream-divergence",
            || repo.upstream_divergence(&cancellation),
            |value| {
                json!(
                    value
                        .as_ref()
                        .map(|value| json!({"ahead": value.ahead, "behind": value.behind}))
                )
            },
        );
        let status = measure(
            iteration,
            "status",
            || repo.status(&cancellation),
            |status| json!({"staged": status.staged.len(), "unstaged": status.unstaged.len()}),
        );
        if let Some(status) = status {
            measure(
                iteration,
                "line-stats",
                || repo.uncommitted_line_stats_for_status(&status, &cancellation),
                |_| json!("read"),
            );
        }
        if let Some(HistoryReadResult::Page { page, snapshot }) = measure(
            iteration,
            "history",
            || repo.read_history(mode, None, &history, &cancellation, &mut |_| {}),
            |result| match result {
                HistoryReadResult::Page { page, .. } => json!({"commits": page.commits.len()}),
                HistoryReadResult::Unchanged => json!("unchanged"),
                HistoryReadResult::Invalidated => json!("invalidated"),
            },
        ) {
            history = HistoryReadRequest::Refresh {
                previous: page,
                snapshot,
            };
        }
        count!("branches", repo.list_branches(&cancellation));
        count!("remotes", repo.list_remotes(&cancellation));
        count!("remote-branches", repo.list_remote_branches(&cancellation));
        measure(
            iteration,
            "sequencer",
            || repo.sequencer_state(&cancellation),
            |_| json!("read"),
        );
        measure(
            iteration,
            "merge-message",
            || repo.merge_commit_message(&cancellation),
            |message| json!({"present": message.is_some()}),
        );
        count!("stashes", repo.stash_list(&cancellation));
        count!("worktrees", repo.list_worktrees(&cancellation));
        count!("submodules", repo.list_submodules(&cancellation));
        measure(
            iteration,
            "large-file-support",
            || repo.large_file_support(&cancellation),
            |support| json!({"lfs": support.lfs.in_use(), "annex": support.annex.in_use()}),
        );
        count!("tags", repo.list_tags(&cancellation));
        if remote_tags {
            count!("remote-tags", repo.list_remote_tags(&cancellation));
        }
    }
}
