use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::{
    RepoSpec, RepoStatus, StashEntry, Submodule, SubmoduleStatus, Worktree,
};
use std::path::PathBuf;

fn fixture(count: usize, section: CollapsedSidebarSection) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(81),
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-long-sidebar"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());
    repo.status = Loadable::Ready(Arc::new(RepoStatus::default()));
    repo.worktrees = Loadable::Ready(Arc::new(if section == CollapsedSidebarSection::Worktrees {
        (0..count)
            .map(|ix| Worktree {
                path: PathBuf::from(format!("/tmp/worktree-{ix:06}")),
                head: None,
                branch: None,
                detached: true,
            })
            .collect()
    } else {
        Vec::new()
    }));
    repo.submodules = Loadable::Ready(Arc::new(
        if section == CollapsedSidebarSection::Submodules {
            (0..count)
                .map(|ix| Submodule {
                    path: PathBuf::from(format!("vendor/submodule-{ix:06}")),
                    recorded_head: CommitId("a".into()),
                    checked_out_head: Some(CommitId("a".into())),
                    status: SubmoduleStatus::UpToDate,
                })
                .collect()
        } else {
            Vec::new()
        },
    ));
    repo.stashes = Loadable::Ready(Arc::new(if section == CollapsedSidebarSection::Stashes {
        (0..count)
            .map(|index| StashEntry {
                index,
                id: CommitId(format!("{index:040x}").into()),
                message: format!("stash {index}").into(),
                created_at: None,
            })
            .collect()
    } else {
        Vec::new()
    }));
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..Default::default()
    })
}

fn row_selector(section: CollapsedSidebarSection, index: usize) -> &'static str {
    // GPUI's test bounds API requires static selectors; only the first and last
    // selectors of the nine fixtures are retained here.
    let selector = match section {
        CollapsedSidebarSection::Submodules => format!("submodule_label_{index}"),
        CollapsedSidebarSection::Worktrees => format!("worktree_row_81_{index}"),
        CollapsedSidebarSection::Stashes => format!("stash_sidebar_row_{index}"),
        _ => unreachable!(),
    };
    selector.leak()
}

#[gpui::test]
fn auxiliary_sidebar_lists_and_popups_keep_rendering_bounded(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    for count in [1_000, 10_000, 50_000] {
        for section in [
            CollapsedSidebarSection::Submodules,
            CollapsedSidebarSection::Worktrees,
            CollapsedSidebarSection::Stashes,
        ] {
            let state = fixture(count, section);
            cx.update(|_window, app| {
                view.update(app, |view, cx| {
                    view.store.replace_snapshot_for_test(Arc::clone(&state));
                    test_support::push_test_state(view, Arc::clone(&state), cx);
                    view.set_sidebar_collapsed(false, cx);
                    view.sidebar_pane.update(cx, |pane, cx| {
                        let mut collapsed = BTreeSet::new();
                        branch_sidebar::set_collapse_state(
                            &mut collapsed,
                            section.storage_key().unwrap(),
                            false,
                        );
                        pane.sidebar_collapsed_items_by_repo
                            .insert(state.repos[0].spec.workdir.clone(), collapsed);
                        pane.sidebar_presentation_cache = SidebarPresentationCache::default();
                        pane.branches_scroll
                            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                })
            });
            test_support::redraw(cx);
            cx.update(|_window, app| {
                let pane = pane.read(app);
                assert!(
                    pane.rendered_rows > 0 && pane.rendered_rows < 160,
                    "expanded {section:?} / {count}: {} rows",
                    pane.rendered_rows
                );
            });
            cx.update(|_window, app| {
                view.update(app, |view, cx| {
                    view.set_sidebar_collapsed(true, cx);
                    view.open_sidebar_collapsed_popover(section, cx);
                })
            });
            test_support::redraw(cx);
            let built = cx.update(|_window, app| {
                let pane = pane.read(app);
                let cached = pane
                    .collapsed_popover_rows_cache
                    .as_ref()
                    .expect("popup cache");
                assert_eq!(cached.rows.len(), count);
                assert!(
                    pane.rendered_rows > 0 && pane.rendered_rows < 160,
                    "popup {section:?} / {count}: {} rows",
                    pane.rendered_rows
                );
                Rc::clone(&cached.rows)
            });
            assert!(cx.debug_bounds(row_selector(section, 0)).is_some());
            assert!(cx.debug_bounds(row_selector(section, count - 1)).is_none());
            cx.update(|_window, app| pane.update(app, |_pane, cx| cx.notify()));
            test_support::redraw(cx);
            cx.update(|_window, app| {
                pane.update(app, |pane, cx| {
                    assert!(Rc::ptr_eq(
                        &built,
                        &pane.collapsed_popover_rows_cache.as_ref().unwrap().rows
                    ));
                    let max = pane.collapsed_popover_scroll.max_offset();
                    assert!(max.y > px(0.0));
                    pane.collapsed_popover_scroll
                        .set_offset(point(px(0.0), -max.y));
                    cx.notify();
                })
            });
            test_support::redraw(cx);
            assert!(
                cx.debug_bounds(row_selector(section, count - 1)).is_some(),
                "last {section:?} / {count} row must render after scrolling"
            );
            assert!(cx.debug_bounds(row_selector(section, 0)).is_none());
            cx.simulate_resize(gpui::size(px(1000.0), px(680.0)));
            test_support::redraw(cx);
            cx.update(|_window, app| assert!(pane.read(app).rendered_rows < 160));
        }
    }
}

fn branch_fixture(count: usize) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(81),
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-long-sidebar"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());
    repo.status = Loadable::Ready(Arc::new(RepoStatus::default()));
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.branches = Loadable::Ready(Arc::new(
        (0..count)
            .map(|ix| gitcomet_core::domain::Branch {
                name: format!("shared/topic-{ix:06}"),
                target: CommitId("a".into()),
                upstream: None,
                divergence: None,
            })
            .collect(),
    ));
    repo.remotes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Remote {
        name: "origin".to_string(),
        url: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(
        (0..count)
            .map(|ix| gitcomet_core::domain::RemoteBranch {
                remote: "origin".to_string(),
                name: format!("shared/topic-{ix:06}"),
                target: CommitId("a".into()),
            })
            .collect(),
    ));
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..Default::default()
    })
}

