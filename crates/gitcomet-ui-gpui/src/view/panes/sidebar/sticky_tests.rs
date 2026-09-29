use super::*;
use crate::view::test_support::{self, TestBackend};

fn scroll(cx: &mut gpui::VisualTestContext, position: Point<Pixels>, delta: Point<Pixels>) {
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(delta),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
}

fn fixture(count: usize) -> Arc<AppState> {
    let mut state = long_list_tests::branch_fixture(count);
    let state_mut = Arc::make_mut(&mut state);
    state_mut.repos[0].head_branch = Loadable::Ready("shared/topic-000000".into());
    state
}

fn insert_branch(state: &mut Arc<AppState>, name: &str) {
    let repo = &mut Arc::make_mut(state).repos[0];
    if let Loadable::Ready(branches) = &mut repo.branches {
        let mut branch = branches[0].clone();
        branch.name = name.into();
        Arc::make_mut(branches).push(branch);
    }
    repo.branches_rev += 1;
}

fn selector(ix: usize) -> &'static str {
    format!("sidebar_sticky_header_{ix}").leak()
}

#[gpui::test]
fn review_selected_pin_falls_back_to_tree_when_its_copy_disappears(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(10);
    Arc::make_mut(&mut state).repos[0]
        .history_state
        .selected_commit = Some(CommitId("a".into()));
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    for removal in ["unpin", "collapse", "filter"] {
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                let target = BranchMenuTarget::local("shared/topic-000001");
                pane.sidebar_pinned_branches_by_repo.insert(
                    state.repos[0].spec.workdir.clone(),
                    BTreeSet::from(["group:local:shared".into()]),
                );
                pane.set_collapsed_keys_for_test(&[]);
                pane.branch_filter_query.clear();
                let presentation = pane.branch_sidebar_presentation_cached().unwrap();
                let ix = presentation
                    .pins
                    .iter()
                    .position(|row| {
                        matches!(
                            row, BranchSidebarRow::Branch { target: t, .. } if *t == target
                        )
                    })
                    .unwrap();
                pane.set_selected_branch(
                    state.repos[0].id,
                    target.clone(),
                    Some(presentation.row_keys[ix].clone()),
                    cx,
                );
                assert!(pane.selected_branch_for_row(None).is_none());
                match removal {
                    "unpin" => {
                        pane.toggle_sidebar_pin(state.repos[0].id, "group:local:shared".into(), cx)
                    }
                    "collapse" => pane.set_collapsed_keys_for_test(&["group:local:shared"]),
                    _ => pane.branch_filter_query = "no matching branches".into(),
                }
                pane.branch_sidebar_presentation_cached().unwrap();
                assert!(
                    pane.selected_branch_pin_key.is_none(),
                    "stale copy after {removal}"
                );
                assert_eq!(pane.selected_branch_for_row(None).unwrap().target, target);

                pane.set_collapsed_keys_for_test(&[]);
                pane.branch_filter_query.clear();
                let presentation = pane.branch_sidebar_presentation_cached().unwrap();
                let ix = presentation
                    .rows
                    .iter()
                    .rposition(|row| {
                        matches!(
                            row, BranchSidebarRow::Branch { target: t, .. } if *t == target
                        )
                    })
                    .unwrap();
                assert!(
                    pane.sticky_context
                        .as_ref()
                        .unwrap()
                        .eligible_rows
                        .contains(&ix),
                    "restored tree selection after {removal}"
                );
            })
        });
    }
}

#[gpui::test]
fn review_rail_popovers_preserve_expanded_tree_scroll(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = fixture(1_000);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state, cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.branches_scroll
                .scroll_to_item_strict(500, gpui::ScrollStrategy::Top);
            cx.notify();
        })
    });
    test_support::redraw(cx);
    let before = cx.update(|_, app| {
        pane.read(app)
            .branches_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    assert!(before.y < px(-1_000.0));
    for section in [
        CollapsedSidebarSection::Local,
        CollapsedSidebarSection::Remote,
    ] {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.set_sidebar_collapsed(true, cx);
                view.open_sidebar_collapsed_popover(section, cx);
            })
        });
        test_support::redraw(cx);
    }
    cx.update(|_, app| view.update(app, |view, cx| view.set_sidebar_collapsed(false, cx)));
    test_support::redraw(cx);
    cx.update(|_, app| {
        assert_eq!(
            pane.read(app)
                .branches_scroll
                .0
                .borrow()
                .base_handle
                .offset(),
            before
        )
    });

    // A repository switch must reset the parked tree as well as the popover.
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_sidebar_collapsed(true, cx);
            view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        })
    });
    test_support::redraw(cx);
    let mut next = fixture(1_000);
    let next_state = Arc::make_mut(&mut next);
    next_state.repos[0].id = RepoId(82);
    next_state.active_repo = Some(RepoId(82));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(next.clone());
            test_support::push_test_state(view, next, cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| view.update(app, |view, cx| view.set_sidebar_collapsed(false, cx)));
    test_support::redraw(cx);
    cx.update(|_, app| {
        assert_eq!(
            pane.read(app)
                .branches_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y,
            px(0.0)
        )
    });
}

#[gpui::test]
fn review_rail_group_rows_use_the_popover_surface(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = fixture(4);
    let theme = AppTheme::from_key("sunset_veil").unwrap();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state, cx);
            view.set_theme(theme, cx);
            view.set_sidebar_collapsed(true, cx);
            view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        })
    });
    test_support::redraw(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    test_support::redraw(cx);
    let fills = crate::test_support::painted_control_quads(cx, "branch_group_0");
    // Rail rows inherit the popover's raised surface. Compare RGB independently
    // of the opening animation's opacity so a faint chrome band also fails.
    assert!(
        !fills.iter().any(|(fill, _)| {
            let Some(mut color) = fill.as_solid() else {
                return false;
            };
            color.alpha = 1.0;
            gpui::Background::from(color) == theme.colors.surface.chrome.into()
        }),
        "expanded-tree bands were painted in the rail: {fills:?}"
    );
}

