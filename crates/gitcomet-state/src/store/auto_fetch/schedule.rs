//! Per-group scheduling state: when a group fetches next, which remotes it
//! skips, and which hosts are waiting out a failure.

use super::policy;
use crate::model::{AutoFetchInterval, RepoAutoFetchState};
use gitcomet_core::error::Error;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Worktrees that share an object store and resolve the same fetch
/// configuration update the same refs, so one fetch serves all of them.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum GroupKey {
    /// A workdir no fetch has opened yet.
    Workdir(PathBuf),
    Store {
        common_dir: PathBuf,
        config: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FetchRemote {
    pub(super) name: String,
    /// `None` for a repository on this machine.
    pub(super) host: Option<String>,
}

pub(super) struct Group {
    pub(super) schedule: Schedule,
    pub(super) snapshot: RepoAutoFetchState,
    pub(super) revision: u64,
    pub(super) request: Option<u64>,
    pub(super) order: u64,
    /// The remotes this group's last fetch saw.
    pub(super) remotes: Vec<FetchRemote>,
    /// Skipped until a manual fetch succeeds: retrying cannot fix them.
    pub(super) paused: BTreeSet<String>,
}

impl Group {
    pub(super) fn new(revision: u64, order: u64) -> Self {
        Self {
            schedule: Schedule::default(),
            snapshot: RepoAutoFetchState::default(),
            revision,
            request: None,
            order,
            remotes: Vec::new(),
            paused: BTreeSet::new(),
        }
    }

    /// A fetch succeeded: the hosts it contacted answer again.
    pub(super) fn reached(&mut self, hosts: &mut BTreeMap<String, HostBlock>, skipped: &[String]) {
        self.schedule.failures = 0;
        self.snapshot.retry_at = None;
        for host in contacted(&self.remotes, skipped).filter_map(|remote| remote.host.as_ref()) {
            hosts.remove(host);
        }
    }

    /// Charges a failure to the remotes the output names, or to every remote
    /// the fetch contacted when it names none.
    pub(super) fn failed(
        &mut self,
        hosts: &mut BTreeMap<String, HostBlock>,
        error: &Error,
        skipped: &[String],
        interval: AutoFetchInterval,
        now: SystemTime,
    ) {
        let Some(failure) = policy::classify(error) else {
            return;
        };
        let named = policy::failed_remotes(error);
        let mut failed: Vec<&FetchRemote> = contacted(&self.remotes, skipped).collect();
        if failed.iter().any(|remote| named.contains(&remote.name)) {
            failed.retain(|remote| named.contains(&remote.name));
        }
        if failure.blocks_host() {
            for host in failed.iter().filter_map(|remote| remote.host.as_ref()) {
                let block = hosts.entry(host.clone()).or_insert(HostBlock {
                    until: None,
                    strikes: 0,
                });
                block.strikes += 1;
                block.until = policy::host_block(failure, block.strikes).map(|wait| now + wait);
            }
        }
        if failure.pauses() {
            let failed: Vec<String> = failed.iter().map(|remote| remote.name.clone()).collect();
            self.paused.extend(failed);
            self.snapshot.paused_remotes = self.paused.iter().cloned().collect();
        }
        self.schedule.failures += 1;
        self.snapshot.retry_at = self.schedule.due_at(interval);
    }

    /// Clears failure state after a fetch that reached every remote.
    pub(super) fn recover(&mut self) {
        self.schedule.failures = 0;
        self.schedule.quiet = 0;
        self.paused.clear();
        self.snapshot.retry_at = None;
        self.snapshot.paused_remotes.clear();
    }
}

/// A host that failed to sign in, answer or serve. No automatic fetch
/// contacts it until `until`, or until a manual fetch reaches it when `None`;
/// the first one after that probes it.
pub(super) struct HostBlock {
    pub(super) until: Option<SystemTime>,
    pub(super) strikes: u32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Schedule {
    pub(super) completed: Option<SystemTime>,
    /// Consecutive failed fetches; each one stretches the next wait.
    pub(super) failures: u32,
    /// Consecutive fetches of an inactive group that moved nothing.
    pub(super) quiet: u32,
    /// Launch grace: no fetch before this, whatever `completed` says.
    pub(super) not_before: Option<SystemTime>,
}

impl Schedule {
    pub(super) fn wait(&self, interval: AutoFetchInterval) -> Duration {
        if self.failures > 0 {
            policy::retry_delay(interval, self.failures)
        } else {
            policy::quiet_delay(interval, self.quiet)
        }
    }

    pub(super) fn is_due(&self, interval: AutoFetchInterval, now: SystemTime) -> bool {
        self.not_before.is_none_or(|not_before| now >= not_before)
            && self.completed.is_none_or(|completed| {
                now.duration_since(completed)
                    .map_or(true, |elapsed| elapsed >= self.wait(interval))
            })
    }

    pub(super) fn due_at(&self, interval: AutoFetchInterval) -> Option<SystemTime> {
        let due = self.completed?.checked_add(self.wait(interval))?;
        Some(
            self.not_before
                .map_or(due, |not_before| due.max(not_before)),
        )
    }

    pub(super) fn finish(&mut self, now: SystemTime) {
        self.completed = Some(now);
    }
}

/// The remotes a fetch that leaves out `skipped` contacts.
pub(super) fn contacted<'a>(
    remotes: &'a [FetchRemote],
    skipped: &'a [String],
) -> impl Iterator<Item = &'a FetchRemote> {
    remotes
        .iter()
        .filter(|remote| !skipped.contains(&remote.name))
}
