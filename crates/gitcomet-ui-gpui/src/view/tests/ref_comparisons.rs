use super::*;
use gitcomet_core::ref_comparison::*;
use gitcomet_core::ref_comparison_memory::ComparisonMemory;
use gitcomet_core::worktree_scan::{ObservedWorktreeStatus, WorktreeCleanliness};
use gitcomet_state::ref_comparison_projections::ComparisonProjections;

#[cfg(all(target_os = "linux", feature = "benchmarks"))]
mod capture;

fn comparison_state() -> (AppState, Arc<ComparisonMemory>) {
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    let repo = &mut state.repos[0];
    let target = RefComparisonTarget {
        reference: "origin/dev".into(),
        oid: CommitId("f".repeat(40).into()),
        context: 1,
    };
    let evidence = [
        RefIntegration::Confirmed(IntegrationEvidence::UpToDate),
        RefIntegration::Confirmed(IntegrationEvidence::Merged),
        RefIntegration::Confirmed(IntegrationEvidence::ChangesMatch),
        RefIntegration::Confirmed(IntegrationEvidence::SquashPatch),
        RefIntegration::Confirmed(IntegrationEvidence::IndividualCommits),
        RefIntegration::Confirmed(IntegrationEvidence::NoNetChanges),
        RefIntegration::NotConfirmed(RefComparisonReason::NoMatch),
    ];
    let names = [
        "a-default-tip",
        "b-merged-feature-with-a-long-branch-name",
        "c-content-matches-the-target",
        "d-squashed-feature-with-later-target-edits",
        "e-rebased-feature-with-later-target-edits",
        "f-empty-or-reverted-feature",
        "g-unconfirmed-local-feature",
    ];
    let mut projections = ComparisonProjections::default();
    let mut results = Vec::new();
    let branches: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let oid = if index == 0 {
                target.oid.clone()
            } else {
                CommitId(format!("{index:040x}").into())
            };
            projections.branches.insert((*name).into(), oid.clone());
            projections.subjects.push(oid.clone());
            results.push(RefComparison {
                subject: oid.clone(),
                counts: RefComparisonCounts::Exact {
                    ahead: if index < 2 { 0 } else { 3 },
                    behind: if index == 0 { 0 } else { 12 },
                },
                integration: evidence[index],
            });
            Branch {
                name: (*name).into(),
                target: oid,
                upstream: (index == 1).then(|| gitcomet_core::domain::Upstream {
                    remote: "origin".into(),
                    branch: (*name).into(),
                }),
                divergence: (index == 1).then_some(gitcomet_core::domain::UpstreamDivergence {
                    ahead: 2,
                    behind: 4,
                }),
            }
        })
        .collect();
    let worktrees: Vec<_> = [
        WorktreeCleanliness::Dirty,
        WorktreeCleanliness::Clean,
        WorktreeCleanliness::Unknown("Checkout is inaccessible.".into()),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, cleanliness)| {
        let path = PathBuf::from(format!(
            "/tmp/linked-checkouts/checkout-{index}-with-a-long-directory-name"
        ));
        let oid = CommitId(format!("{:040x}", index + 100).into());
        projections.worktrees.insert(path.clone(), oid.clone());
        projections.subjects.push(oid.clone());
        Arc::make_mut(&mut repo.worktree_observations).insert(
            path.clone(),
            ObservedWorktreeStatus {
                scan_id: 1,
                head: Some(oid.clone()),
                cleanliness,
            },
        );
        results.push(RefComparison {
            subject: oid.clone(),
            counts: RefComparisonCounts::Exact {
                ahead: 0,
                behind: 12,
            },
            integration: evidence[index + 1],
        });
        gitcomet_core::domain::Worktree {
            path,
            head: Some(oid),
            branch: (index != 2).then(|| names[index + 1].to_string()),
            detached: index == 2,
        }
    })
    .collect();
    repo.head_branch = Loadable::Ready(names[0].into());
    repo.branches = Loadable::Ready(Arc::new(branches));
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remotes = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.worktrees = Loadable::Ready(Arc::new(worktrees));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.ref_comparisons.target = Loadable::Ready(target);
    repo.ref_comparisons.projections = Arc::new(projections);
    let memory = Arc::new(ComparisonMemory::default());
    repo.ref_comparisons
        .decorations
        .publish(&memory, results)
        .unwrap();
    (state, memory)
}

