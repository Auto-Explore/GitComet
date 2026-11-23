use super::*;
use gitcomet_core::history_index::HistoryIndexBuilder;
use gitcomet_core::services::HistorySnapshot;

fn raw(row: usize) -> [u8; 20] {
    let mut raw = [0u8; 20];
    raw[..8].copy_from_slice(&(row as u64 + 1).to_be_bytes());
    raw
}

#[test]
fn checkpoint_windows_match_continuous_graph_and_attribution() {
    for (linear, width) in [(true, 1), (false, 83), (false, 512)] {
        let count = 4200;
        let mut builder =
            HistoryIndexBuilder::new(HistorySnapshot("graph".into()), LogScope::AllBranches, 20)
                .unwrap();
        let mut commits = Vec::new();
        for row in 0..count {
            let mut parents = Vec::new();
            if row + width < count {
                parents.push(raw(row + width));
            }
            if !linear && row.is_multiple_of(37) && row + 1 < count {
                parents.push(raw(row + 1));
            }
            builder
                .push(&raw(row), parents.iter().map(|id| id.as_slice()), false)
                .unwrap();
            commits.push(Commit {
                id: CommitId(gitcomet_core::hex::encode(&raw(row)).into()),
                parent_ids: parents
                    .iter()
                    .map(|id| CommitId(gitcomet_core::hex::encode(id).into()))
                    .collect(),
                summary: Arc::from("commit"),
                author: Arc::from("author"),
                time: std::time::UNIX_EPOCH,
            });
        }
        let index = builder.finish(&CancellationToken::new()).unwrap();
        let branches = vec![
            Branch {
                name: "main".into(),
                target: commits[0].id.clone(),
                upstream: None,
                divergence: None,
            },
            Branch {
                name: "feature".into(),
                target: commits[35].id.clone(),
                upstream: None,
                divergence: None,
            },
        ];
        let graph = IndexedGraph::build(
            index,
            &branches,
            &[],
            &[],
            Some("main"),
            Some(commits[0].id.as_ref()),
            &CancellationToken::new(),
        )
        .unwrap();
        let request = HistoryBaseCacheRequest {
            repo_id: RepoId(1),
            history_scope: LogScope::AllBranches,
            log_source: 0,
            history_author_filter: None,
            history_focus: Default::default(),
            head_branch_rev: 0,
            detached_head_commit: None,
            head_branch_target: Some(commits[0].id.clone()),
            branches_rev: 0,
            remote_branches: Default::default(),
            stashes_rev: 0,
        };
        let page = LogPage {
            commits,
            next_cursor: None,
        };
        let theme = AppTheme::gitcomet_dark();
        let base = build_history_base_cache(
            request.clone(),
            &page,
            theme,
            Some("main"),
            &branches,
            &[],
            &[],
        );
        let decorations = build_history_decoration_cache(
            HistoryDecorationCacheRequest {
                base_request: request,
                head_branch_rev: 0,
                detached_head_commit: None,
                branches_rev: 0,
                remote_branches: Default::default(),
                tags_rev: 0,
            },
            &page,
            &base,
            Some("main"),
            &branches,
            &[],
            &[],
        );
        for start in [0, 900, 1023, 1024, 2047, 3071, 4100] {
            let end = (start + 150).min(count);
            let window = graph.window(start..end, &CancellationToken::new()).unwrap();
            assert_eq!(
                window.rows.as_ref(),
                &base.graph_rows[start..end],
                "linear={linear} start={start}"
            );
            for anchor in [0, 35, 1025, 3700] {
                use crate::view::rows::history_graph_paint as paint;
                for branch in [None, Some(false), Some(true)] {
                    let color = branch.map_or(base.graph_rows[anchor].node_color_ix, |on_branch| {
                        paint::band_node_for(&base.graph_rows[anchor], on_branch).color_ix
                    });
                    let expected = paint::selected_lane_at(&base.graph_rows, anchor, color);
                    let actual = graph
                        .selected_lane(anchor, branch, start, &CancellationToken::new())
                        .unwrap();
                    for row in 0..window.rows.len() {
                        for color in 0..8 {
                            assert_eq!(
                                actual.map(|lane| lane.covers(theme, row, color)),
                                expected.map(|lane| lane.covers(theme, start + row, color)),
                                "anchor={anchor} window={start} branch={branch:?}"
                            );
                        }
                    }
                }
            }
            for (ix, label) in window.labels.iter().enumerate() {
                let expected = decorations.row_vms[start + ix]
                    .lane_branch
                    .and_then(|ix| decorations.branch_names.get(ix as usize));
                assert_eq!(
                    label.as_ref(),
                    expected,
                    "linear={linear} row={}",
                    start + ix
                );
            }
        }
        assert_eq!(graph.checkpoints.len(), count.div_ceil(CHECKPOINT_STRIDE));
    }
}

