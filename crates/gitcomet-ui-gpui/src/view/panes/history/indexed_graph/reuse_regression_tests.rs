use super::*;
use gitcomet_core::domain::UpstreamDivergence;
use gitcomet_core::history_perf::{self, Work};

fn branch(name: &str, target: CommitId) -> Branch {
    Branch {
        name: name.into(),
        target,
        upstream: None,
        divergence: None,
    }
}

#[test]
fn checkpoints_carry_no_capacity_slack_at_any_width() {
    for width in [1usize, 64, 512, 5261] {
        let fixture = IndexedHistoryFixture::new(20_000, width, 20);
        assert_eq!(fixture.graph.checkpoints.len(), 20);
        assert_eq!(
            fixture.graph.checkpoint_capacity_slack(),
            0,
            "width={width}"
        );
    }
}

#[test]
fn integration_containment_and_labels_are_reused_when_their_inputs_hold_still() {
    let fixture = IndexedHistoryFixture::new(2000, 64, 20);
    let index = fixture.graph.projection.index.clone();
    let cancel = CancellationToken::new();
    let head = index.commit_id(0).unwrap();
    let mut branches = vec![
        branch("main", head.clone()),
        branch("feature", index.commit_id(65).unwrap()),
    ];
    let _capture = history_perf::capture();
    let first = IndexedGraph::build(
        index.clone(),
        &branches,
        &[],
        &[],
        Some("main"),
        Some(head.as_ref()),
        &cancel,
    )
    .unwrap();
    assert_eq!(history_perf::count(Work::ContainmentWalk), 1);
    assert_eq!(history_perf::count(Work::GraphTransition), 2000);
    let window = first.window(0..40, &cancel).unwrap();
    assert_eq!(window.labels[0].as_deref(), Some("main"));

    // Divergence moves after every fetch without touching attribution.
    branches[1].divergence = Some(UpstreamDivergence {
        ahead: 1,
        behind: 2,
    });
    let before = history_perf::count(Work::GraphTransition);
    let same = IndexedGraph::build_reusing(
        index.clone(),
        &branches,
        &[],
        &[],
        Some("main"),
        Some(head.as_ref()),
        &cancel,
        Some(&first),
    )
    .unwrap();
    assert_eq!(
        history_perf::count(Work::ContainmentWalk),
        1,
        "reachability reused"
    );
    assert_eq!(
        history_perf::count(Work::GraphTransition),
        before,
        "labels reused without replaying the rows"
    );
    assert!(Arc::ptr_eq(
        &first.containment[0].bits,
        &same.containment[0].bits
    ));
    let reused = same.window(0..40, &cancel).unwrap();
    assert_eq!(reused.labels, window.labels);
    assert!(Arc::ptr_eq(&window.rows, &reused.rows));

    // A rename changes attribution but not reachability: replay, keep the walk.
    branches[1].name = "renamed".into();
    let before = history_perf::count(Work::GraphTransition);
    let renamed = IndexedGraph::build_reusing(
        index.clone(),
        &branches,
        &[],
        &[],
        Some("main"),
        Some(head.as_ref()),
        &cancel,
        Some(&same),
    )
    .unwrap();
    assert_eq!(history_perf::count(Work::ContainmentWalk), 1);
    assert_eq!(history_perf::count(Work::GraphTransition), before + 2000);
    assert!(Arc::ptr_eq(
        &same.containment[0].bits,
        &renamed.containment[0].bits
    ));
    assert_eq!(
        renamed.window(0..40, &cancel).unwrap().labels,
        window.labels
    );

    // Moving the integration tip invalidates its reachability.
    branches[0].target = index.commit_id(1).unwrap();
    let moved = IndexedGraph::build_reusing(
        index,
        &branches,
        &[],
        &[],
        Some("main"),
        Some(branches[0].target.as_ref()),
        &cancel,
        Some(&renamed),
    )
    .unwrap();
    assert_eq!(history_perf::count(Work::ContainmentWalk), 2);
    assert!(!Arc::ptr_eq(
        &renamed.containment[0].bits,
        &moved.containment[0].bits
    ));
    assert_eq!(moved.containment[0].tip, 1);
}

#[test]
#[ignore = "reachability walk cost per integration tip at two million rows"]
fn integration_containment_walk_timing() {
    use std::time::Instant;
    let fixture = IndexedHistoryFixture::new(2_000_000, 64, 20);
    let index = fixture.graph.projection.index.clone();
    let head = index.commit_id(0).unwrap();
    let mut branches = vec![branch("main", head.clone())];
    let cancel = CancellationToken::new();
    let started = Instant::now();
    let first = IndexedGraph::build(
        index.clone(),
        &branches,
        &[],
        &[],
        Some("main"),
        Some(head.as_ref()),
        &cancel,
    )
    .unwrap();
    let build = started.elapsed().as_secs_f64();
    branches[0].divergence = Some(UpstreamDivergence {
        ahead: 3,
        behind: 0,
    });
    let started = Instant::now();
    let _capture = history_perf::capture();
    let again = IndexedGraph::build_reusing(
        index,
        &branches,
        &[],
        &[],
        Some("main"),
        Some(head.as_ref()),
        &cancel,
        Some(&first),
    )
    .unwrap();
    eprintln!(
        "containment rows=2000000 first_build_s={build:.3} rebuild_s={:.3} walks={} transitions={}",
        started.elapsed().as_secs_f64(),
        history_perf::count(Work::ContainmentWalk),
        history_perf::count(Work::GraphTransition)
    );
    assert_eq!(again.checkpoints.len(), first.checkpoints.len());
}
