use super::ref_comparisons::*;
use crate::model::{AppState, Loadable, RepoId, RepoState};
use crate::msg::Effect;
use crate::ref_comparisons::RefComparisonsMsg;
use gitcomet_core::domain::{Branch, CommitId, RepoSpec};
use gitcomet_core::ref_comparison::*;
use std::sync::Arc;

fn fixture() -> AppState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/comparisons".into(),
        },
    );
    repo.ref_comparisons.memory = Some(Arc::new(
        gitcomet_core::ref_comparison_memory::ComparisonMemory::default(),
    ));
    repo.set_branches(Loadable::Ready(vec![Branch {
        name: "topic".into(),
        target: CommitId("a".repeat(40).into()),
        upstream: None,
        divergence: None,
    }]));
    repo.set_remotes(Loadable::Ready(Vec::new()));
    repo.set_remote_branches(Loadable::Ready(Vec::new()));
    AppState {
        repos: vec![repo],
        ..Default::default()
    }
}

fn start(state: &mut AppState) -> crate::ref_comparisons::RefComparisonSubscription {
    let Effect::RefComparisons(crate::ref_comparisons::RefComparisonsEffect::Compare(work)) =
        ensure(&mut state.repos[0], false).remove(0)
    else {
        panic!("comparison effect")
    };
    work
}

fn target() -> RefComparisonTarget {
    RefComparisonTarget {
        reference: "origin/dev".into(),
        oid: CommitId("b".repeat(40).into()),
        context: 1,
    }
}

fn resolve(state: &mut AppState, work: &crate::ref_comparisons::RefComparisonSubscription) {
    reduce(
        state,
        RefComparisonsMsg::Target {
            reservation: None,
            repo_id: work.repo_id,
            lifetime: work.lifetime,
            generation: work.generation,
            result: Ok(target()),
        },
    );
}

fn publish(state: &mut AppState, work: &crate::ref_comparisons::RefComparisonSubscription) {
    reduce(
        state,
        RefComparisonsMsg::Publish {
            metrics: Default::default(),
            repo_id: work.repo_id,
            lifetime: work.lifetime,
            generation: work.generation,
            reservation: work.memory.reserve(32 * 1024).unwrap(),
            batch: RefComparisonBatch {
                target: target(),
                results: vec![RefComparison {
                    subject: work.projections.subjects[0].clone(),
                    counts: RefComparisonCounts::Exact {
                        ahead: 0,
                        behind: 3,
                    },
                    integration: RefIntegration::Confirmed(IntegrationEvidence::Merged),
                }],
                graph_reads: 3,
                cache_hits: 0,
            },
        },
    );
}

#[test]
fn comparison_projections_are_bounded_and_remain_owned_by_retained_snapshots() {
    let _guard = crate::ref_comparisons::MEMORY_TESTS.lock().unwrap();
    let mut state = fixture();
    let memory = state.repos[0]
        .ref_comparisons
        .memory
        .as_ref()
        .unwrap()
        .clone();
    let held = memory.reserve(COMPARISON_REPOSITORY_BYTES).unwrap();
    assert!(matches!(
        ensure(&mut state.repos[0], false).as_slice(),
        [Effect::RefComparisons(
            crate::ref_comparisons::RefComparisonsEffect::Release { .. }
        )]
    ));
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Error(_)
    ));
    drop(held);
    assert!(
        ensure(&mut state.repos[0], false).is_empty(),
        "exhaustion is deferred until an explicit retry"
    );
    let Effect::RefComparisons(crate::ref_comparisons::RefComparisonsEffect::Compare(work)) =
        ensure(&mut state.repos[0], true).remove(0)
    else {
        panic!("explicit retry")
    };
    assert!(memory.used_bytes() > 0);
    let retained = state.clone();
    drop(state);
    drop(work);
    assert!(memory.used_bytes() > 0);
    drop(retained);
    assert_eq!(memory.used_bytes(), 0);
}

#[test]
fn unchanged_refresh_waits_for_the_worktree_snapshot_without_starting_comparisons() {
    let mut state = fixture();
    state.repos[0].set_worktrees(Loadable::Ready(vec![gitcomet_core::domain::Worktree {
        path: "/linked".into(),
        bare: false,
        head: Some(CommitId("c".repeat(40).into())),
        branch: None,
        detached: true,
    }]));
    let work = start(&mut state);
    resolve(&mut state, &work);
    let previous = state.repos[0].worktrees.ready().unwrap().as_ref().clone();
    state.repos[0].set_worktrees(Loadable::Loading);
    assert!(ensure(&mut state.repos[0], false).is_empty());
    state.repos[0].set_worktrees(Loadable::Ready(previous));
    assert!(ensure(&mut state.repos[0], false).is_empty());
}

#[test]
fn empty_accepted_subjects_release_ownership_and_reject_the_previous_completion() {
    let mut state = fixture();
    let work = start(&mut state);
    resolve(&mut state, &work);
    publish(&mut state, &work);
    state.repos[0].set_branches(Loadable::Ready(Vec::new()));
    let effects = ensure(&mut state.repos[0], false);
    assert!(
        matches!(effects.as_slice(), [Effect::RefComparisons(crate::ref_comparisons::RefComparisonsEffect::Release { lifetime })] if *lifetime == work.lifetime)
    );
    assert!(
        state.repos[0]
            .ref_comparisons
            .projections
            .subjects
            .is_empty()
    );
    assert!(
        state.repos[0]
            .ref_comparisons
            .decorations
            .get(&work.projections.subjects[0])
            .is_none()
    );
    resolve(&mut state, &work);
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::NotLoaded
    ));
    assert!(ensure(&mut state.repos[0], false).is_empty());
}