#[gpui::test]
fn sticky_sidebar_headers_scroll_navigate_and_respect_group_collapse(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(1_000);
    Arc::make_mut(&mut state).repos[0].worktrees =
        long_list_tests::fixture(40, CollapsedSidebarSection::Worktrees).repos[0]
            .worktrees
            .clone();
    Arc::make_mut(&mut state).repos[0].stashes =
        long_list_tests::fixture(100, CollapsedSidebarSection::Stashes).repos[0]
            .stashes
            .clone();
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let (headers, worktrees, stashes, group, rows) = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            let presentation = pane.branch_sidebar_presentation_cached().unwrap();
            assert_eq!(presentation.structure.sections.len(), 5);
            let group = presentation.structure.headers
                [branch_sidebar::local_group_storage_key("shared").as_str()];
            assert!(
                pane.sticky_context
                    .as_ref()
                    .unwrap()
                    .eligible_rows
                    .contains(&group)
            );
            (
                pane.sticky_context.as_ref().unwrap().eligible_rows.clone(),
                presentation.structure.headers[branch_sidebar::worktrees_section_storage_key()],
                presentation.structure.headers[branch_sidebar::stash_section_storage_key()],
                group,
                presentation.rows.clone(),
            )
        })
    });
    let mut bottom = px(0.0);
    for ix in headers.iter() {
        let bounds = cx
            .debug_bounds(selector(*ix))
            .expect("every eligible header is painted");
        assert!(bounds.top() >= bottom);
        bottom = bounds.bottom();
    }
    // Wheel input over a sticky header must reach the one main scroll surface.
    let local = cx.debug_bounds(selector(0)).unwrap();
    scroll(cx, local.center(), point(px(0.0), px(-800.0)));
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            assert!(pane.branches_scroll.0.borrow().base_handle.offset().y < px(0.0));
            assert!(Rc::ptr_eq(
                &rows,
                &pane.branch_sidebar_presentation_cached().unwrap().rows
            ));
        })
    });
    let stash = cx.debug_bounds(selector(stashes)).unwrap();
    cx.simulate_click(stash.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    let after_stash = cx.update(|_, app| {
        pane.read(app)
            .branches_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y
    });
    assert!(after_stash < px(-20_000.0));
    let worktree = cx.debug_bounds(selector(worktrees)).unwrap();
    cx.simulate_click(worktree.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    assert!(
        cx.update(|_, app| pane
            .read(app)
            .branches_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y)
            > after_stash
    );
    // The clicked heading keeps focus as it moves. Enter uses the same
    // offset-aware navigation as a pointer click.
    let worktree_offset = cx.update(|_, app| {
        pane.read(app)
            .branches_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.branches_scroll
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.0), px(0.0)));
            cx.notify();
        })
    });
    test_support::redraw(cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    test_support::redraw(cx);
    assert_eq!(
        cx.update(|_, app| pane
            .read(app)
            .branches_scroll
            .0
            .borrow()
            .base_handle
            .offset()),
        worktree_offset
    );
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.navigate_sidebar_header(
                branch_sidebar::local_group_storage_key("shared").into(),
                cx,
            );
        })
    });
    test_support::redraw(cx);
    let toggle = cx
        .debug_bounds(format!("sidebar_group_toggle_{group}").leak())
        .unwrap();
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            let presentation = pane.branch_sidebar_presentation_cached().unwrap();
            let group = presentation.structure.headers
                [branch_sidebar::local_group_storage_key("shared").as_str()];
            assert!(
                !pane
                    .sticky_context
                    .as_ref()
                    .unwrap()
                    .eligible_rows
                    .contains(&group)
            );
            assert!(branch_sidebar::is_collapsed(
                &pane.collapsed_items_for_test(),
                &branch_sidebar::local_group_storage_key("shared")
            ));
        })
    });
}

#[gpui::test]
fn sticky_sidebar_section_gaps_close_when_headers_stick(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(1_000);
    Arc::make_mut(&mut state).repos[0].worktrees =
        long_list_tests::fixture(40, CollapsedSidebarSection::Worktrees).repos[0]
            .worktrees
            .clone();
    Arc::make_mut(&mut state).repos[0].stashes =
        long_list_tests::fixture(100, CollapsedSidebarSection::Stashes).repos[0]
            .stashes
            .clone();
    let repo_id = state.repos[0].id.0;
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    cx.simulate_resize(gpui::size(px(1100.0), px(1100.0)));
    test_support::redraw(cx);
    let (rows, sections, remote, row_height) = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            let handle = pane.branches_scroll.0.borrow();
            let row_height = handle.last_item_size.unwrap().contents.height / p.rows.len() as f32;
            (
                p.rows.clone(),
                p.structure.sections.clone(),
                p.structure.headers[branch_sidebar::remote_section_storage_key()],
                row_height,
            )
        })
    });
    assert_eq!(sections.len(), 5);
    for section in &sections[1..] {
        assert!(matches!(rows[section - 1], BranchSidebarRow::SectionSpacer));
    }
    let set_offset = |cx: &mut gpui::VisualTestContext, offset: Pixels| {
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.branches_scroll
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(point(px(0.0), -offset));
                cx.notify();
            })
        });
        test_support::redraw(cx);
    };
    // Overlay rows in stack order: (row, painted bounds, natural top).
    let sticky = |cx: &mut gpui::VisualTestContext| {
        let (decorated, natural) = cx.update(|_, app| {
            let pane = pane.read(app);
            let handle = pane.branches_scroll.0.borrow();
            (
                pane.decorated_sidebar_rows(),
                handle.base_handle.bounds().top() + handle.base_handle.offset().y,
            )
        });
        let mut painted = Vec::new();
        for ix in decorated.iter().copied() {
            let bounds = cx.debug_bounds(selector(ix)).unwrap();
            painted.push((ix, bounds, natural + row_height * ix));
        }
        painted
    };
    let near = |a: Pixels, b: Pixels| (a - b).abs() < px(0.5);
    // A gap slot slides under the edge stacks: stuck neighbours always touch.
    let assert_stacks_touch = |painted: &[(usize, Bounds<Pixels>, Pixels)]| {
        for pair in painted.windows(2) {
            let (a, above, above_natural) = pair[0];
            let (b, below, below_natural) = pair[1];
            if below.top() > below_natural + px(0.5) || above.top() < above_natural - px(0.5) {
                assert!(near(below.top(), above.bottom()), "gap between {a} and {b}");
            }
        }
    };

    // At the top, every later section waits in the bottom stack.
    let painted = sticky(cx);
    assert_stacks_touch(&painted);
    for (ix, bounds, natural) in &painted {
        if sections[1..].contains(ix) {
            assert!(
                bounds.top() < *natural - px(0.5),
                "section {ix} is not stuck"
            );
        }
    }

    // Navigating to the last section stacks every header above it, gap-free.
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.navigate_sidebar_header(branch_sidebar::stash_section_storage_key().into(), cx)
        })
    });
    test_support::redraw(cx);
    let painted = sticky(cx);
    let list_top = cx.update(|_, app| {
        pane.read(app)
            .branches_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
            .top()
    });
    assert!(near(painted[0].1.top(), list_top));
    for pair in painted.windows(2) {
        assert!(near(pair[1].1.top(), pair[0].1.bottom()));
    }
    assert_eq!(painted.last().unwrap().0, sections[4]);

    // Scrolling along, a section shows one blank slot above its header.
    let last_local = remote - 2;
    assert!(matches!(rows[last_local], BranchSidebarRow::Branch { .. }));
    set_offset(cx, row_height * remote - px(400.0));
    let painted = sticky(cx);
    assert_stacks_touch(&painted);
    let (_, header, natural) = *painted.iter().find(|(ix, ..)| *ix == remote).unwrap();
    assert!(near(header.top(), natural));
    let branch = cx
        .debug_bounds(format!("branch_row_{repo_id}_{last_local}").leak())
        .unwrap();
    assert!(near(header.top() - branch.bottom(), row_height));

    // Leaving the stack, the gap grows from zero rather than jumping.
    let rank = painted.iter().position(|(ix, ..)| *ix == remote).unwrap();
    for (natural_y, gap) in [(rank as f32 - 0.5, 0.0), (rank as f32 + 0.5, 0.5)] {
        set_offset(cx, row_height * remote - row_height * natural_y);
        let painted = sticky(cx);
        assert_eq!(painted[rank].0, remote);
        assert!(
            near(
                painted[rank].1.top() - painted[rank - 1].1.bottom(),
                row_height * gap
            ),
            "gap at natural y {natural_y}"
        );
    }
}

