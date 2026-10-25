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

fn start(state: &mut AppState) -> crate::ref_comparisons::RefComparisonsEffect {
    let Effect::RefComparisons(work) = ensure(&mut state.repos[0], false).remove(0) else {
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

fn resolve(state: &mut AppState, work: &crate::ref_comparisons::RefComparisonsEffect) {
    reduce(
        state,
        RefComparisonsMsg::Target {
            repo_id: work.repo_id,
            lifetime: work.lifetime,
            generation: work.generation,
            result: Ok(target()),
        },
    );
}

fn publish(state: &mut AppState, work: &crate::ref_comparisons::RefComparisonsEffect) {
    reduce(
        state,
        RefComparisonsMsg::Publish {
            repo_id: work.repo_id,
            lifetime: work.lifetime,
            generation: work.generation,
            batch: RefComparisonBatch {
                target: target(),
                results: vec![RefComparison {
                    subject: work.subjects[0].clone(),
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
            .get(&work.subjects[0])
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
            .get(&next.subjects[0])
            .is_none()
    );
    publish(&mut state, &next);
    assert!(old_snapshot.get(&next.subjects[0]).is_none());
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
            .get(&work.subjects[0])
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
            .get(&work.subjects[0])
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
            .get(&work.subjects[0])
            .is_none()
    );
}
