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
