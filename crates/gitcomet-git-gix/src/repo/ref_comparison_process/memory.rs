use gitcomet_core::ref_comparison_memory::{ComparisonMemory, ComparisonReservation};
use std::sync::Arc;
#[cfg(not(target_os = "linux"))]
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const CONTROL_HEADROOM_BYTES: usize = 1024 * 1024;
const MINIMUM_BUFFER_BYTES: usize = 2 * 1024 * 1024;
const MIB: usize = 1024 * 1024;
/// Observed walks use about 150 bytes of private memory per commit.
const NATIVE_BYTES_PER_COMMIT: usize = 256;
const MINIMUM_NATIVE_BYTES: usize = 128 * MIB;
const MAXIMUM_NATIVE_BYTES: usize = 1024 * MIB;

/// Private-memory limit for one Git process, sized by the repository's
/// commit count. Unknown sizes get the maximum.
pub(in super::super) fn native_limit(commits: Option<u64>) -> usize {
    commits.map_or(MAXIMUM_NATIVE_BYTES, |commits| {
        usize::try_from(commits)
            .unwrap_or(usize::MAX)
            .saturating_mul(NATIVE_BYTES_PER_COMMIT)
            .clamp(MINIMUM_NATIVE_BYTES, MAXIMUM_NATIVE_BYTES)
    })
}

pub(in super::super) struct JobMemory {
    pub buffer_bytes: usize,
    pub native: NativeMemory,
    _reservation: Arc<ComparisonReservation>,
}

impl JobMemory {
    /// Reserves `requested` bytes of application buffers. Git processes are
    /// bounded by `native_limit` instead of the shared reservation.
    pub fn reserve(
        owner: &Arc<ComparisonMemory>,
        requested: usize,
        native_limit: usize,
    ) -> Option<Self> {
        let bytes = requested.min(
            owner
                .available_bytes()
                .saturating_sub(CONTROL_HEADROOM_BYTES),
        );
        if bytes < MINIMUM_BUFFER_BYTES {
            return None;
        }
        let reservation = owner.reserve(bytes)?;
        Some(Self {
            buffer_bytes: bytes,
            native: NativeMemory {
                owner: owner.clone(),
                limit: native_limit,
            },
            _reservation: reservation,
        })
    }
}

pub(in super::super) struct NativeMemory {
    owner: Arc<ComparisonMemory>,
    limit: usize,
}

impl NativeMemory {
    #[cfg(all(test, unix))]
    pub fn for_test(owner: Arc<ComparisonMemory>, limit: usize) -> Self {
        Self { owner, limit }
    }

    pub(super) fn monitor(&self, pid: u32) -> ProcessMemory<'_> {
        ProcessMemory {
            budget: self,
            previous: 0,
            #[cfg(not(target_os = "linux"))]
            system: System::new(),
            #[cfg(target_os = "linux")]
            status: format!("/proc/{pid}/status").into(),
            #[cfg(target_os = "linux")]
            exe: format!("/proc/{pid}/exe").into(),
            #[cfg(target_os = "linux")]
            own_exe: std::fs::read_link("/proc/self/exe").ok(),
            #[cfg(not(target_os = "linux"))]
            pid: Pid::from_u32(pid),
        }
    }
}

pub(super) struct ProcessMemory<'a> {
    budget: &'a NativeMemory,
    previous: usize,
    #[cfg(not(target_os = "linux"))]
    system: System,
    #[cfg(target_os = "linux")]
    status: std::path::PathBuf,
    #[cfg(target_os = "linux")]
    exe: std::path::PathBuf,
    #[cfg(target_os = "linux")]
    own_exe: Option<std::path::PathBuf>,
    #[cfg(not(target_os = "linux"))]
    pid: Pid,
}

