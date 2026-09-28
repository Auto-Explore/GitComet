use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::{
    RepoSpec, RepoStatus, StashEntry, Submodule, SubmoduleStatus, Worktree,
};
use std::path::PathBuf;

pub(super) fn fixture(count: usize, section: CollapsedSidebarSection) -> Arc<AppState> {
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
        ..AppState::test_default()
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
            let built = cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    let presentation = pane.branch_sidebar_presentation_cached().unwrap();
                    assert_eq!(presentation.rows.len(), count);
                    assert!(pane.rendered_rows > 0 && pane.rendered_rows < 160);
                    presentation.rows
                })
            });
            assert!(cx.debug_bounds(row_selector(section, 0)).is_some());
            assert!(cx.debug_bounds(row_selector(section, count - 1)).is_none());
            cx.update(|_window, app| pane.update(app, |_pane, cx| cx.notify()));
            test_support::redraw(cx);
            cx.update(|_window, app| {
                pane.update(app, |pane, cx| {
                    assert!(Rc::ptr_eq(
                        &built,
                        &pane.branch_sidebar_presentation_cached().unwrap().rows
                    ));
                    let max = pane.branches_scroll.0.borrow().base_handle.max_offset();
                    assert!(max.y > px(0.0));
                    pane.branches_scroll
                        .0
                        .borrow()
                        .base_handle
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

pub(super) fn branch_fixture(count: usize) -> Arc<AppState> {
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
        ..AppState::test_default()
    })
}

#[gpui::test]
fn scoped_popover_rows_share_density_and_keep_search_fixed(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    let state = branch_fixture(400);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state, cx);
            view.set_sidebar_collapsed(true, cx);
            view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        })
    });
    for density in [
        crate::appearance::UiDensity::Compact,
        crate::appearance::UiDensity::Comfortable,
        crate::appearance::UiDensity::Spacious,
    ] {
        cx.update(|_, app| {
            app.set_global(crate::appearance::Appearance {
                density,
                ..Default::default()
            });
            view.update(app, |view, cx| view.notify_font_preferences_changed(cx));
            pane.update(app, |pane, cx| {
                pane.branch_search_open = true;
                pane.branch_filter_query = "shared/topic".into();
                pane.branches_scroll
                    .scroll_to_item(20, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
        test_support::redraw(cx);
        // Let density anchoring finish before issuing a fresh navigation.
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.branches_scroll
                    .scroll_to_item(20, gpui::ScrollStrategy::Top);
                cx.notify();
            })
        });
        test_support::redraw(cx);
        let search = cx.debug_bounds("sidebar_branches_search").unwrap();
        let rows = cx.update(|_, app| {
            pane.update(app, |pane, _| {
                pane.branch_sidebar_presentation_cached().unwrap().rows
            })
        });
        assert!(!rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                ..
            }
        )));
        let first = cx.debug_bounds("branch_row_81_22").unwrap();
        let second = cx.debug_bounds("branch_row_81_23").unwrap();
        let height = cx.update(|_, app| {
            sidebar_list_row_height(pane.read(app).theme, ui_scale::current(app).percent)
        });
        assert!((second.top() - first.top() - height).abs() < px(0.5));
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.branches_scroll
                    .scroll_to_item(80, gpui::ScrollStrategy::Top);
                cx.notify();
            })
        });
        test_support::redraw(cx);
        assert_eq!(cx.debug_bounds("sidebar_branches_search").unwrap(), search);
    }
}

/// The popover rebuilds its presentation on every frame it is open, so a hit
/// must not copy the persisted sets just to compare them.
#[gpui::test]
fn an_open_popover_reuses_its_rows_without_copying_the_persisted_sets(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
        pane.update(app, |pane, _| {
            assert!(Rc::ptr_eq(
                &rows,
                &pane.branch_sidebar_presentation_cached().unwrap().rows
            ));
            assert_eq!(pane.sidebar_collapsed_items_by_repo[&workdir].len(), 400);
            assert_eq!(pane.sidebar_pinned_branches_by_repo[&workdir].len(), 400);
        });
    });
    assert!(
        allocations < 200,
        "a cache hit allocated {allocations} times against 800 persisted entries"
    );
}

pub(super) fn file_fixture(count: usize) -> Arc<AppState> {
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
        ..AppState::test_default()
    })
}

/// The Files popover holds the most rows of any collapsed-rail section, so it
/// is the one that must not build an element per row.
#[gpui::test]
fn collapsed_files_popover_renders_a_bounded_window(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    pane.file_browser_scroll
                        .0
                        .borrow()
                        .base_handle
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
            cx.debug_bounds("file_browser_scroll_container").is_some(),
            "{count}: popover row band missing"
        );
        assert!(
            cx.debug_bounds("file_browser_row_0").is_some(),
            "{count}: first row missing"
        );

        let max = cx.update(|_window, app| {
            pane.read(app)
                .file_browser_scroll
                .0
                .borrow()
                .base_handle
                .max_offset()
                .y
        });
        assert!(max > px(0.0), "{count} files must overflow the popover");
        cx.update(|_window, app| {
            pane.update(app, |pane, cx| {
                pane.file_browser_scroll
                    .0
                    .borrow()
                    .base_handle
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
