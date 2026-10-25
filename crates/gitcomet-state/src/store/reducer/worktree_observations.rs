use crate::model::{RepoState, WorktreeDirtyScope};
use gitcomet_core::domain::WorktreeDirtySummary;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::worktree_scan::{
    ObservedWorktreeStatus, WorktreeCleanliness, WorktreeObservation, WorktreeScan,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn apply(
    repo: &mut RepoState,
    scope: &WorktreeDirtyScope,
    scan: WorktreeScan,
) -> Option<Result<Vec<WorktreeDirtySummary>, Error>> {
    if scan.id < repo.worktree_scan_id {
        return None;
    }
    let observations = match scan.outcome {
        Ok(observations) => observations,
        Err(error) => {
            mark_unknown(repo, scope, scan.id, &error);
            return Some(Err(error));
        }
    };
    let mut seen = FxHashSet::default();
    if observations.iter().any(|observation| {
        !scope.includes(observation.path()) || !seen.insert(observation.path().to_path_buf())
    }) || matches!(scope, WorktreeDirtyScope::Paths(paths) if paths.iter().any(|path| !seen.contains(path)))
    {
        let error = Error::new(ErrorKind::Backend(
            "Invalid worktree scan observations.".into(),
        ));
        mark_unknown(repo, scope, scan.id, &error);
        return Some(Err(error));
    }
    let mut dirty: FxHashMap<PathBuf, WorktreeDirtySummary> = repo
        .worktree_dirty
        .ready()
        .into_iter()
        .flat_map(|items| items.iter())
        .map(|summary| (summary.path.clone(), summary.clone()))
        .collect();
    let mut order: Vec<PathBuf> = repo
        .worktree_dirty
        .ready()
        .into_iter()
        .flat_map(|items| items.iter().map(|summary| summary.path.clone()))
        .collect();
    let mut changed = false;
    let statuses = Arc::make_mut(&mut repo.worktree_observations);
    if matches!(scope, WorktreeDirtyScope::All) {
        repo.worktree_scan_id = scan.id;
        dirty.retain(|path, _| {
            seen.contains(path)
                || statuses
                    .get(path)
                    .is_some_and(|status| status.scan_id > scan.id)
        });
        statuses.retain(|path, status| {
            let keep = seen.contains(path) || status.scan_id > scan.id;
            changed |= !keep;
            keep
        });
    }
    for observation in observations {
        let path = observation.path().to_path_buf();
        if statuses
            .get(&path)
            .is_some_and(|status| status.scan_id > scan.id)
        {
            continue;
        }
        let next = ObservedWorktreeStatus {
            scan_id: scan.id,
            head: observation.head().cloned(),
            cleanliness: observation.cleanliness(),
        };
        changed |= statuses.get(&path).is_none_or(|previous| {
            previous.head != next.head || previous.cleanliness != next.cleanliness
        });
        statuses.insert(path.clone(), next);
        match observation {
            WorktreeObservation::Clean(_) => {
                dirty.remove(&path);
            }
            WorktreeObservation::Dirty(summary) => {
                if !dirty.contains_key(&path) {
                    order.push(path.clone());
                }
                dirty.insert(path, summary);
            }
            WorktreeObservation::Unknown { .. } => {}
        }
    }
    if changed {
        repo.worktree_observations_rev = repo.worktree_observations_rev.wrapping_add(1);
    }
    Some(Ok(order
        .into_iter()
        .filter_map(|path| dirty.remove(&path))
        .collect()))
}

fn mark_unknown(repo: &mut RepoState, scope: &WorktreeDirtyScope, scan_id: u64, error: &Error) {
    if matches!(scope, WorktreeDirtyScope::All) {
        repo.worktree_scan_id = scan_id;
    }
    let error: Arc<str> = error
        .to_string()
        .chars()
        .take(512)
        .collect::<String>()
        .into();
    let mut affected = FxHashMap::default();
    if let Some(worktrees) = repo.worktrees.ready() {
        affected.extend(
            worktrees
                .iter()
                .map(|worktree| (worktree.path.clone(), worktree.head.clone())),
        );
    }
    if let Some(dirty) = repo.worktree_dirty.ready() {
        affected.extend(
            dirty
                .iter()
                .map(|summary| (summary.path.clone(), summary.head.clone())),
        );
    }
    affected.extend(
        repo.worktree_observations
            .iter()
            .map(|(path, status)| (path.clone(), status.head.clone())),
    );
    if let WorktreeDirtyScope::Paths(paths) = scope {
        for path in paths {
            affected.entry(path.clone()).or_insert(None);
        }
    }
    let statuses = Arc::make_mut(&mut repo.worktree_observations);
    let mut changed = false;
    for (path, head) in affected {
        if !scope.includes(&path)
            || statuses
                .get(&path)
                .is_some_and(|status| status.scan_id > scan_id)
        {
            continue;
        }
        let cleanliness = WorktreeCleanliness::Unknown(error.clone());
        changed |= statuses
            .get(&path)
            .is_none_or(|previous| previous.head != head || previous.cleanliness != cleanliness);
        statuses.insert(
            path,
            ObservedWorktreeStatus {
                scan_id,
                head,
                cleanliness,
            },
        );
    }
    if changed {
        repo.worktree_observations_rev = repo.worktree_observations_rev.wrapping_add(1);
    }
}
