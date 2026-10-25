use gitcomet_core::ref_comparison_memory::{ComparisonMemory, ComparisonReservation};
use std::sync::Arc;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const CONTROL_HEADROOM_BYTES: usize = 1024 * 1024;
const MINIMUM_JOB_BYTES: usize = 8 * 1024 * 1024;

pub(in super::super) struct JobMemory {
    pub buffer_bytes: usize,
    pub native: NativeMemory,
    _reservation: Arc<ComparisonReservation>,
}

impl JobMemory {
    pub fn reserve(owner: &Arc<ComparisonMemory>, requested: usize) -> Option<Self> {
        let bytes = requested.min(
            owner
                .available_bytes()
                .saturating_sub(CONTROL_HEADROOM_BYTES),
        );
        if bytes < MINIMUM_JOB_BYTES {
            return None;
        }
        let reservation = owner.reserve(bytes)?;
        let buffer_bytes = bytes / 4;
        Some(Self {
            buffer_bytes,
            native: NativeMemory {
                owner: owner.clone(),
                limit: bytes - buffer_bytes,
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
    #[cfg(test)]
    pub fn for_test(owner: Arc<ComparisonMemory>, limit: usize) -> Self {
        Self { owner, limit }
    }

    pub(super) fn monitor(&self, pid: u32) -> ProcessMemory<'_> {
        ProcessMemory {
            budget: self,
            system: System::new(),
            pid: Pid::from_u32(pid),
        }
    }
}

pub(super) struct ProcessMemory<'a> {
    budget: &'a NativeMemory,
    system: System,
    pid: Pid,
}

impl ProcessMemory<'_> {
    pub fn exceeded(&mut self) -> Option<bool> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_memory().without_tasks(),
        );
        let process = self.system.process(self.pid)?;
        #[cfg(windows)]
        let bytes = process.virtual_memory();
        #[cfg(not(windows))]
        let bytes = process.memory();
        let bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
        self.budget.owner.observe_native_bytes(bytes);
        Some(bytes > self.budget.limit)
    }
}
