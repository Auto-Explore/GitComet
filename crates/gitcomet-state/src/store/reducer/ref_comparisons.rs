use crate::model::{AppState, Loadable, RepoState};
use crate::msg::Effect;
use crate::ref_comparison_decorations::ComparisonDecorations;
use crate::ref_comparisons::{RefComparisonSubscription, RefComparisonsEffect, RefComparisonsMsg};
use std::hash::{Hash, Hasher};

fn source(repo: &RepoState, reference: Option<&str>) -> Option<(u64, u64, u64, u64, Option<u64>)> {
    if matches!(repo.worktrees, Loadable::Loading)
        || repo
            .loads_in_flight
            .is_in_flight(crate::model::LoadKind::Worktrees)
    {
        return None;
    }
    if [
        crate::model::LoadKind::Branches,
        crate::model::LoadKind::Remotes,
        crate::model::LoadKind::RemoteBranches,
    ]
    .iter()
    .any(|kind| repo.loads_in_flight.is_in_flight(*kind))
    {
        return None;
    }
    let (Loadable::Ready(branches), Loadable::Ready(remotes), Loadable::Ready(remote_branches)) =
        (&repo.branches, &repo.remotes, &repo.remote_branches)
    else {
        return None;
    };
    let mut local = rustc_hash::FxHasher::default();
    for branch in branches.iter() {
        branch.name.hash(&mut local);
        branch.target.hash(&mut local);
    }
    if reference.is_some_and(|reference| reference.contains("HEAD")) {
        let Loadable::Ready(head) = &repo.head_branch else {
            return None;
        };
        if head == "HEAD" && repo.detached_head_commit.is_none() {
            return None;
        }
        head.hash(&mut local);
        repo.detached_head_commit.hash(&mut local);
    }
    if reference.is_some()
        && let Loadable::Ready(tags) = &repo.tags
    {
        for tag in tags.iter() {
            tag.name.hash(&mut local);
            tag.target.hash(&mut local);
        }
    }
    let mut remote = rustc_hash::FxHasher::default();
    for branch in remote_branches.iter() {
        branch.remote.hash(&mut remote);
        branch.name.hash(&mut remote);
        branch.target.hash(&mut remote);
    }
    let mut defaults = rustc_hash::FxHasher::default();
    repo.ref_comparisons.context_epoch.hash(&mut defaults);
    for remote in remotes.iter() {
        remote.name.hash(&mut defaults);
        remote.default_branch.hash(&mut defaults);
    }
    let mut worktrees = rustc_hash::FxHasher::default();
    if let Loadable::Ready(list) = &repo.worktrees {
        for worktree in list.iter() {
            worktree.path.hash(&mut worktrees);
            worktree.head.hash(&mut worktrees);
        }
    }
    Some((
        local.finish(),
        worktrees.finish(),
        remote.finish(),
        defaults.finish(),
        reference.map(|reference| {
            let mut hasher = rustc_hash::FxHasher::default();
            reference.hash(&mut hasher);
            hasher.finish()
        }),
    ))
}

pub(super) fn ensure(repo: &mut RepoState, deep: bool) -> Vec<Effect> {
    let lifetime = repo.lifetime_id();
    let reference = repo
        .shared_preferences
        .as_ref()
        .and_then(|snapshot| snapshot.preferences.comparison_ref.as_deref());
    if reference.is_some() && !matches!(repo.tags, Loadable::Ready(_) | Loadable::Error(_)) {
        repo.ref_comparisons.invalidate();
        if repo.loads_in_flight.request(crate::model::LoadKind::Tags) {
            return vec![Effect::References(crate::msg::ReferenceEffect::LoadTags {
                repo_id: repo.id,
            })];
        }
        return Vec::new();
    }
    let Some(source) = source(repo, reference) else {
        return Vec::new();
    };
    if !deep
        && repo.ref_comparisons.publication_generation.is_some()
        && repo.ref_comparisons.resolved_source.as_ref() == Some(&source)
    {
        if repo.ref_comparisons.source.as_ref() != Some(&source) {
            repo.ref_comparisons.generation = repo.ref_comparisons.generation.wrapping_add(1);
            repo.ref_comparisons.source = Some(source);
        }
        if matches!(repo.ref_comparisons.target, Loadable::Loading)
            && let Some(target) = repo.ref_comparisons.captured_target.clone()
        {
            repo.ref_comparisons.target = Loadable::Ready(target);
            repo.ref_comparisons.rev = repo.ref_comparisons.rev.wrapping_add(1);
        }
        return Vec::new();
    }
    if !deep && repo.ref_comparisons.source.as_ref() == Some(&source) {
        return Vec::new();
    }
    if !matches!(repo.branches, Loadable::Ready(_)) && !matches!(repo.worktrees, Loadable::Ready(_))
    {
        return Vec::new();
    }
    let projections = repo.ref_comparisons.memory.as_ref().and_then(|memory| {
        crate::ref_comparison_projections::ComparisonProjections::capture(
            memory,
            repo.branches.ready().map_or(&[], |list| list.as_slice()),
            repo.worktrees.ready().map_or(&[], |list| list.as_slice()),
            &repo.ref_comparisons.visible,
            repo.head_commit_id(),
            64 * 1024
                + reference
                    .as_ref()
                    .map_or(0, |reference| reference.len().saturating_mul(6)),
        )
    });
    let Some(projections) = projections else {
        let comparisons = &mut repo.ref_comparisons;
        comparisons.forget_subjects();
        comparisons.source = Some(source);
        comparisons.target = Loadable::Error(if comparisons.memory.is_some() {
            "Comparison analysis reached its shared memory or subject limit. Close unused repositories or run a deeper check to retry.".into()
        } else {
            "Comparison analysis is not supported by this Git backend.".into()
        });
        comparisons.rev = comparisons.rev.wrapping_add(1);
        return vec![Effect::RefComparisons(RefComparisonsEffect::Release {
            lifetime,
        })];
    };
    if projections.subjects.is_empty() {
        let comparisons = &mut repo.ref_comparisons;
        comparisons.forget_subjects();
        comparisons.source = Some(source);
        comparisons.target = Loadable::NotLoaded;
        comparisons.rev = comparisons.rev.wrapping_add(1);
        return vec![Effect::RefComparisons(RefComparisonsEffect::Release {
            lifetime,
        })];
    }
    let state = &mut repo.ref_comparisons;
    if let Some(memory) = &state.memory
        && state
            .decorations
            .retain(memory, &projections.members)
            .is_err()
    {
        state.decorations = ComparisonDecorations::default();
    }
    state.source = Some(source);
    state.projections = projections.clone();
    state.generation = state.generation.wrapping_add(1);
    state.publication_generation = None;
    state.target = Loadable::Loading;
    state.deferred = None;
    state.rev = state.rev.wrapping_add(1);
    vec![Effect::RefComparisons(RefComparisonsEffect::Compare(
        RefComparisonSubscription {
            repo_id: repo.id,
            lifetime,
            generation: state.generation,
            reference: reference.map(str::to_owned),
            projections,
            visible: state.visible.clone(),
            memory: state
                .memory
                .as_ref()
                .expect("captured projections have a memory owner")
                .clone(),
            deep,
        },
    ))]
}