#[gpui::test]
fn sticky_sidebar_pins_share_scrolling_filtering_and_virtualized_overflow(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = fixture(10_000);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
            view.sidebar_pane.update(cx, |pane, cx| {
                pane.sidebar_pinned_branches_by_repo.insert(
                    state.repos[0].spec.workdir.clone(),
                    (0..1_000)
                        .flat_map(|ix| {
                            [
                                format!("local:shared/topic-{ix:06}"),
                                format!("remote:origin/shared/topic-{ix:06}"),
                            ]
                        })
                        .collect(),
                );
                pane.sidebar_presentation_cache = SidebarPresentationCache::default();
                pane.sync_popover_pinned_branches(cx);
                cx.notify();
            });
        })
    });
    test_support::redraw(cx);
    let body = cx.debug_bounds("branch_sidebar_scroll_container").unwrap();
    assert!(
        cx.debug_bounds("sidebar_pinned_area").is_none(),
        "no nested pin viewport"
    );
    let pins = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            assert!(pane.rendered_rows < 200);
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            assert_eq!(p.pins.len(), 2_000);
            assert_eq!(&p.rows[..p.pins.len()], p.pins.as_ref());
            p.pins
        })
    });
    assert!(cx.debug_bounds("sidebar_pin_marker_0").is_some());
    assert!(cx.debug_bounds("sidebar_more_pins_bottom").is_some());
    scroll(cx, body.center(), point(px(0.0), px(-800.0)));
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            assert!(pane.branches_scroll.0.borrow().base_handle.offset().y < px(0.0));
            assert!(Rc::ptr_eq(
                &pane.branch_sidebar_presentation_cached().unwrap().pins,
                &pins
            ));
            assert!(pane.rendered_rows < 200);
        })
    });
    let overflow = cx.debug_bounds("sidebar_more_pins_top").unwrap();
    cx.simulate_click(overflow.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    assert!(cx.update(|_, app| matches!(
        test_support::popover_kind(view.read(app), app),
        Some(PopoverKind::SidebarPinnedOverflow { bottom: false, .. })
    )));
    let entry = cx.debug_bounds("pin_overflow_entry_0").unwrap();
    cx.simulate_click(entry.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("sidebar_pin_marker_0").is_some(),
        "overflow reveals the natural pin"
    );
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.branch_filter_query = "no matching refs".into();
            pane.sticky_context = None;
            cx.notify();
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            assert!(p.pins.is_empty());
            assert!(
                !p.rows
                    .iter()
                    .any(|row| matches!(row, BranchSidebarRow::Branch { .. }))
            );
            assert_eq!(
                pane.pinned_branches_for_test().len(),
                2_000,
                "filtering preserves stored pins"
            );
        })
    });
}

#[gpui::test]
fn sticky_sidebar_pinned_branch_interactions_highlight_only_the_clicked_copy(
    cx: &mut gpui::TestAppContext,
) {
    use crate::test_support::painted_control_quads as paint;
    use gitcomet_core::domain::{Commit, LogPage};

    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(4);
    let repo = &mut Arc::make_mut(&mut state).repos[0];
    repo.history_state.selected_commit = Some(CommitId("a".into()));
    repo.log = Loadable::Ready(
        Arc::new(LogPage {
            commits: vec![Commit {
                id: CommitId("a".into()),
                parent_ids: Default::default(),
                summary: "Pinned branch tip".into(),
                author: "Test".into(),
                time: std::time::SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        })
        .into(),
    );
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
            view.sidebar_pane.update(cx, |pane, cx| {
                pane.sidebar_pinned_branches_by_repo.insert(
                    state.repos[0].spec.workdir.clone(),
                    BTreeSet::from([
                        "group:local:shared".into(),
                        "group:remote:origin:shared".into(),
                        "local:shared/topic-000001".into(),
                        "remote:origin/shared/topic-000001".into(),
                    ]),
                );
                pane.sidebar_presentation_cache = SidebarPresentationCache::default();
                cx.notify();
            });
        })
    });
    cx.simulate_resize(gpui::size(px(1100.0), px(1100.0)));
    test_support::redraw(cx);
    let theme = cx.update(|_, app| pane.read(app).theme);
    let selected_bg = crate::view::selected_branch_row_bg(theme).into();
    for target in [
        BranchMenuTarget::local("shared/topic-000001"),
        BranchMenuTarget::remote("origin", "shared/topic-000001"),
    ] {
        let (group_member_ix, pin_ix, tree_ix, rows) = cx.update(|_, app| {
            pane.update(app, |pane, _| {
                let p = pane.branch_sidebar_presentation_cached().unwrap();
                let matching = |row: &BranchSidebarRow| {
                    matches!(row, BranchSidebarRow::Branch { target: candidate, .. } if candidate == &target)
                };
                (
                    p.pins.iter().position(matching).unwrap(),
                    p.pins.iter().rposition(matching).unwrap(),
                    p.rows.iter().rposition(matching).unwrap(),
                    p.rows,
                )
            })
        });
        assert_ne!(group_member_ix, pin_ix);
        let group_member_selector: &'static str =
            format!("pinned_branch_row_81_{group_member_ix}").leak();
        let pin_selector: &'static str = format!("pinned_branch_row_81_{pin_ix}").leak();
        let tree_selector: &'static str = format!("branch_row_81_{tree_ix}").leak();
        let tree_resting = paint(cx, tree_selector);
        let pin = cx.debug_bounds(pin_selector).unwrap();
        cx.simulate_mouse_down(pin.center(), MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_up(pin.center(), MouseButton::Right, gpui::Modifiers::default());
        test_support::redraw(cx);
        assert!(
            paint(cx, pin_selector)
                .iter()
                .any(|(fill, _)| *fill == theme.active_overlay().into())
        );
        assert_eq!(
            paint(cx, tree_selector),
            tree_resting,
            "opening a pin's menu must not highlight its tree copy"
        );
        assert!(cx.debug_bounds("sidebar_sticky_selected_branch").is_none());
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.popover_host
                    .update(cx, |host, cx| host.close_popover(cx));
            });
        });
        test_support::redraw(cx);

        // Repeatedly switch between copies of the SAME branch. The commit and
        // cached rows stay identical; selection must follow the actual click.
        for clicked_selector in [
            pin_selector,
            group_member_selector,
            pin_selector,
            tree_selector,
            group_member_selector,
            pin_selector,
        ] {
            let bounds = cx.debug_bounds(clicked_selector).unwrap();
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.simulate_mouse_move(
                point(px(1000.0), px(1000.0)),
                None,
                gpui::Modifiers::default(),
            );
            test_support::redraw(cx);
            let pinned = clicked_selector != tree_selector;
            for copy_selector in [pin_selector, group_member_selector, tree_selector] {
                assert_eq!(
                    paint(cx, copy_selector)
                        .iter()
                        .any(|(fill, _)| *fill == selected_bg),
                    copy_selector == clicked_selector,
                    "only {clicked_selector} should be selected; checking {copy_selector}",
                );
            }
            assert_eq!(
                cx.debug_bounds("sidebar_sticky_selected_branch").is_some(),
                clicked_selector != pin_selector
            );
            if clicked_selector != pin_selector {
                assert_eq!(
                    cx.debug_bounds("sidebar_sticky_selected_branch"),
                    cx.debug_bounds(clicked_selector),
                    "the sticky selection must follow the clicked copy",
                );
            }
            if pinned {
                assert_eq!(paint(cx, tree_selector), tree_resting);
            }
            cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    assert_eq!(pane.selected_branch().unwrap().target, target);
                    assert!(Rc::ptr_eq(
                        &rows,
                        &pane.branch_sidebar_presentation_cached().unwrap().rows
                    ));
                    assert_eq!(
                        pane.sticky_context
                            .as_ref()
                            .unwrap()
                            .eligible_rows
                            .contains(&tree_ix),
                        !pinned
                    );
                    assert_eq!(
                        pane.sticky_context
                            .as_ref()
                            .unwrap()
                            .eligible_rows
                            .contains(&group_member_ix),
                        clicked_selector == group_member_selector,
                    );
                });
                assert_eq!(
                    view.read(app)
                        .main_pane
                        .read(app)
                        .history_view
                        .read(app)
                        .selected_branch_for_history_row(state.repos[0].id, true)
                        .unwrap()
                        .target,
                    target,
                    "pin clicks still identify the selected branch in history",
                );
            });
        }
    }
}

