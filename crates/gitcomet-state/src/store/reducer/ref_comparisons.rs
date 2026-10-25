use crate::model::{AppState, Loadable, RepoState};
use crate::msg::Effect;
use crate::ref_comparisons::{
    ComparisonDecorations, MAX_COMPARISON_PROJECTIONS, RefComparisonsEffect, RefComparisonsMsg,
};
use gitcomet_core::domain::CommitId;
use rustc_hash::FxHashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

fn source(
    repo: &RepoState,
    reference: Option<String>,
) -> Option<(u64, u64, u64, u64, Option<String>)> {
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
    if reference
        .as_deref()
        .is_some_and(|reference| reference.contains("HEAD"))
    {
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
        reference,
    ))
}

pub(super) fn ensure(repo: &mut RepoState, deep: bool) -> Vec<Effect> {
    let lifetime = repo.lifetime_id();
    let reference = repo
        .shared_preferences
        .as_ref()
        .and_then(|snapshot| snapshot.preferences.comparison_ref.clone());
    if reference.is_some() && !matches!(repo.tags, Loadable::Ready(_) | Loadable::Error(_)) {
        repo.ref_comparisons.invalidate();
        if repo.loads_in_flight.request(crate::model::LoadKind::Tags) {
            return vec![Effect::References(crate::msg::ReferenceEffect::LoadTags {
                repo_id: repo.id,
            })];
        }
        return Vec::new();
    }
    let Some(source) = source(repo, reference.clone()) else {
        return Vec::new();
    };
    if !deep && repo.ref_comparisons.resolved_source.as_ref() == Some(&source) {
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
    let mut branches = rustc_hash::FxHashMap::default();
    let mut worktrees = rustc_hash::FxHashMap::default();
    let mut subjects = Vec::new();
    let mut seen = FxHashSet::default();
    let mut push = |id: &CommitId| {
        if subjects.len() < MAX_COMPARISON_PROJECTIONS && seen.insert(id.clone()) {
            subjects.push(id.clone());
        }
    };
    for id in repo.ref_comparisons.visible.iter() {
        push(id);
    }
    if let Loadable::Ready(list) = &repo.branches
        && let Loadable::Ready(head) = &repo.head_branch
        && let Some(branch) = list.iter().find(|branch| &branch.name == head)
    {
        push(&branch.target);
    }
    if let Loadable::Ready(list) = &repo.worktrees {
        for worktree in list.iter().take(MAX_COMPARISON_PROJECTIONS) {
            if let Some(head) = &worktree.head {
                worktrees.insert(worktree.path.clone(), head.clone());
                push(head);
            }
        }
    }
    if let Loadable::Ready(list) = &repo.branches {
        for branch in list.iter().take(MAX_COMPARISON_PROJECTIONS) {
            branches.insert(branch.name.clone(), branch.target.clone());
            push(&branch.target);
        }
    }
    if subjects.is_empty() {
        return Vec::new();
    }
    let state = &mut repo.ref_comparisons;
    state.source = Some(source);
    state.branches = Arc::new(branches);
    state.worktrees = Arc::new(worktrees);
    state.generation = state.generation.wrapping_add(1);
    state.target = Loadable::Loading;
    state.rev = state.rev.wrapping_add(1);
    vec![Effect::RefComparisons(RefComparisonsEffect {
        repo_id: repo.id,
        lifetime,
        generation: state.generation,
        priority_only: false,
        reference,
        subjects: Arc::new(subjects),
        deep,
    })]
}

pub(super) fn reduce(state: &mut AppState, message: RefComparisonsMsg) -> Vec<Effect> {
    let repo_id = match &message {
        RefComparisonsMsg::Ensure { repo_id }
        | RefComparisonsMsg::Prioritize { repo_id, .. }
        | RefComparisonsMsg::Deeper { repo_id }
        | RefComparisonsMsg::Target { repo_id, .. }
        | RefComparisonsMsg::Publish { repo_id, .. } => *repo_id,
    };
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    match message {
        RefComparisonsMsg::Ensure { .. } => ensure(repo, false),
        RefComparisonsMsg::Deeper { .. } => ensure(repo, true),
        RefComparisonsMsg::Prioritize { subjects, .. } => {
            if subjects.as_slice() != repo.ref_comparisons.visible.as_slice() {
                repo.ref_comparisons.visible = Arc::new(subjects.into_iter().take(128).collect());
                return vec![Effect::RefComparisons(RefComparisonsEffect {
                    repo_id,
                    lifetime: repo.lifetime_id(),
                    generation: repo.ref_comparisons.generation,
                    reference: None,
                    subjects: repo.ref_comparisons.visible.clone(),
                    deep: false,
                    priority_only: true,
                })];
            }
            Vec::new()
        }
        RefComparisonsMsg::Target {
            lifetime,
            generation,
            result,
            ..
        } => {
            if lifetime != repo.lifetime_id() || generation != repo.ref_comparisons.generation {
                return Vec::new();
            }
            let comparisons = &mut repo.ref_comparisons;
            match result {
                Ok(target) => {
                    if comparisons
                        .captured_target
                        .as_ref()
                        .is_none_or(|old| old.oid != target.oid || old.context != target.context)
                    {
                        comparisons.decorations = ComparisonDecorations::default();
                    }
                    comparisons.captured_target = Some(target.clone());
                    comparisons.publication_generation = Some(generation);
                    comparisons.resolved_source = comparisons.source.clone();
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
            ..
        } => {
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
            repo.ref_comparisons.decorations.publish(batch.results);
            repo.ref_comparisons.rev = repo.ref_comparisons.rev.wrapping_add(1);
            Vec::new()
        }
    }
}