pub(super) fn reduce(state: &mut AppState, message: RefComparisonsMsg) -> Vec<Effect> {
    let repo_id = match &message {
        RefComparisonsMsg::Ensure { repo_id }
        | RefComparisonsMsg::Prioritize { repo_id, .. }
        | RefComparisonsMsg::Deeper { repo_id }
        | RefComparisonsMsg::Deferred { repo_id, .. }
        | RefComparisonsMsg::Target { repo_id, .. }
        | RefComparisonsMsg::Publish { repo_id, .. } => *repo_id,
    };
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    match message {
        RefComparisonsMsg::Deferred {
            lifetime,
            generation,
            reason,
            reservation,
            ..
        } => {
            let _reservation = reservation;
            if lifetime == repo.lifetime_id()
                && repo.ref_comparisons.publication_generation == Some(generation)
            {
                repo.ref_comparisons.deferred = Some(reason);
                repo.ref_comparisons.rev = repo.ref_comparisons.rev.wrapping_add(1);
            }
            Vec::new()
        }
        RefComparisonsMsg::Ensure { .. } => ensure(repo, false),
        RefComparisonsMsg::Deeper { .. } => ensure(repo, true),
        RefComparisonsMsg::Prioritize { subjects, .. } => {
            if subjects.as_slice() != repo.ref_comparisons.visible.as_slice() {
                let Some(visible) = repo.ref_comparisons.memory.as_ref().and_then(|memory| {
                    crate::ref_comparisons::ComparisonPriorities::capture(memory, subjects)
                }) else {
                    return Vec::new();
                };
                repo.ref_comparisons.visible = visible;
                return vec![Effect::RefComparisons(RefComparisonsEffect::Prioritize {
                    lifetime: repo.lifetime_id(),
                    subjects: repo.ref_comparisons.visible.clone(),
                })];
            }
            Vec::new()
        }
        RefComparisonsMsg::Target {
            lifetime,
            generation,
            result,
            reservation,
            ..
        } => {
            if lifetime != repo.lifetime_id() || generation != repo.ref_comparisons.generation {
                return Vec::new();
            }
            let comparisons = &mut repo.ref_comparisons;
            match result {
                Ok(target) => {
                    comparisons.target_reservation = reservation;
                    if comparisons
                        .captured_target
                        .as_ref()
                        .is_none_or(|old| old.oid != target.oid || old.context != target.context)
                    {
                        comparisons.decorations = ComparisonDecorations::default();
                    }
                    comparisons.captured_target = Some(target.clone());
                    comparisons.publication_generation = Some(generation);
                    comparisons.resolved_source = comparisons.source;
                    comparisons.target = Loadable::Ready(target);
                }
                Err(error) => comparisons.target = Loadable::Error(error.to_string()),
            }
            comparisons.rev = comparisons.rev.wrapping_add(1);
            Vec::new()
        }
        RefComparisonsMsg::Publish {
            lifetime,
            generation,
            batch,
            reservation,
            ..
        } => {
            let _reservation = reservation;
            if lifetime != repo.lifetime_id()
                || Some(generation) != repo.ref_comparisons.publication_generation
                || repo
                    .ref_comparisons
                    .captured_target
                    .as_ref()
                    .is_none_or(|target| {
                        target.oid != batch.target.oid || target.context != batch.target.context
                    })
            {
                return Vec::new();
            }
            if let Some(memory) = &repo.ref_comparisons.memory {
                let results: Vec<_> = batch
                    .results
                    .into_iter()
                    .filter(|result| {
                        repo.ref_comparisons
                            .projections
                            .members
                            .contains(&result.subject)
                    })
                    .collect();
                if results.is_empty() {
                    return Vec::new();
                }
                if let Err(reason) = repo.ref_comparisons.decorations.publish(memory, results) {
                    repo.ref_comparisons.deferred = Some(reason);
                }
            }
            repo.ref_comparisons.rev = repo.ref_comparisons.rev.wrapping_add(1);
            Vec::new()
        }
    }
}