/// A cross-section filter is the only case that mixes the 8px spacer into the
/// 24px rows, so it is the only one that exercises the mixed-height prefix sum.
#[gpui::test]
fn cross_section_filter_popover_places_rows_using_both_row_heights(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    let state = branch_fixture(400);

    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(true, cx);
            view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        pane.update(app, |pane, cx| {
            pane.collapsed_popover_filter_open = true;
            pane.collapsed_popover_filter_query = "shared/topic".to_string();
            pane.collapsed_popover_rows_cache = None;
            cx.notify();
        })
    });
    test_support::redraw(cx);

    let (spacer_ix, next_branch_ix, tops, row_count) = cx.update(|_window, app| {
        let pane = pane.read(app);
        let cache = pane
            .collapsed_popover_rows_cache
            .as_ref()
            .expect("popover rows cached");
        let spacer_ix = cache
            .rows
            .iter()
            .position(|row| matches!(row, BranchSidebarRow::SectionSpacer))
            .unwrap_or_else(|| {
                let repo = &pane.state.repos[0];
                let full = branch_sidebar::branch_sidebar_rows(
                    repo,
                    &BTreeSet::new(),
                    &Default::default(),
                    "shared/topic",
                );
                let kinds: Vec<String> = full
                    .iter()
                    .filter(|r| !matches!(r, BranchSidebarRow::Branch { .. }))
                    .map(|r| format!("{r:?}").chars().take(60).collect())
                    .collect();
                panic!("no spacer; full non-branch rows: {kinds:#?}")
            });
        let next_branch_ix = cache.rows[spacer_ix + 1..]
            .iter()
            .position(|row| matches!(row, BranchSidebarRow::Branch { .. }))
            .map(|offset| spacer_ix + 1 + offset)
            .expect("a branch row after the spacer");
        (
            spacer_ix,
            next_branch_ix,
            cache.tops.clone(),
            cache.rows.len(),
        )
    });

    assert_eq!(tops.len(), row_count + 1);
    assert_eq!(
        tops[spacer_ix + 1] - tops[spacer_ix],
        crate::view::rows::sidebar::BRANCH_TREE_SPACER_HEIGHT_PX,
        "the spacer must be measured with the spacer height"
    );
    assert_eq!(
        tops[spacer_ix] - tops[spacer_ix - 1],
        crate::view::rows::sidebar::BRANCH_TREE_ROW_HEIGHT_PX,
        "the row before it must be measured with the row height"
    );

    cx.update(|_window, app| {
        pane.update(app, |pane, cx| {
            let top = crate::ui_scale::UiScale::current(cx).px(tops[spacer_ix - 1]);
            pane.collapsed_popover_scroll
                .set_offset(gpui::point(px(0.0), -top));
            cx.notify();
        })
    });
    test_support::redraw(cx);

    let before: &'static str = format!("branch_row_81_{}", spacer_ix - 1).leak();
    let after: &'static str = format!("branch_row_81_{next_branch_ix}").leak();
    let before_bounds = cx.debug_bounds(before).expect("row before the spacer");
    let after_bounds = cx
        .debug_bounds(after)
        .expect("first branch row after the spacer");
    let measured = f32::from(after_bounds.origin.y - before_bounds.origin.y);
    let predicted = tops[next_branch_ix] - tops[spacer_ix - 1];
    assert!(
        (measured - predicted).abs() < 0.5,
        "tops predicted {predicted}px between rows {} and {next_branch_ix}, laid out {measured}px",
        spacer_ix - 1
    );

    cx.update(|_window, app| {
        assert!(
            pane.read(app).rendered_rows > 0 && pane.read(app).rendered_rows < 160,
            "filtered popover must stay virtualized: {} rows",
            pane.read(app).rendered_rows
        );
    });
}