impl ProcessMemory<'_> {
    pub fn exceeded(&mut self) -> Option<bool> {
        let bytes = self.private_bytes()?;
        let bytes = sustained(
            &mut self.previous,
            usize::try_from(bytes).unwrap_or(usize::MAX),
        );
        self.budget.owner.observe_native_bytes(bytes);
        Some(bytes > self.budget.limit)
    }

    /// Anonymous memory only: mapped packs, commit-graphs and binaries are
    /// page cache the kernel can reclaim.
    #[cfg(target_os = "linux")]
    fn private_bytes(&mut self) -> Option<u64> {
        // The executable belongs to the memory map. Until exec swaps the map,
        // the child still reports ours; read it before the status so a
        // finished swap can only show the child's own numbers.
        if self.own_exe.is_some() && std::fs::read_link(&self.exe).ok() == self.own_exe {
            return Some(0);
        }
        parse_rss_anon(&std::fs::read_to_string(&self.status).ok()?)
    }

    /// Windows reports private commit; other systems only resident memory.
    #[cfg(not(target_os = "linux"))]
    fn private_bytes(&mut self) -> Option<u64> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_memory().without_tasks(),
        );
        let process = self.system.process(self.pid)?;
        #[cfg(windows)]
        return Some(process.virtual_memory());
        #[cfg(not(windows))]
        return Some(process.memory());
    }
}

/// The smaller of this and the previous sample. Spawn returns while the
/// child can still report the parent's memory map (the kernel releases a
/// vfork parent before exec swaps the map), which Linux detects exactly and
/// other systems cannot, so one sample proves nothing.
fn sustained(previous: &mut usize, bytes: usize) -> usize {
    std::mem::replace(previous, bytes).min(bytes)
}

/// `RssAnon` from `/proc/<pid>/status`, in bytes. An exiting process drops
/// its memory map, and with it the line, before it becomes waitable.
#[cfg(any(target_os = "linux", test))]
fn parse_rss_anon(status: &str) -> Option<u64> {
    let Some(value) = status
        .lines()
        .find_map(|line| line.strip_prefix("RssAnon:"))
    else {
        return Some(0);
    };
    let kib = value
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse::<u64>()
        .ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_high_sample_is_not_sustained() {
        let mut previous = 0;
        assert_eq!(sustained(&mut previous, 900 * MIB), 0);
        assert_eq!(sustained(&mut previous, MIB), MIB);
        assert_eq!(sustained(&mut previous, 300 * MIB), MIB);
        assert_eq!(sustained(&mut previous, 400 * MIB), 300 * MIB);
    }

    #[test]
    fn rss_anon_is_read_in_bytes() {
        let status =
            "Name:\tgit\nVmRSS:\t  190000 kB\nRssAnon:\t    5320 kB\nRssFile:\t  184680 kB\n";
        assert_eq!(parse_rss_anon(status), Some(5320 * 1024));
        assert_eq!(parse_rss_anon("Name:\tgit\nState:\tR (running)\n"), Some(0));
        assert_eq!(parse_rss_anon("RssAnon:\tlots kB\n"), None);
    }

    /// Two workers used to reserve all but 1 MiB of the repository budget
    /// for their Git processes, so decoration pages and publications failed
    /// and deferred every row.
    #[test]
    fn running_jobs_leave_room_for_published_results() {
        let owner = Arc::new(ComparisonMemory::default());
        let budget = gitcomet_core::ref_comparison::RefComparisonBudget::automatic().memory_bytes;
        let _first = JobMemory::reserve(&owner, budget, native_limit(None)).unwrap();
        let _second = JobMemory::reserve(&owner, budget, native_limit(None)).unwrap();
        assert!(owner.reserve(8 * MIB).is_some());
    }

    #[test]
    fn native_limit_scales_with_commits_within_bounds() {
        assert_eq!(native_limit(Some(10)), MINIMUM_NATIVE_BYTES);
        assert_eq!(
            native_limit(Some(2_000_000)),
            2_000_000 * NATIVE_BYTES_PER_COMMIT
        );
        assert_eq!(native_limit(Some(u64::MAX)), MAXIMUM_NATIVE_BYTES);
        assert_eq!(native_limit(None), MAXIMUM_NATIVE_BYTES);
    }
}