#[test]
fn comparison_only_publications_do_not_change_sidebar_structure_and_reject_stale_generations() {
    let mut state = fixture();
    let work = start(&mut state);
    resolve(&mut state, &work);
    let structural = state.repos[0].branch_sidebar_rev;
    publish(&mut state, &work);
    assert_eq!(state.repos[0].branch_sidebar_rev, structural);
    assert!(
        state.repos[0]
            .ref_comparisons
            .decorations
            .get(&work.projections.subjects[0])
            .is_some()
    );
    let old_snapshot = state.repos[0].ref_comparisons.decorations.clone();
    let mut branches = Arc::unwrap_or_clone(match &state.repos[0].branches {
        Loadable::Ready(branches) => branches.clone(),
        _ => panic!(),
    });
    branches[0].target = CommitId("c".repeat(40).into());
    state.repos[0].set_branches(Loadable::Ready(branches));
    let next = start(&mut state);
    resolve(&mut state, &next);
    let rev = state.repos[0].ref_comparisons.rev;
    publish(&mut state, &work);
    assert_eq!(state.repos[0].ref_comparisons.rev, rev);
    assert!(
        state.repos[0]
            .ref_comparisons
            .decorations
            .get(&next.projections.subjects[0])
            .is_none()
    );
    publish(&mut state, &next);
    assert!(old_snapshot.get(&next.projections.subjects[0]).is_none());
}

#[test]
fn unchanged_refresh_reuses_results_without_scheduling_comparison_work() {
    let mut state = fixture();
    let work = start(&mut state);
    resolve(&mut state, &work);
    publish(&mut state, &work);
    let branches = match &state.repos[0].branches {
        Loadable::Ready(branches) => branches.as_ref().clone(),
        _ => panic!(),
    };
    state.repos[0].set_branches(Loadable::Loading);
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Loading
    ));
    state.repos[0].set_branches(Loadable::Ready(branches));
    assert!(ensure(&mut state.repos[0], false).is_empty());
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Ready(_)
    ));
    assert!(
        state.repos[0]
            .ref_comparisons
            .decorations
            .get(&work.projections.subjects[0])
            .is_some()
    );
}

#[test]
fn replacement_repository_lifetime_rejects_the_old_completion() {
    let mut state = fixture();
    let work = start(&mut state);
    state = fixture();
    let next = start(&mut state);
    assert_ne!(work.lifetime, next.lifetime);
    resolve(&mut state, &work);
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Loading
    ));
}

#[test]
fn unchanged_refresh_during_counting_preserves_the_immutable_input_subscription() {
    let mut state = fixture();
    let work = start(&mut state);
    resolve(&mut state, &work);
    let branches = match &state.repos[0].branches {
        Loadable::Ready(branches) => branches.as_ref().clone(),
        _ => panic!(),
    };
    state.repos[0].set_branches(Loadable::Loading);
    publish(&mut state, &work);
    resolve(&mut state, &work);
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Loading
    ));
    state.repos[0].set_branches(Loadable::Ready(branches));
    assert!(ensure(&mut state.repos[0], false).is_empty());
    assert!(
        state.repos[0]
            .ref_comparisons
            .decorations
            .get(&work.projections.subjects[0])
            .is_some()
    );
}

#[test]
fn an_unchanged_snapshot_cannot_restore_a_previous_target_while_an_override_is_pending() {
    let mut state = fixture();
    let work = start(&mut state);
    resolve(&mut state, &work);
    state.repos[0].set_tags(Loadable::Ready(Vec::new()));
    let preferences = crate::model::SharedRepositoryPreferences {
        comparison_ref: Some("release".into()),
        ..Default::default()
    };
    state.repos[0].shared_preferences = Some(crate::model::RepositoryPreferencesSnapshot {
        key: state.repos[0].repository_key(),
        revision: 1,
        preferences: Arc::new(preferences),
    });
    let next = start(&mut state);
    assert_ne!(next.generation, work.generation);
    assert!(ensure(&mut state.repos[0], false).is_empty());
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Loading
    ));
    resolve(&mut state, &work);
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Loading
    ));
}

#[test]
fn a_verification_context_only_event_restarts_comparisons_without_waiting_for_ref_loads() {
    let mut state = fixture();
    let work = start(&mut state);
    resolve(&mut state, &work);
    publish(&mut state, &work);
    let change = crate::msg::RepoExternalChange {
        verification_context: true,
        ..Default::default()
    };
    let effects = super::external_and_history::repo_externally_changed(
        &super::test_inputs::clock(),
        &mut state,
        work.repo_id,
        change,
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefComparisons(_)))
    );
    assert!(matches!(
        state.repos[0].ref_comparisons.target,
        Loadable::Loading
    ));
    publish(&mut state, &work);
    assert!(
        state.repos[0]
            .ref_comparisons
            .decorations
            .get(&work.projections.subjects[0])
            .is_none()
    );
}

#[test]
fn bare_conversion_reopen_restarts_comparisons_under_the_new_lifetime() {
    let _guard = crate::ref_comparisons::MEMORY_TESTS.lock().unwrap();
    let mut state = fixture();
    let work = start(&mut state);
    resolve(&mut state, &work);
    publish(&mut state, &work);
    let rev = state.repos[0].ref_comparisons.rev;

    state.repos[0].retire_repository_lifetime();
    assert_ne!(state.repos[0].ref_comparisons.rev, rev);
    // The reopened handle brings its own memory; the refs reload unchanged.
    state.repos[0].ref_comparisons.memory = Some(Arc::new(
        gitcomet_core::ref_comparison_memory::ComparisonMemory::default(),
    ));
    let next = start(&mut state);
    assert_ne!(next.lifetime, work.lifetime);
}