/// Every row kind has to lay out at the height `branch_sidebar_row_height_px`
/// claims. Measured across a window spanning headers and the section spacer:
/// one wrong height in between moves every row after it.
#[gpui::test]
fn collapsed_popover_row_heights_match_what_is_laid_out(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    let state = branch_fixture(400);

    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(true, cx);
            view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        pane.update(app, |pane, cx| {
            pane.collapsed_popover_filter_open = true;
            pane.collapsed_popover_filter_query = "shared/topic".to_string();
            pane.collapsed_popover_rows_cache = None;
            cx.notify();
        })
    });
    test_support::redraw(cx);

    let (rows, tops) = cx.update(|_window, app| {
        let pane = pane.read(app);
        let cache = pane
            .collapsed_popover_rows_cache
            .as_ref()
            .expect("popover rows cached");
        (Rc::clone(&cache.rows), cache.tops.clone())
    });
    let spacer_ix = rows
        .iter()
        .position(|row| matches!(row, BranchSidebarRow::SectionSpacer))
        .expect("a cross-section filter separates the two sections with a spacer");

    // Park the window over the spacer, where the mixed kinds are.
    cx.update(|_window, app| {
        pane.update(app, |pane, cx| {
            let top = crate::ui_scale::UiScale::current(cx).px(tops[spacer_ix.saturating_sub(4)]);
            pane.collapsed_popover_scroll
                .set_offset(gpui::point(px(0.0), -top));
            cx.notify();
        })
    });
    test_support::redraw(cx);

    // Two branch rows either side of the spacer, chosen from the row list so the
    // probe leaks exactly two selectors (`debug_bounds` takes `&'static str`).
    let is_branch = |ix: &usize| matches!(rows[*ix], BranchSidebarRow::Branch { .. });
    let first_ix = (0..spacer_ix)
        .rev()
        .find(is_branch)
        .expect("a branch row before the spacer");
    let last_ix = (spacer_ix + 1..rows.len())
        .filter(is_branch)
        .nth(14)
        .expect("branch rows after the spacer");
    let bounds_of = |cx: &mut gpui::VisualTestContext, ix: usize| {
        let selector: &'static str = format!("branch_row_81_{ix}").leak();
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("row {ix} must be inside the rendered window"))
    };
    let first = bounds_of(cx, first_ix);
    let last = bounds_of(cx, last_ix);
    let covered: Vec<&BranchSidebarRow> = rows[first_ix..last_ix].iter().collect();
    assert!(
        covered
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::FilterGroupHeader { .. })),
        "and a header, so the check is not only about branch rows"
    );

    let measured = f32::from(last.origin.y - first.origin.y);
    let predicted = tops[last_ix] - tops[first_ix];
    assert!(
        (measured - predicted).abs() < 0.5,
        "rows {first_ix}..{last_ix} ({} of them, kinds {:?}) predicted {predicted}px, laid out {measured}px",
        covered.len(),
        covered
            .iter()
            .map(|row| format!("{row:?}").chars().take(24).collect::<String>())
            .collect::<std::collections::BTreeSet<_>>(),
    );
}