#[test]
fn sticky_sidebar_paths_include_both_targets_and_ignore_closed_or_filtered_paths() {
    let state = fixture(4);
    let repo = &state.repos[0];
    let selected = BranchMenuTarget::remote("origin", "shared/topic-000001");
    let rows = branch_sidebar::expanded_sidebar_rows(repo, &BTreeSet::new(), "");
    let structure = crate::view::sidebar_sticky::SidebarStructure::new(&rows);
    assert_eq!(
        structure
            .active_path(
                &rows,
                &BranchMenuTarget::local("shared/topic-000000"),
                &crate::view::sidebar_search::SidebarSearch::new("", Default::default())
            )
            .len(),
        1
    );
    assert_eq!(
        structure
            .active_path(
                &rows,
                &selected,
                &crate::view::sidebar_search::SidebarSearch::new("", Default::default())
            )
            .len(),
        2
    );
    assert!(
        structure
            .active_path(
                &rows,
                &selected,
                &crate::view::sidebar_search::SidebarSearch::new("no-match", Default::default())
            )
            .is_empty()
    );
    let collapsed = BTreeSet::from([branch_sidebar::remote_group_storage_key("origin", "shared")]);
    let rows = branch_sidebar::expanded_sidebar_rows(repo, &collapsed, "");
    let structure = crate::view::sidebar_sticky::SidebarStructure::new(&rows);
    assert!(
        structure
            .active_path(
                &rows,
                &selected,
                &crate::view::sidebar_search::SidebarSearch::new("", Default::default())
            )
            .is_empty()
    );
}

#[gpui::test]
fn sticky_sidebar_double_click_keeps_the_scrolled_branch_under_the_pointer(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::{Commit, LogPage};

    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(1_000);
    let repo = &mut Arc::make_mut(&mut state).repos[0];
    repo.history_state.selected_commit = Some(CommitId("a".into()));
    repo.log = Loadable::Ready(
        Arc::new(LogPage {
            commits: vec![Commit {
                id: CommitId("a".into()),
                parent_ids: Default::default(),
                summary: "Branch tip".into(),
                author: "Test".into(),
                time: std::time::SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        })
        .into(),
    );
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        });
    });
    test_support::redraw(cx);
    let target = BranchMenuTarget::remote("origin", "shared/topic-000500");
    let ix = cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            let ix = p.rows.iter().position(|row| {
                matches!(row, BranchSidebarRow::Branch { target: candidate, .. } if candidate == &target)
            }).unwrap();
            let before = pane.sticky_context.as_ref().unwrap().eligible_rows.partition_point(|row| *row < ix);
            let handle = pane.branches_scroll.0.borrow();
            let height = handle.last_item_size.unwrap().contents.height / p.rows.len() as f32;
            handle.base_handle.set_offset(point(px(0.0), -height * (ix - before)));
            cx.notify();
            ix
        })
    });
    test_support::redraw(cx);
    let branch_selector: &'static str = format!("branch_row_81_{ix}").leak();
    let before = cx.debug_bounds(branch_selector).unwrap();
    let position = before.center();
    cx.simulate_click(position, gpui::Modifiers::default());
    test_support::redraw(cx);
    assert_eq!(
        cx.debug_bounds(branch_selector).unwrap(),
        before,
        "the first click must not move the branch out from under the second click"
    );
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    // A frame between the press and release must preserve click ownership too.
    test_support::redraw(cx);
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    test_support::redraw(cx);
    assert!(
        cx.update(|_, app| matches!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::CheckoutRemoteBranchPrompt { remote, branch, .. })
                if remote == "origin" && branch == "shared/topic-000500"
        )),
        "the same-position double click must open the clicked branch's checkout prompt"
    );
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        })
    });
    cx.simulate_mouse_move(
        point(px(900.0), px(500.0)),
        None,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);
    let sticky = cx.debug_bounds("sidebar_sticky_selected_branch").unwrap();
    assert!(
        sticky.top() > before.top(),
        "leaving the clicked row must apply the deferred sticky ancestors"
    );
    let viewport = cx.update(|_, app| {
        pane.read(app)
            .branches_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
    });
    scroll(cx, viewport.center(), point(px(0.0), px(-400.0)));
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("sidebar_sticky_selected_branch").is_some(),
        "normal selected-branch stickiness must resume after the gesture"
    );
}

