use crate::util::{
    configure_git_process_tree, git_workdir_cmd_for, terminate_process_tree_and_wait,
};
use gitcomet_core::process::configure_background_command;
use gitcomet_core::ref_comparison::RefComparisonReason;
use gitcomet_core::services::CancellationToken;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) type AnalysisResult<T> = Result<T, RefComparisonReason>;

pub(super) struct Analysis<'a> {
    pub workdir: &'a Path,
    pub deadline: Instant,
    pub cancellation: &'a CancellationToken,
    pub memory: usize,
    pub empty_graph: Option<&'a Path>,
}

impl Analysis<'_> {
    pub fn check(&self) -> AnalysisResult<()> {
        if self.cancellation.is_cancelled() {
            return Err(RefComparisonReason::IncompleteHistory);
        }
        if Instant::now() >= self.deadline {
            return Err(RefComparisonReason::Deadline);
        }
        Ok(())
    }

    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = git_workdir_cmd_for(self.workdir);
        cmd.env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_COUNT", "0")
            .args(["--no-optional-locks", "-c", "core.pager=cat"])
            .args(args);
        if let Some(empty) = self.empty_graph {
            cmd.env("GIT_SHALLOW_FILE", empty)
                .env("GIT_GRAFT_FILE", empty);
        }
        cmd
    }

    pub fn capture_live_graph(&self, args: &[&str]) -> AnalysisResult<Vec<u8>> {
        let mut command = self.command(args);
        command
            .env_remove("GIT_SHALLOW_FILE")
            .env_remove("GIT_GRAFT_FILE");
        let limit = self.memory / 4;
        self.run(command, &[], move |reader| {
            let mut bytes = Vec::new();
            reader
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| RefComparisonReason::IncompleteHistory)?;
            if bytes.len() > limit {
                return Err(RefComparisonReason::Budget);
            }
            Ok(bytes)
        })
    }

    pub fn capture(&self, args: &[&str], input: &[u8]) -> AnalysisResult<Vec<u8>> {
        let limit = self.memory / 4;
        self.run(self.command(args), input, move |reader| {
            let mut bytes = Vec::new();
            reader
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| RefComparisonReason::IncompleteHistory)?;
            if bytes.len() > limit {
                return Err(RefComparisonReason::Budget);
            }
            Ok(bytes)
        })
    }

    pub fn run<T: Send>(
        &self,
        mut cmd: Command,
        input: &[u8],
        parse: impl FnOnce(BufReader<std::process::ChildStdout>) -> AnalysisResult<T> + Send,
    ) -> AnalysisResult<T> {
        self.check()?;
        configure_background_command(&mut cmd);
        configure_git_process_tree(&mut cmd);
        let allow_no_merge_base = cmd.get_args().any(|arg| arg == "merge-base");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd
            .spawn()
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        let mut child = ReapedChild {
            child,
            reaped: false,
        };
        let mut stdin = child
            .child
            .stdin
            .take()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let stdout = child
            .child
            .stdout
            .take()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let mut stderr = child
            .child
            .stderr
            .take()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let stopped = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let stop = &stopped;
            let writer = scope.spawn(move || stdin.write_all(input));
            let error_reader =
                scope.spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
            let reader = scope.spawn(move || {
                let result = parse(BufReader::new(stdout));
                if result.is_err() {
                    stop.store(true, Ordering::Release);
                }
                result
            });
            let mut failure = None;
            let success = loop {
                if let Err(reason) = self.check() {
                    failure = Some(reason);
                    let _ = terminate_process_tree_and_wait(&mut child.child);
                    break false;
                }
                if stopped.load(Ordering::Acquire) {
                    let _ = terminate_process_tree_and_wait(&mut child.child);
                    break false;
                }
                match child.child.try_wait() {
                    Ok(Some(status))
                        if reader.is_finished()
                            && writer.is_finished()
                            && error_reader.is_finished() =>
                    {
                        break status.success()
                            || (allow_no_merge_base && status.code() == Some(1));
                    }
                    Err(_) => break false,
                    _ => std::thread::sleep(Duration::from_millis(1)),
                }
            };
            let _ = child.child.wait();
            child.reaped = true;
            let _ = writer.join();
            let _ = error_reader.join();
            let parsed = reader
                .join()
                .map_err(|_| RefComparisonReason::IncompleteHistory)?;
            if let Some(reason) = failure {
                return Err(reason);
            }
            if !success {
                return Err(parsed
                    .err()
                    .unwrap_or(RefComparisonReason::IncompleteHistory));
            }
            parsed
        })
    }
}

struct ReapedChild {
    child: Child,
    reaped: bool,
}

impl Drop for ReapedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = terminate_process_tree_and_wait(&mut self.child);
        }
    }
}