/// The popover rebuilds its presentation on every frame it is open, so a hit
/// must not copy the persisted sets just to compare them.
#[gpui::test]
fn an_open_popover_reuses_its_rows_without_copying_the_persisted_sets(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    let state = branch_fixture(400);
    let workdir = state.repos[0].spec.workdir.clone();

    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(true, cx);
            view.sidebar_pane.update(cx, |pane, _cx| {
                pane.sidebar_collapsed_items_by_repo.insert(
                    workdir.clone(),
                    (0..400).map(|ix| format!("group:feat/{ix:06}")).collect(),
                );
                pane.sidebar_pinned_branches_by_repo.insert(
                    workdir.clone(),
                    (0..400).map(|ix| format!("shared/topic-{ix:06}")).collect(),
                );
            });
            view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        })
    });
    test_support::redraw(cx);

    let (rows, allocations) = cx.update(|_window, app| {
        pane.update(app, |pane, _cx| {
            let mut rows = None;
            let mut fewest = u64::MAX;
            // `measure_allocations` watches a process-global allocator, so a
            // parallel test can only ever inflate a sample: take the smallest.
            for _ in 0..3 {
                let (presentation, metrics) = crate::perf_alloc::measure_allocations(|| {
                    pane.build_collapsed_popover_presentation(CollapsedSidebarSection::Local)
                });
                fewest = fewest.min(metrics.alloc_ops);
                rows = presentation.map(|presentation| presentation.rows);
            }
            (rows.expect("a popover presentation"), fewest)
        })
    });

    cx.update(|_window, app| {
        let cached = pane.read(app);
        let cached = cached
            .collapsed_popover_rows_cache
            .as_ref()
            .expect("popover rows cached");
        assert!(
            Rc::ptr_eq(&rows, &cached.rows),
            "a repeat frame must hand back the rows it already built"
        );
        assert_eq!(cached.collapsed.len(), 400);
        assert_eq!(cached.pinned.len(), 400);
    });
    assert!(
        allocations < 200,
        "a cache hit allocated {allocations} times against 800 persisted entries"
    );
}

fn file_fixture(count: usize) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(81),
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-long-sidebar"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());
    repo.status = Loadable::Ready(Arc::new(RepoStatus::default()));
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.file_browser.active = true;
    repo.file_browser.entries = Loadable::Ready(Arc::new(
        (0..count)
            .map(|ix| gitcomet_core::domain::FileEntry {
                name: format!("file_{ix:06}.txt"),
                path: Arc::new(PathBuf::from(format!("file_{ix:06}.txt"))),
                kind: gitcomet_core::domain::FileEntryKind::File,
                depth: 0,
            })
            .collect(),
    ));
    repo.file_browser.bump_rev();
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..Default::default()
    })
}

/// The Files popover holds the most rows of any collapsed-rail section, so it
/// is the one that must not build an element per row.
#[gpui::test]
fn collapsed_files_popover_renders_a_bounded_window(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());

    for count in [1_000, 50_000] {
        let state = file_fixture(count);
        cx.update(|_window, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(Arc::clone(&state));
                test_support::push_test_state(view, Arc::clone(&state), cx);
                view.set_sidebar_collapsed(true, cx);
                view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Files, cx);
                view.sidebar_pane.update(cx, |pane, cx| {
                    pane.collapsed_popover_scroll
                        .set_offset(point(px(0.0), px(0.0)));
                    cx.notify();
                });
            })
        });
        test_support::redraw(cx);

        cx.update(|_window, app| {
            let rendered = pane.read(app).rendered_rows;
            assert!(
                rendered > 0 && rendered < 160,
                "{count} files rendered {rendered} rows"
            );
        });
        assert!(
            cx.debug_bounds("collapsed_file_browser_rows").is_some(),
            "{count}: popover row band missing"
        );
        assert!(
            cx.debug_bounds("file_browser_row_0").is_some(),
            "{count}: first row missing"
        );

        let max = cx.update(|_window, app| pane.read(app).collapsed_popover_scroll.max_offset().y);
        assert!(max > px(0.0), "{count} files must overflow the popover");
        cx.update(|_window, app| {
            pane.update(app, |pane, cx| {
                pane.collapsed_popover_scroll
                    .set_offset(point(px(0.0), -max));
                cx.notify();
            })
        });
        test_support::redraw(cx);

        assert!(
            cx.debug_bounds("file_browser_row_0").is_none(),
            "{count}: the first row must leave the window after scrolling to the end"
        );
        cx.update(|_window, app| {
            let rendered = pane.read(app).rendered_rows;
            assert!(
                rendered > 0 && rendered < 160,
                "{count} files rendered {rendered} rows at the end"
            );
        });
    }
}