#[gpui::test]
fn sticky_selected_branches_follow_both_edges_and_survive_compaction(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(1_000);
    Arc::make_mut(&mut state).repos[0]
        .history_state
        .selected_commit = Some(CommitId("a".into()));
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    for target in [
        BranchMenuTarget::local("shared/topic-000500"),
        BranchMenuTarget::remote("origin", "shared/topic-000500"),
    ] {
        cx.simulate_resize(gpui::size(px(1000.0), px(800.0)));
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.branch_filter_query.clear();
                pane.set_selected_branch(state.repos[0].id, target.clone(), None, cx);
            })
        });
        test_support::redraw(cx);
        let (rows, eligible, selected_ix, row_height, viewport, max_offset) =
            cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    let p = pane.branch_sidebar_presentation_cached().unwrap();
                    let selected_ix = p.rows.iter().position(|row| matches!(row,
                    BranchSidebarRow::Branch { target: candidate, .. } if candidate == &target
                )).unwrap();
                    let handle = pane.branches_scroll.0.borrow();
                    let row_height =
                        handle.last_item_size.unwrap().contents.height / p.rows.len() as f32;
                    (
                        p.rows,
                        pane.sticky_context.as_ref().unwrap().eligible_rows.clone(),
                        selected_ix,
                        row_height,
                        handle.base_handle.bounds(),
                        handle.base_handle.max_offset().y,
                    )
                })
            });
        let rank = eligible.iter().position(|ix| *ix == selected_ix).unwrap();
        for offset in [
            px(0.0),
            row_height * selected_ix - viewport.size.height / 2.0,
            max_offset,
        ] {
            cx.update(|_, app| {
                pane.update(app, |pane, cx| {
                    pane.branches_scroll
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.0), -offset));
                    cx.notify();
                })
            });
            test_support::redraw(cx);
            let branch = cx.debug_bounds("sidebar_sticky_selected_branch").unwrap();
            assert!(branch.top() >= viewport.top());
            assert!(branch.bottom() <= viewport.bottom());
            if offset == px(0.0) {
                assert_eq!(
                    branch.top(),
                    viewport.bottom() - row_height * (eligible.len() - rank)
                );
            } else if offset == max_offset {
                assert_eq!(branch.top(), viewport.top() + row_height * rank);
            } else {
                assert!((branch.top() - viewport.center().y).abs() < px(1.0));
            }
            let mut previous = viewport.top();
            for ix in eligible.iter() {
                let bounds = if *ix == selected_ix {
                    branch
                } else {
                    cx.debug_bounds(selector(*ix)).unwrap()
                };
                assert!(bounds.top() >= previous);
                previous = bounds.bottom();
            }
            cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    assert!(Rc::ptr_eq(
                        &rows,
                        &pane.branch_sidebar_presentation_cached().unwrap().rows
                    ));
                    assert!(Rc::ptr_eq(
                        &eligible,
                        &pane.sticky_context.as_ref().unwrap().eligible_rows
                    ));
                })
            });
        }
        let chrome = cx.update(|window, _| window.viewport_size().height - viewport.size.height);
        for (slots, count, selected_visible) in [
            (7.25, 6, true),
            (6.25, 5, false),
            (5.5, 0, false),
            (12.0, eligible.len(), true),
        ] {
            cx.simulate_resize(gpui::size(px(1000.0), chrome + row_height * slots));
            test_support::redraw(cx);
            cx.update(|_, app| assert_eq!(pane.read(app).decorated_sidebar_rows().len(), count));
            assert_eq!(
                cx.debug_bounds("sidebar_sticky_selected_branch").is_some(),
                selected_visible
            );
            if count == 6 {
                assert!(cx.debug_bounds("sidebar_ancestor_menu_0").is_some());
            }
        }
        // A query with no matches hides the selected leaf and empty sections.
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.branch_filter_query = "no matching refs".into();
                cx.notify();
            })
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_sticky_selected_branch").is_none());
        cx.update(|_, app| assert!(pane.read(app).decorated_sidebar_rows().is_empty()));
    }
}

#[gpui::test]
fn sticky_groups_navigate_at_edges_and_toggle_at_their_natural_position(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(500);
    Arc::make_mut(&mut state).repos[0]
        .history_state
        .selected_commit = Some(CommitId("a".into()));
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    for (target, key) in [
        (
            BranchMenuTarget::local("shared/topic-000100"),
            branch_sidebar::local_group_storage_key("shared"),
        ),
        (
            BranchMenuTarget::remote("origin", "shared/topic-000100"),
            branch_sidebar::remote_header_storage_key("origin"),
        ),
        (
            BranchMenuTarget::remote("origin", "shared/topic-000100"),
            branch_sidebar::remote_group_storage_key("origin", "shared"),
        ),
    ] {
        for hit in ["label", "icon", "chevron"] {
            cx.update(|_, app| {
                pane.update(app, |pane, cx| {
                    pane.set_active_repo_collapse_key(
                        branch_sidebar::remote_header_storage_key("origin").into(),
                        false,
                        cx,
                    );
                    pane.set_active_repo_collapse_key(key.clone().into(), false, cx);
                    pane.set_selected_branch(state.repos[0].id, target.clone(), None, cx);
                    pane.branches_scroll
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.0), px(-1_000.0)));
                })
            });
            test_support::redraw(cx);
            let ix = cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    pane.branch_sidebar_presentation_cached()
                        .unwrap()
                        .structure
                        .headers[key.as_str()]
                })
            });
            let row = cx.debug_bounds(selector(ix)).unwrap();
            let toggle = cx
                .debug_bounds(format!("sidebar_group_toggle_{ix}").leak())
                .unwrap();
            let position = match hit {
                "chevron" => toggle.center(),
                "icon" => point(toggle.right() + px(14.0), row.center().y),
                _ => point(row.right() - px(50.0), row.center().y),
            };
            cx.simulate_click(position, gpui::Modifiers::default());
            test_support::redraw(cx);
            // Edge-held group rows navigate, including their chevron. Once
            // back at their natural position the same targets toggle.
            let navigated_offset = cx.update(|_, app| {
                let scale = ui_scale::current(app).percent;
                let pane = pane.read(app);
                assert!(!branch_sidebar::is_collapsed(
                    &pane.collapsed_items_for_test(),
                    &key
                ));
                let handle = pane.branches_scroll.0.borrow();
                let offset = handle.base_handle.offset().y;
                assert_ne!(offset, px(-1_000.0));
                let row_height = sidebar_list_row_height(pane.theme, scale);
                let natural_top = handle.base_handle.bounds().top() + offset + row_height * ix;
                (offset, natural_top)
            });
            let row = cx.debug_bounds(selector(ix)).unwrap();
            assert!((row.top() - navigated_offset.1).abs() < px(0.5), "{key}");
            let toggle = cx
                .debug_bounds(format!("sidebar_group_toggle_{ix}").leak())
                .unwrap();
            let position = match hit {
                "chevron" => toggle.center(),
                "icon" => point(toggle.right() + px(14.0), row.center().y),
                _ => point(row.right() - px(50.0), row.center().y),
            };
            cx.simulate_click(position, gpui::Modifiers::default());
            test_support::redraw(cx);
            assert!(cx.debug_bounds("sidebar_sticky_selected_branch").is_none());
            cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    let p = pane.branch_sidebar_presentation_cached().unwrap();
                    let closed_ix = p.structure.headers[key.as_str()];
                    assert!(branch_sidebar::is_collapsed(
                        &pane.collapsed_items_for_test(),
                        &key
                    ));
                    assert!(
                        !pane
                            .sticky_context
                            .as_ref()
                            .unwrap()
                            .eligible_rows
                            .contains(&closed_ix)
                    );
                    let handle = pane.branches_scroll.0.borrow();
                    assert_eq!(
                        handle.base_handle.offset().y,
                        navigated_offset.0.max(-handle.base_handle.max_offset().y),
                        "toggling preserves the offset except for clamping a shortened list"
                    );
                    assert!(pane.pending_sidebar_navigation.is_none());
                })
            });
        }
        cx.update(|_, app| {
            pane.update(app, |pane, cx| {
                pane.set_active_repo_collapse_key(key.clone().into(), false, cx);
            })
        });
    }
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.set_selected_branch(
                state.repos[0].id,
                BranchMenuTarget::local("shared/topic-000100"),
                None,
                cx,
            );
            pane.branches_scroll
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.0), px(-1_000.0)));
        })
    });
    test_support::redraw(cx);
    let compact_height = cx.update(|window, app| {
        let pane = pane.read(app);
        let viewport_height = pane
            .branches_scroll
            .0
            .borrow()
            .last_item_size
            .unwrap()
            .item
            .height;
        window.viewport_size().height - viewport_height
            + sidebar_list_row_height(pane.theme, ui_scale::current(app).percent) * 7.25
    });
    cx.simulate_resize(gpui::size(px(1000.0), compact_height));
    test_support::redraw(cx);
    let menu = cx.debug_bounds("sidebar_ancestor_menu_0").unwrap();
    cx.simulate_click(menu.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    let entry = cx.debug_bounds("context_menu_collapse_shared").unwrap();
    cx.simulate_click(entry.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    assert!(cx.debug_bounds("sidebar_sticky_selected_branch").is_none());
    cx.update(|_, app| {
        let pane = pane.read(app);
        assert!(branch_sidebar::is_collapsed(
            &pane.collapsed_items_for_test(),
            &branch_sidebar::local_group_storage_key("shared")
        ));
        assert_eq!(
            pane.branches_scroll.0.borrow().base_handle.offset().y,
            px(-1_000.0)
        );
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(800.0)));
    // A non-active group remains in the same tree position as it toggles,
    // allowing its row's keyboard activation to be checked after a pointer click.
    insert_branch(&mut state, "aaa/topic");
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.sidebar_pane.update(cx, |pane, cx| {
                pane.sticky_context = None;
                pane.branches_scroll
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(point(px(0.0), px(0.0)));
                cx.notify();
            });
        })
    });
    test_support::redraw(cx);
    let key = branch_sidebar::local_group_storage_key("aaa");
    let ix = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            pane.branch_sidebar_presentation_cached()
                .unwrap()
                .structure
                .headers[key.as_str()]
        })
    });
    let bounds = cx
        .debug_bounds(format!("branch_group_{ix}").leak())
        .unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    cx.update(|_, app| {
        assert!(branch_sidebar::is_collapsed(
            &pane.read(app).collapsed_items_for_test(),
            &key
        ))
    });
    for keypress in ["enter", "space"] {
        cx.simulate_keystrokes(keypress);
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(keypress).unwrap(),
        });
        test_support::redraw(cx);
        cx.update(|_, app| {
            assert_eq!(
                branch_sidebar::is_collapsed(&pane.read(app).collapsed_items_for_test(), &key),
                keypress == "space"
            );
            assert_eq!(
                pane.read(app)
                    .branches_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .y,
                px(0.0)
            );
        });
    }
}

