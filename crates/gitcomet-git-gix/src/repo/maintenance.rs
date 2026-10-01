//! `git maintenance`: whether git recommends it, and running it with an
//! estimated progress meter.
use super::GixRepo;
use crate::util::{
    bytes_to_text_preserving_utf8, run_git_raw_output, run_git_with_output_and_timeout,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::git_operation::{self, GitOperationEvent};
use gitcomet_core::git_progress::GitProgressMeter;
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::process::{GitVersion, current_git_runtime};
use gitcomet_core::services::{CommandOutput, MAINTENANCE_CHECK_COMMAND, Result};
use rustc_hash::FxHashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// Repacking a large repository takes minutes to hours; Stop ends it sooner.
const MAINTENANCE_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// git's default `gc.autoPackLimit`.
const DEFAULT_AUTO_PACK_LIMIT: u64 = 50;

impl GixRepo {
    pub(crate) fn common_dir_impl(&self) -> PathBuf {
        canonicalize_or_original(self.repo().common_dir().to_path_buf())
    }

    /// Only gc's thresholds count: since git 2.54 the default strategy's own
    /// checks ask after ~500 loose objects or one stale worktree, expecting to
    /// run after every command, which GitComet turns off.
    pub(super) fn maintenance_needed_impl(&self) -> Result<bool> {
        // `is-needed` arrived in git 2.53; older git gets no recommendation.
        if current_git_runtime()
            .version()
            .is_none_or(|version| version < GitVersion::MAINTENANCE_IS_NEEDED)
        {
            return Ok(false);
        }
        // `git maintenance register` turns this off because a scheduler runs
        // maintenance. Read the user's value: git itself would see our `-c`.
        let config = self.repo_with_current_config()?;
        if config.config_snapshot().boolean("maintenance.auto") == Some(false) {
            return Ok(false);
        }
        Ok(self.is_needed(Some("gc"))?.status.code() == Some(0))
    }

    /// `git maintenance is-needed --auto`: exit 0 when needed, 1 when not.
    fn is_needed(&self, task: Option<&str>) -> Result<std::process::Output> {
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["maintenance", "is-needed", "--auto"]);
        if let Some(task) = task {
            cmd.arg(format!("--task={task}"));
        }
        run_git_raw_output(cmd, MAINTENANCE_CHECK_COMMAND)
    }

    pub(super) fn run_maintenance_impl(&self) -> Result<CommandOutput> {
        let objects = self.common_dir_impl().join("objects");
        // Under a held lock git skips every task and still exits 0.
        let lock = objects.join("maintenance.lock");
        if lock.exists() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "Another Git process is already running maintenance on this repository. If \
                 none is running, delete {} and try again.",
                lock.display()
            ))));
        }
        // The run prints nothing over a pipe, so only this check can tell a
        // run that did nothing; the recommendation may be hours old.
        let check = self.is_needed(None)?;
        if check.status.code() == Some(1) {
            return Ok(CommandOutput {
                command: MAINTENANCE_CHECK_COMMAND.to_string(),
                stdout: bytes_to_text_preserving_utf8(&check.stdout),
                stderr: bytes_to_text_preserving_utf8(&check.stderr),
                exit_code: Some(1),
            });
        }
        let pack_limit = self
            .repo_with_current_config()
            .ok()
            .and_then(|repo| repo.config_snapshot().integer("gc.autoPackLimit"))
            .and_then(|limit| u64::try_from(limit).ok())
            .unwrap_or(DEFAULT_AUTO_PACK_LIMIT);
        let started = SystemTime::now();
        let poller = ProgressPoller::start(&objects, pack_limit);
        let mut cmd = self.git_workdir_cmd();
        cmd.args(["maintenance", "run", "--auto", "--no-detach", "--no-quiet"]);
        let result =
            run_git_with_output_and_timeout(cmd, "git maintenance run --auto", MAINTENANCE_TIMEOUT);
        drop(poller);
        if let Err(error) = &result
            && was_killed(error.kind())
        {
            remove_own_maintenance_lock(&objects, started);
        }
        result
    }
}

fn was_killed(kind: &ErrorKind) -> bool {
    match kind {
        ErrorKind::Cancelled => true,
        ErrorKind::Git(failure) => failure.id() == gitcomet_core::error::GitFailureId::Timeout,
        _ => false,
    }
}

/// A killed run (Stop, timeout) skips git's lockfile cleanup, and a stale
/// `maintenance.lock` makes every later run exit 0 without doing anything.
/// Only a lock taken since this run started can be ours.
fn remove_own_maintenance_lock(objects: &Path, started: SystemTime) {
    let lock = objects.join("maintenance.lock");
    let ours = std::fs::metadata(&lock)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified >= started);
    if ours {
        let _ = std::fs::remove_file(lock);
    }
}