/// A history of mostly plain rows among merges, roots, branch heads and
/// stashes, whose second parents the graph hides.
fn mixed_history(count: usize, width: usize, seed: u64) -> HistoryIndexHandle {
    let mut random = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut below = |limit: usize| {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        (random >> 11) as usize % limit
    };
    let mut builder =
        HistoryIndexBuilder::new(HistorySnapshot("mixed".into()), LogScope::AllBranches, 20)
            .unwrap();
    for row in 0..count {
        let stash = row % 97 == 5 && row + 3 < count;
        let mut parents: Vec<usize> = Vec::new();
        if stash {
            parents.extend([row + 3, row + 1]);
        } else if row % 97 == 6 && row + 2 < count {
            parents.push(row + 2);
        } else {
            let wanted = match below(20) {
                0 => 0,
                1 | 2 => 2,
                3 => 3,
                _ => 1,
            };
            for _ in 0..wanted {
                let parent = row + 1 + below(width);
                if parent < count && !parents.contains(&parent) {
                    parents.push(parent);
                }
            }
        }
        let parents: Vec<_> = parents.into_iter().map(raw).collect();
        builder
            .push(&raw(row), parents.iter().map(|id| id.as_slice()), stash)
            .unwrap();
    }
    builder.finish(&CancellationToken::new()).unwrap()
}

#[test]
fn plain_rows_build_the_graph_the_general_path_builds() {
    use crate::view::history_graph::PLAIN_STEPS_OFF;
    let cancel = CancellationToken::new();
    let id = |row: usize| CommitId(gitcomet_core::hex::encode(&raw(row)).into());
    let branch = |name: &str, row: usize| Branch {
        name: name.into(),
        target: id(row),
        upstream: None,
        divergence: None,
    };
    for (seed, width) in [(1, 1), (2, 3), (3, 12), (4, 60)] {
        let count = 5_000;
        let index = mixed_history(count, width, seed);
        let branches = [
            branch("main", 0),
            branch("feature", 40),
            branch("topic", 1_500),
        ];
        // Everything a graph keeps and shows, built with or without the
        // plain steps.
        let shown = |general_only: bool| {
            PLAIN_STEPS_OFF.set(general_only);
            let graph = IndexedGraph::build(
                index.clone(),
                &branches,
                &[],
                &[],
                Some("main"),
                Some(id(0).as_ref()),
                &cancel,
            )
            .unwrap();
            let windows: Vec<_> = [0, 700, 1_023, 1_024, 2_500, 4_900]
                .into_iter()
                .map(|start| {
                    let window = graph.window(start..start + 90, &cancel).unwrap();
                    (window.rows.to_vec(), window.labels.to_vec())
                })
                .collect();
            // Labels alone, over rows a first window already painted.
            let again = graph.window(4_900..4_990, &cancel).unwrap().labels.to_vec();
            PLAIN_STEPS_OFF.set(false);
            let spans: Vec<Vec<_>> = graph
                .lane_spans
                .iter()
                .map(|column| {
                    column
                        .iter()
                        .map(|span| (span.first, span.last, span.color))
                        .collect()
                })
                .collect();
            let checkpoints = format!("{:?}", graph.checkpoints);
            (graph, (checkpoints, spans, windows, again))
        };
        let (graph, plain) = shown(false);
        let (_, general) = shown(true);
        assert!(plain == general, "seed={seed} width={width}");

        // The plain steps are what built a good part of it.
        let mut state = graph.checkpoints[1].restore();
        let taken = (1_024..2_048)
            .filter(|&row| {
                let plain = graph.plain_step(row, &mut state).is_some();
                if !plain {
                    graph.step(row, &mut state, false);
                }
                plain
            })
            .count();
        assert!(taken > 250, "seed={seed} width={width}: {taken}");
    }
}