#[gpui::test]
fn sticky_sidebar_selection_updates_without_rebuilding_rows(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(100);
    Arc::make_mut(&mut state).repos[0]
        .history_state
        .selected_commit = Some(CommitId("a".into()));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    let rows = cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            assert_eq!(pane.sticky_context.as_ref().unwrap().eligible_rows.len(), 6);
            pane.set_selected_branch(
                state.repos[0].id,
                BranchMenuTarget::remote("origin", "shared/topic-000001"),
                None,
                cx,
            );
            p.rows
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert_eq!(pane.sticky_context.as_ref().unwrap().eligible_rows.len(), 9);
            pane.set_selected_branch(
                state.repos[0].id,
                BranchMenuTarget::local("shared/topic-000001"),
                None,
                cx,
            );
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            assert_eq!(
                pane.sticky_context.as_ref().unwrap().eligible_rows.len(),
                7,
                "shared paths are deduplicated"
            );
            pane.set_selected_branch(
                state.repos[0].id,
                BranchMenuTarget::remote("origin", "shared/topic-000001"),
                None,
                cx,
            );
        })
    });
    test_support::redraw(cx);
    Arc::make_mut(&mut state).repos[0]
        .history_state
        .selected_commit = Some(CommitId("b".into()));
    // Only a history selection changed: the store observer must repaint the
    // active paths without changing the branch presentation.
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            assert!(Rc::ptr_eq(&p.rows, &rows));
            assert_eq!(pane.sticky_context.as_ref().unwrap().eligible_rows.len(), 6);
        })
    });
    for head in ["HEAD", "shared/deleted-branch"] {
        let repo = &mut Arc::make_mut(&mut state).repos[0];
        repo.head_branch = Loadable::Ready(head.into());
        repo.head_branch_rev += 1;
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state.clone(), cx);
            })
        });
        test_support::redraw(cx);
        cx.update(|_, app| {
            assert_eq!(
                pane.read(app)
                    .sticky_context
                    .as_ref()
                    .unwrap()
                    .eligible_rows
                    .len(),
                5
            )
        });
    }
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.navigate_sidebar_header(branch_sidebar::stash_section_storage_key().into(), cx);
        })
    });
    let next = Arc::make_mut(&mut state);
    next.repos[0].id = RepoId(82);
    next.repos[0].spec.workdir = "/tmp/gitcomet-next-sidebar".into();
    next.active_repo = Some(RepoId(82));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        let pane = pane.read(app);
        assert!(pane.pending_sidebar_navigation.is_none());
        assert_eq!(
            pane.branches_scroll.0.borrow().base_handle.offset().y,
            px(0.0)
        );
    });
}

#[gpui::test]
fn sticky_sidebar_short_viewports_and_resizing_keep_content_accessible(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(1_000);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    insert_branch(&mut state, "aaa/first-branch");
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        assert_eq!(
            pane.read(app)
                .branches_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y,
            px(0.0),
            "new content at the start must remain visible"
        )
    });
    let chrome = cx.update(|window, app| {
        let pane = pane.read(app);
        let size = pane.branches_scroll.0.borrow().last_item_size.unwrap();
        window.viewport_size().height - size.item.height
    });
    let h = cx.update(|_, app| {
        sidebar_list_row_height(pane.read(app).theme, ui_scale::current(app).percent)
    });
    for (slots, expected) in [(6.25, 5), (5.5, 0), (12.0, 6)] {
        cx.simulate_resize(gpui::size(px(1000.0), chrome + h * slots));
        test_support::redraw(cx);
        cx.update(|_, app| assert_eq!(pane.read(app).decorated_sidebar_rows().len(), expected));
        if expected == 5 {
            assert!(cx.debug_bounds("sidebar_ancestor_menu_0").is_some());
        }
        assert_eq!(cx.debug_bounds(selector(0)).is_some(), expected != 0);
    }
    cx.simulate_resize(gpui::size(px(1000.0), px(800.0)));
    test_support::redraw(cx);
    let local = cx.debug_bounds(selector(0)).unwrap();
    scroll(cx, local.center(), point(px(0.0), px(-1_200.0)));
    test_support::redraw(cx);
    let (anchor, before) = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            let h = f32::from(
                pane.branches_scroll
                    .0
                    .borrow()
                    .last_item_size
                    .unwrap()
                    .contents
                    .height,
            ) / p.rows.len() as f32;
            let scroll = -f32::from(pane.branches_scroll.0.borrow().base_handle.offset().y);
            let ix = (scroll / h).ceil() as usize + 2;
            (p.rows[ix].clone(), ix as f32 - scroll / h)
        })
    });
    // A new group ahead of the viewport must not shift the content being read.
    insert_branch(&mut state, "aaaa/new-branch");
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
        })
    });
    test_support::redraw(cx);
    for (density, scale) in [
        (crate::appearance::UiDensity::Compact, 100),
        (crate::appearance::UiDensity::Spacious, 150),
    ] {
        cx.update(|_, app| {
            app.set_global(crate::appearance::Appearance {
                density,
                ..Default::default()
            });
            ui_scale::set_current(app, scale);
            view.update(app, |view, cx| view.notify_font_preferences_changed(cx));
        });
        test_support::redraw(cx);
        cx.update(|_, app| {
            pane.update(app, |pane, _| {
                let p = pane.branch_sidebar_presentation_cached().unwrap();
                let ix = p
                    .rows
                    .iter()
                    .position(|row| crate::view::sidebar_sticky::same_row(row, &anchor))
                    .unwrap();
                let h = f32::from(
                    pane.branches_scroll
                        .0
                        .borrow()
                        .last_item_size
                        .unwrap()
                        .contents
                        .height,
                ) / p.rows.len() as f32;
                let scroll = -f32::from(pane.branches_scroll.0.borrow().base_handle.offset().y);
                assert!((ix as f32 - scroll / h - before).abs() < 0.01);
            })
        });
    }
}