/// Emits an estimated meter while the maintenance git runs: repack prints no
/// percentage unless stderr is a terminal, but the pack it writes grows on disk.
struct ProgressPoller {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ProgressPoller {
    fn start(objects: &Path, pack_limit: u64) -> Option<Self> {
        let context = git_operation::current()?;
        let baseline = PackBaseline::read(objects, pack_limit);
        let pack_dir = objects.join("pack");
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("gitcomet-maintenance-progress".to_string())
            .spawn({
                let stop = Arc::clone(&stop);
                move || {
                    let mut last = None;
                    while !stop.load(Ordering::Acquire) {
                        let meter = baseline.meter(&pack_dir);
                        if last.as_ref() != Some(&meter) {
                            context.emit(GitOperationEvent::Progress(meter.clone()));
                            last = Some(meter);
                        }
                        std::thread::park_timeout(POLL_INTERVAL);
                    }
                }
            })
            .ok()?;
        Some(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for ProgressPoller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

/// The packs present when maintenance started and how much it should write.
struct PackBaseline {
    packs: FxHashSet<OsString>,
    expected_bytes: u64,
}

impl PackBaseline {
    /// gc repacks everything when there are more packs than
    /// `gc.autoPackLimit`, otherwise just the loose objects.
    fn read(objects: &Path, pack_limit: u64) -> Self {
        let mut packs = FxHashSet::default();
        let mut repackable = 0u64;
        let mut repackable_bytes = 0u64;
        for entry in std::fs::read_dir(objects.join("pack"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = entry.file_name();
            let path = entry.path();
            if !is_pack_file(&name) {
                continue;
            }
            if !path.with_extension("keep").exists() {
                repackable += 1;
                repackable_bytes += entry.metadata().map_or(0, |metadata| metadata.len());
            }
            packs.insert(name);
        }
        let loose_bytes = sampled_loose_bytes(objects);
        let full = repackable > pack_limit;
        Self {
            packs,
            expected_bytes: (if full { repackable_bytes } else { 0 } + loose_bytes).max(1),
        }
    }

    fn meter(&self, pack_dir: &Path) -> GitProgressMeter {
        let mut written = 0u64;
        let mut writing = false;
        for entry in std::fs::read_dir(pack_dir).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let text = name.to_string_lossy();
            let temporary = text.starts_with("tmp_pack_")
                || (text.starts_with(".tmp-") && text.ends_with(".pack"));
            let new_pack = is_pack_file(&name) && !self.packs.contains(&name);
            if temporary || new_pack {
                writing |= temporary;
                written += entry.metadata().map_or(0, |metadata| metadata.len());
            }
        }
        if writing {
            let percent = (written.saturating_mul(100) / self.expected_bytes).min(99);
            GitProgressMeter::estimated("Writing new pack", percent as u8)
        } else if written > 0 {
            GitProgressMeter {
                title: Arc::from("Removing old packs"),
                percent: None,
                estimated: true,
            }
        } else {
            GitProgressMeter {
                title: Arc::from("Preparing"),
                percent: None,
                estimated: true,
            }
        }
    }
}

fn is_pack_file(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with("pack-") && name.ends_with(".pack")
}

/// Loose object bytes, estimated from one of the 256 fan-out directories the
/// way git estimates their count.
fn sampled_loose_bytes(objects: &Path) -> u64 {
    std::fs::read_dir(objects.join("17"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            matches!(name.len(), 38 | 62) && name.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .map(|entry| entry.metadata().map_or(0, |metadata| metadata.len()))
        .sum::<u64>()
        .saturating_mul(256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_follows_the_new_pack_against_the_expected_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let objects = dir.path();
        let pack_dir = objects.join("pack");
        std::fs::create_dir_all(&pack_dir).expect("pack dir");
        for index in 0..3 {
            std::fs::write(pack_dir.join(format!("pack-{index}.pack")), vec![0u8; 1000])
                .expect("old pack");
        }
        std::fs::write(pack_dir.join("pack-2.keep"), b"").expect("keep file");
        let baseline = PackBaseline::read(objects, 1);
        assert_eq!(baseline.expected_bytes, 2000, "kept packs are not repacked");
        assert_eq!(&*baseline.meter(&pack_dir).title, "Preparing");

        std::fs::write(pack_dir.join("tmp_pack_ab12"), vec![0u8; 500]).expect("tmp pack");
        let meter = baseline.meter(&pack_dir);
        assert_eq!(
            (&*meter.title, meter.percent),
            ("Writing new pack", Some(25))
        );
        assert!(meter.estimated);

        std::fs::remove_file(pack_dir.join("tmp_pack_ab12")).expect("finish tmp");
        std::fs::write(pack_dir.join("pack-new.pack"), vec![0u8; 1900]).expect("new pack");
        assert_eq!(&*baseline.meter(&pack_dir).title, "Removing old packs");
    }

    #[test]
    fn below_the_pack_limit_only_loose_objects_are_expected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let objects = dir.path();
        std::fs::create_dir_all(objects.join("pack")).expect("pack dir");
        std::fs::write(objects.join("pack/pack-0.pack"), vec![0u8; 1000]).expect("pack");
        std::fs::create_dir_all(objects.join("17")).expect("fan-out dir");
        std::fs::write(objects.join("17").join("a".repeat(38)), vec![0u8; 10]).expect("loose");
        std::fs::write(objects.join("17/not-an-object"), vec![0u8; 99]).expect("junk");

        assert_eq!(PackBaseline::read(objects, 50).expected_bytes, 10 * 256);
    }

    #[test]
    fn only_a_lock_taken_during_the_run_is_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = dir.path().join("maintenance.lock");
        std::fs::write(&lock, b"").expect("lock");

        remove_own_maintenance_lock(dir.path(), SystemTime::now() + Duration::from_secs(60));
        assert!(
            lock.exists(),
            "a lock older than the run belongs to someone else"
        );

        remove_own_maintenance_lock(dir.path(), SystemTime::UNIX_EPOCH);
        assert!(!lock.exists());
    }
}
