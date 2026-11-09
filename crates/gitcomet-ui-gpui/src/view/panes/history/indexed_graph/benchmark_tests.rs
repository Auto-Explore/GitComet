use super::*;
#[test]
#[ignore = "two million row index/graph benchmark"]
fn indexed_history_two_million_graph_window_benchmark() {
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::services::HistorySnapshot;
    use std::time::Instant;
    let raw = |row: usize| {
        let mut id = [0u8; 20];
        id[..8].copy_from_slice(&(row as u64).to_be_bytes());
        id
    };
    let count: usize = 2_000_000;
    let cancellation = CancellationToken::new();
    let mut builder =
        HistoryIndexBuilder::new(HistorySnapshot("bench".into()), LogScope::AllBranches, 20)
            .unwrap();
    for row in 0..count {
        let parents: SmallVec<[[u8; 20]; 2]> = [row + 1, row + 97]
            .into_iter()
            .filter(|&parent| parent < count && (parent == row + 1 || row.is_multiple_of(101)))
            .map(raw)
            .collect();
        builder
            .push(
                &raw(row),
                parents.iter().map(|parent| parent.as_slice()),
                false,
            )
            .unwrap();
    }
    let index = builder.finish(&cancellation).unwrap();
    let started = Instant::now();
    let graph = IndexedGraph::build(index, &[], &[], &[], None, None, &cancellation).unwrap();
    eprintln!(
        "graph rows={count} seconds={:.3} checkpoints={}",
        started.elapsed().as_secs_f64(),
        graph.checkpoints.len()
    );
    let mut samples = Vec::new();
    for ix in 0..100 {
        let start = (ix * 97171) % (count - 256);
        let started = Instant::now();
        let window = graph.window(start..start + 256, &cancellation).unwrap();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(window.rows.len(), 256);
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "graph 256-row replay p50_ms={:.3} p95_ms={:.3}",
        samples[50], samples[95]
    );
    assert_eq!(graph.checkpoints.len(), count.div_ceil(CHECKPOINT_STRIDE));
}