#[test]
fn sticky_sidebar_ignores_legacy_top_level_collapse_and_requests_visible_data() {
    let state = fixture(4);
    let repo = &state.repos[0];
    let collapsed = BTreeSet::from([
        branch_sidebar::local_section_storage_key().to_string(),
        branch_sidebar::remote_section_storage_key().to_string(),
    ]);
    let rows = branch_sidebar::expanded_sidebar_rows(repo, &collapsed, "");
    assert_eq!(
        rows.iter()
            .filter(|row| matches!(row, BranchSidebarRow::Branch { .. }))
            .count(),
        8
    );
    let (_, request) =
        sidebar_presentation::active_sidebar_data_request(&state, &BTreeMap::new(), true).unwrap();
    assert!(request.worktrees && request.submodules && request.stashes);
}

#[test]
fn sticky_sidebar_stash_identity_survives_reindexing_and_invalidates_cached_rows() {
    let mut state = long_list_tests::fixture(2, CollapsedSidebarSection::Stashes);
    let mut cache = SidebarPresentationCache::default();
    let empty = BTreeMap::new();
    let before =
        sidebar_presentation::build_sidebar_presentation(&mut cache, &state, &empty, &empty, "")
            .unwrap();
    let old = before
        .rows
        .iter()
        .find(|row| matches!(row, BranchSidebarRow::StashItem { .. }))
        .unwrap();
    let mut reindexed = old.clone();
    if let BranchSidebarRow::StashItem { index, .. } = &mut reindexed {
        *index += 1;
    }
    assert!(crate::view::sidebar_sticky::same_row(old, &reindexed));
    let repo = &mut Arc::make_mut(&mut state).repos[0];
    if let Loadable::Ready(stashes) = &mut repo.stashes {
        let mut updated = stashes.as_ref().clone();
        updated[0].id = CommitId("replacement".into());
        *stashes = Arc::new(updated);
    }
    repo.stashes_rev += 1;
    repo.branch_sidebar_rev += 1;
    let after =
        sidebar_presentation::build_sidebar_presentation(&mut cache, &state, &empty, &empty, "")
            .unwrap();
    assert!(!Rc::ptr_eq(&before.rows, &after.rows));
    let new = after
        .rows
        .iter()
        .find(|row| matches!(row, BranchSidebarRow::StashItem { .. }))
        .unwrap();
    assert!(!crate::view::sidebar_sticky::same_row(old, new));
}

#[gpui::test]
fn sticky_sidebar_surfaces_and_pin_alignment_follow_theme_density_and_scale(
    cx: &mut gpui::TestAppContext,
) {
    use crate::test_support::painted_control_quads as paint;
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(100);
    Arc::make_mut(&mut state).repos[0]
        .history_state
        .selected_commit = Some(CommitId("a".into()));
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
            view.sidebar_pane.update(cx, |pane, cx| {
                pane.sidebar_pinned_branches_by_repo.insert(
                    state.repos[0].spec.workdir.clone(),
                    BTreeSet::from([
                        "local:shared/topic-000001".into(),
                        "remote:origin/shared/topic-000050".into(),
                    ]),
                );
                pane.sidebar_presentation_cache = SidebarPresentationCache::default();
                pane.set_selected_branch(
                    state.repos[0].id,
                    BranchMenuTarget::remote("origin", "shared/topic-000050"),
                    None,
                    cx,
                );
            });
        })
    });
    cx.simulate_resize(gpui::size(px(1100.0), px(1100.0)));
    for key in [
        "gitcomet_light",
        "gitcomet_dark",
        "sunset_veil",
        "tokyo_night",
        "amber_dark",
    ] {
        for (density, scale) in [
            (crate::appearance::UiDensity::Compact, 100),
            (crate::appearance::UiDensity::Spacious, 150),
        ] {
            cx.update(|_, app| {
                app.set_global(crate::appearance::Appearance {
                    density,
                    ..Default::default()
                });
                ui_scale::set_current(app, scale);
                view.update(app, |view, cx| {
                    view.notify_font_preferences_changed(cx);
                    view.set_theme(AppTheme::from_key(key).unwrap(), cx);
                });
            });
            test_support::redraw(cx);
            let theme = cx.update(|_, app| pane.read(app).theme);
            assert!(
                paint(cx, "pinned_branch_row_81_0")
                    .iter()
                    .any(|(fill, _)| *fill == theme.colors.surface.panel.into())
            );
            let mut saw_top_stuck = false;
            let mut saw_bottom_stuck = false;
            for offset in [px(0.0), sidebar_list_row_height(theme, scale) * 6] {
                cx.update(|_, app| {
                    pane.read(app)
                        .branches_scroll
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.0), -offset));
                });
                test_support::redraw(cx);
                let headers = cx.update(|_, app| {
                    pane.update(app, |pane, _| {
                        let p = pane.branch_sidebar_presentation_cached().unwrap();
                        pane.sticky_context
                            .as_ref()
                            .unwrap()
                            .eligible_rows
                            .iter()
                            .copied()
                            .filter_map(|ix| {
                                crate::view::sidebar_sticky::header_key(&p.rows[ix])?;
                                let group = matches!(
                                    &p.rows[ix],
                                    BranchSidebarRow::GroupHeader { .. }
                                        | BranchSidebarRow::RemoteHeader { .. }
                                );
                                let handle = pane.branches_scroll.0.borrow();
                                let row_height = handle.last_item_size.unwrap().contents.height
                                    / p.rows.len() as f32;
                                let natural_top = handle.base_handle.bounds().top()
                                    + handle.base_handle.offset().y
                                    + row_height * ix;
                                Some((ix, group, natural_top))
                            })
                            .collect::<Vec<_>>()
                    })
                });
                for (ix, group, natural_top) in headers {
                    let header = cx.debug_bounds(selector(ix)).unwrap();
                    let stuck = (header.top() - natural_top).abs() > px(0.5);
                    let background = if group && !stuck {
                        theme.colors.surface.chrome
                    } else {
                        theme.colors.surface.panel
                    };
                    assert!(
                        paint(cx, selector(ix))
                            .iter()
                            .any(|(fill, _)| *fill == background.into()),
                        "groups should use the header background only when held at a sticky edge",
                    );
                    saw_top_stuck |= stuck && header.top() > natural_top;
                    saw_bottom_stuck |= stuck && header.top() < natural_top;
                    let divider_selector = format!("sidebar_sticky_divider_{ix}").leak();
                    let divider = cx.debug_bounds(divider_selector);
                    assert_eq!(divider.is_some(), stuck && !theme.is_dark);
                    if let Some(divider) = divider {
                        assert_eq!(divider.size.height, px(1.0));
                        assert_eq!(divider.left(), header.left());
                        assert_eq!(divider.right(), header.right());
                        if header.top() > natural_top {
                            assert_eq!(divider.bottom(), header.bottom());
                        } else {
                            assert_eq!(divider.top(), header.top());
                        }
                        assert!(
                            paint(cx, divider_selector)
                                .iter()
                                .any(|(fill, _)| *fill == theme.colors.stroke.subtle.into())
                        );
                    }
                }
            }
            assert!(saw_top_stuck && saw_bottom_stuck);
            assert!(
                paint(cx, "sidebar_sticky_selected_branch")
                    .iter()
                    .any(|(fill, _)| *fill == crate::view::selected_branch_row_bg(theme).into())
            );
            assert!(cx.debug_bounds("sidebar_pinned_area").is_none());
            let local = cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    pane.branch_sidebar_presentation_cached()
                        .unwrap()
                        .structure
                        .sections[0]
                })
            });
            let header = cx.debug_bounds(selector(local)).unwrap();
            let pinned_row = cx.debug_bounds("pinned_branch_row_81_0").unwrap();
            let tree_row = cx.debug_bounds("sidebar_sticky_selected_branch").unwrap();
            let panel = cx.debug_bounds("branch_sidebar_scroll_container").unwrap();
            for row in [header, pinned_row, tree_row] {
                assert_eq!(row.left(), panel.left());
                assert_eq!(row.right(), panel.right());
            }
            let pin = cx.debug_bounds("sidebar_pin_marker_0").unwrap();
            let header_toggle = cx
                .debug_bounds(format!("sidebar_header_toggle_{local}").leak())
                .unwrap();
            let pin_icon = cx.debug_bounds("sidebar_branch_icon_Pins_0").unwrap();
            let header_icon = cx
                .debug_bounds(format!("sidebar_header_icon_{local}").leak())
                .unwrap();
            assert_eq!(pin.left(), header_toggle.left());
            assert_eq!(pin_icon.left(), header_icon.left());
            assert_eq!(pin_icon.size.width, header_icon.size.width);
            assert!(cx.debug_bounds("sidebar_pinned_heading").is_none());
        }
    }
}