#[gpui::test]
fn comparison_details_render_below_branch_and_checkout_identity_at_narrow_widths(
    cx: &mut gpui::TestAppContext,
) {
    let (state, _memory) = comparison_state();
    let state = Arc::new(state);
    let (store, _unobserved_events) = AppStore::new_test(Arc::new(TestBackend));
    store.disable_repo_monitors_for_test();
    store.replace_snapshot_for_test(state.clone());
    let (_quiet, events) = smol::channel::unbounded();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.simulate_resize(gpui::size(px(1400.0), px(1500.0)));
    for (case, count) in [4, 1_000_000, u32::MAX].into_iter().enumerate() {
        let mut next = state.as_ref().clone();
        let repo = &mut next.repos[0];
        if let Loadable::Ready(branches) = &mut repo.branches {
            Arc::make_mut(branches)[1].divergence =
                Some(gitcomet_core::domain::UpstreamDivergence {
                    ahead: count as usize,
                    behind: count as usize,
                });
        }
        repo.branches_rev = case as u64 + 1;
        repo.branch_sidebar_rev = case as u64 + 1;
        let state = Arc::new(next);
        for percent in [100, 150] {
            cx.update(|window, app| {
                ui_scale::set_window_percent(
                    app,
                    window.window_handle().window_id(),
                    Some(percent),
                );
                view.update(app, |view, cx| {
                    test_support::push_test_state(view, state.clone(), cx);
                    view.apply_ui_scale_percent(percent, window, cx);
                    test_support::set_sidebar_width_for_test(
                        view,
                        px(300.0 * percent as f32 / 100.0),
                        cx,
                    );
                    view.sidebar_pane.update(cx, |pane, cx| {
                        let expanded = branch_sidebar::expanded_default_section_storage_key(
                            branch_sidebar::worktrees_section_storage_key(),
                        )
                        .unwrap();
                        pane.set_collapsed_keys_for_test(&[expanded.as_str()]);
                        cx.notify();
                    });
                })
            });
            cx.run_until_parked();
            test_support::redraw(cx);
            let rows = cx.update(|_, app| {
                let pane = view.read(app).sidebar_pane.clone();
                pane.update(app, |pane, _| {
                    pane.branch_sidebar_presentation_cached().unwrap().rows
                })
            });
            for (index, row) in rows.iter().enumerate() {
                let selector = match row {
                    branch_sidebar::BranchSidebarRow::WorktreeItem { .. } => {
                        format!("worktree_path_label_{index}")
                    }
                    branch_sidebar::BranchSidebarRow::Branch {
                        section: branch_sidebar::BranchSection::Local,
                        ..
                    } => format!("branch_comparison_label_{index}"),
                    _ => continue,
                };
                let bounds = cx
                    .debug_bounds(Box::leak(selector.clone().into_boxed_str()))
                    .unwrap_or_else(|| panic!("missing {selector}"));
                let row_selector =
                    if matches!(row, branch_sidebar::BranchSidebarRow::WorktreeItem { .. }) {
                        format!("worktree_row_1_{index}")
                    } else {
                        format!("branch_row_1_{index}")
                    };
                let row_bounds = cx
                    .debug_bounds(Box::leak(row_selector.into_boxed_str()))
                    .unwrap();
                // The name fills the first line up to the age column, which
                // sits on the row's trailing edge.
                let age_selector =
                    if matches!(row, branch_sidebar::BranchSidebarRow::WorktreeItem { .. }) {
                        format!("worktree_age_{index}")
                    } else {
                        format!("branch_age_{index}")
                    };
                let age = cx
                    .debug_bounds(Box::leak(age_selector.into_boxed_str()))
                    .unwrap();
                let gap = px(12.0 * percent as f32 / 100.0);
                assert!(
                    bounds.size.width >= row_bounds.size.width * 0.5
                        && bounds.right() <= age.left()
                        && age.left() - bounds.right() <= gap
                        && row_bounds.right() - age.right() <= gap,
                    "{selector} must use the first-line width up to the age column at scale {percent}: {bounds:?}, age {age:?}, row {row_bounds:?}"
                );
                assert!(
                    bounds.left() >= row_bounds.left() && bounds.right() <= row_bounds.right(),
                    "{selector} escaped its row: {bounds:?}, {row_bounds:?}"
                );
                let subject = match row {
                    branch_sidebar::BranchSidebarRow::WorktreeItem { path, .. } => {
                        let badge = cx
                            .debug_bounds(Box::leak(
                                format!("worktree_branch_badge_{index}").into_boxed_str(),
                            ))
                            .unwrap();
                        assert!(
                            bounds.bottom() <= badge.top(),
                            "checkout branch badge must sit below its path"
                        );
                        state.repos[0]
                            .ref_comparisons
                            .projections
                            .worktrees
                            .get(path)
                    }
                    branch_sidebar::BranchSidebarRow::Branch { name, .. } => state.repos[0]
                        .ref_comparisons
                        .projections
                        .branches
                        .get(name.as_ref()),
                    _ => unreachable!(),
                }
                .unwrap();
                let comparison = cx
                    .debug_bounds(Box::leak(
                        format!("ref_comparison_{subject}").into_boxed_str(),
                    ))
                    .unwrap();
                assert!(
                    bounds.bottom() <= comparison.top(),
                    "comparison evidence must sit below the identity"
                );
                assert!(
                    comparison.left() >= bounds.left()
                        && comparison.right() <= row_bounds.right()
                        && comparison.bottom() <= row_bounds.bottom(),
                    "comparison evidence escaped its row"
                );
                if matches!(row, branch_sidebar::BranchSidebarRow::Branch { name, .. } if name.as_ref().starts_with("b-merged"))
                {
                    let pull = cx
                        .debug_bounds(Box::leak(
                            format!("branch_pull_badge_{index}").into_boxed_str(),
                        ))
                        .unwrap();
                    let push = cx
                        .debug_bounds(Box::leak(
                            format!("branch_push_badge_{index}").into_boxed_str(),
                        ))
                        .unwrap();
                    let worktree = cx
                        .debug_bounds(Box::leak(
                            format!("branch_worktree_badge_{index}").into_boxed_str(),
                        ))
                        .unwrap();
                    assert!(
                        comparison.right() <= pull.left()
                            && pull.right() <= push.left()
                            && push.right() <= worktree.left()
                            && worktree.right() <= row_bounds.right(),
                        "upstream/worktree accessories overlap or escape at count {count}: {comparison:?} {pull:?} {push:?} {worktree:?} row {row_bounds:?}"
                    );
                }
            }
        }
    }
}