#[gpui::test]
fn sticky_sidebar_pinned_groups_expand_cache_and_unpin_as_single_roots(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = fixture(2_000);
    insert_branch(&mut state, "shared/nested/leaf");
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let group_ix = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            pane.branch_sidebar_presentation_cached()
                .unwrap()
                .structure
                .headers["group:local:shared"]
        })
    });
    let group = cx.debug_bounds(selector(group_ix)).unwrap();
    cx.simulate_mouse_down(
        group.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        group.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);
    let pin = cx.debug_bounds("context_menu_pin_group").unwrap();
    cx.simulate_click(pin.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    assert!(cx.debug_bounds("sidebar_group_pin_marker_0").is_some());
    assert!(
        cx.debug_bounds("pinned_branch_group_1").is_some(),
        "nested folder is rendered"
    );
    assert!(cx.debug_bounds("pinned_branch_row_81_2").is_some());
    assert!(
        cx.debug_bounds("sidebar_pin_marker_2").is_none(),
        "inherited members are not individual pins"
    );
    let _pins = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            assert!(
                pane.rendered_rows < 200,
                "large pinned group must stay virtualized"
            );
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            assert_eq!(p.pins.len(), 2_003);
            p.pins
        })
    });
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.branch_filter_query = "nothing matches this".into();
            pane.sync_popover_branch_filter(cx);
            cx.notify();
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            let p = pane.branch_sidebar_presentation_cached().unwrap();
            assert!(p.pins.is_empty());
            assert!(
                !p.rows
                    .iter()
                    .any(|row| matches!(row, BranchSidebarRow::Branch { .. }))
            );
        })
    });
    cx.update(|_, app| pane.update(app, |pane, cx| pane.clear_branch_filter(cx)));
    test_support::redraw(cx);
    // Nested folders and pinned roots retain their canonical collapse keys.
    let nested = cx.debug_bounds("pinned_sidebar_group_toggle_1").unwrap();
    cx.simulate_click(nested.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    cx.update(|_, app| pane.update(app, |pane, _| {
        let p = pane.branch_sidebar_presentation_cached().unwrap();
        assert_eq!(p.pins.len(), 2_002);
        assert!(!p.pins.iter().any(|row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "shared/nested/leaf")));
    }));
    let root = cx.debug_bounds("pinned_branch_group_0").unwrap();
    cx.simulate_click(root.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            assert_eq!(
                pane.branch_sidebar_presentation_cached()
                    .unwrap()
                    .pins
                    .len(),
                1
            );
            assert_eq!(
                pane.branches_scroll.0.borrow().base_handle.offset().y,
                px(0.0)
            );
            assert_eq!(
                pane.saved_sidebar_pinned_branches()[&state.repos[0].spec.workdir],
                BTreeSet::from(["group:local:shared".into()])
            );
        })
    });
    let root = cx.debug_bounds("pinned_branch_group_0").unwrap();
    cx.simulate_mouse_down(
        root.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        root.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);
    assert!(cx.debug_bounds("context_menu_unpin_group").is_some());
    assert!(cx.update(|_, app| matches!(
        test_support::popover_kind(view.read(app), app),
        Some(PopoverKind::BranchGroupMenu {
            section: BranchSection::Local,
            ..
        })
    )));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        })
    });
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.toggle_sidebar_pin(state.repos[0].id, "group:remote:origin:shared".into(), cx);
        })
    });
    test_support::redraw(cx);
    // Bulk unpin counts roots, including closed and filtered-out groups, and
    // leaves the other section's pin intact.
    let local_ix = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            pane.branch_sidebar_presentation_cached()
                .unwrap()
                .structure
                .sections[0]
        })
    });
    let local_header = cx.debug_bounds(selector(local_ix)).unwrap();
    cx.simulate_mouse_down(
        local_header.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        local_header.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);
    let unpin = cx.debug_bounds("context_menu_unpin_all_local_1").unwrap();
    cx.simulate_click(unpin.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    cx.update(|_, app| {
        pane.update(app, |pane, _| {
            assert_eq!(
                pane.pinned_branches_for_test(),
                BTreeSet::from(["group:remote:origin:shared".into()])
            );
            assert_eq!(
                pane.branch_sidebar_presentation_cached()
                    .unwrap()
                    .pins
                    .len(),
                2_001
            );
        })
    });
    let root = cx.debug_bounds("pinned_branch_group_0").unwrap();
    cx.simulate_mouse_down(
        root.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        root.center(),
        MouseButton::Right,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);
    let unpin = cx.debug_bounds("context_menu_unpin_group").unwrap();
    cx.simulate_click(unpin.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    assert!(cx.debug_bounds("sidebar_pinned_area").is_none());
    cx.update(|_, app| assert!(pane.read(app).saved_sidebar_pinned_branches().is_empty()));
}
