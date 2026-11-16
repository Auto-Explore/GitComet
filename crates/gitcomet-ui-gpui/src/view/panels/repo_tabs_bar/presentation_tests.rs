use super::*;
use crate::view::test_support;
use gitcomet_core::domain::{
    Commit, CommitId, LogPage, RepoSpec, RepositoryMode, Upstream, UpstreamDivergence,
};
use gitcomet_state::model::RepoIncomingSnapshot;
use gpui::{ScrollDelta, ScrollWheelEvent, TouchPhase};
use gpui::{TestAppContext, VisualTestContext};
use std::time::UNIX_EPOCH;

const TAB_COUNT: usize = 264;

fn mount(cx: &mut TestAppContext) -> (Entity<GitCometView>, &mut VisualTestContext, AppState) {
    let (store, events) = AppStore::new_test(Arc::new(test_support::TestBackend));
    store.disable_repo_monitors_for_test();
    let fixture_store = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1400.0), px(900.0)));
    let mut state = AppState {
        repos: (0..TAB_COUNT)
            .map(|ix| {
                let mut repo = RepoState::new_opening(
                    RepoId(ix as u64 + 1),
                    RepoSpec {
                        workdir: PathBuf::from(format!("/tmp/tab-cache/repository-{ix:03}")),
                    },
                );
                repo.open = Loadable::Ready(());
                repo.repository_mode = Some(RepositoryMode::WorkingTree);
                repo.head_branch = Loadable::Ready("main".into());
                repo.status = Loadable::Ready(Default::default());
                repo
            })
            .collect(),
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    let page = Arc::new(LogPage {
        commits: (0..100)
            .map(|ix| Commit {
                id: CommitId(format!("commit-{ix}").into()),
                parent_ids: Default::default(),
                summary: format!("Commit {ix}").into(),
                author: "Author".into(),
                time: UNIX_EPOCH,
            })
            .collect(),
        next_cursor: None,
    });
    state.repos[0].log = Loadable::Ready(page.clone());
    state.repos[0].history_state.log = Loadable::Ready(page);
    state.repos[0].log_rev = 1;
    fixture_store.replace_snapshot_for_test(Arc::new(state.clone()));
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(view, Arc::new(state.clone()), cx);
        });
        window.set_a11y_forced(true);
    });
    cx.run_until_parked();
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    cx.update(|_, app| {
        history.update(app, |history, cx| {
            history.history_relative_dates = false;
            history.ensure_history_cache(cx);
        })
    });
    for _ in 0..4 {
        cx.run_until_parked();
        test_support::redraw(cx);
    }
    cx.update(|_, app| {
        assert!(history.read(app).history_cache.is_some());
        test_support::dismiss_error_toasts(&view, app);
    });
    test_support::redraw(cx);
    (view, cx, state)
}

fn bar(view: &Entity<GitCometView>, cx: &mut VisualTestContext) -> Entity<RepoTabsBarView> {
    cx.update(|_, app| view.read(app).repo_tabs_bar.clone())
}

fn counters(bar: &Entity<RepoTabsBarView>, cx: &mut VisualTestContext) -> (usize, usize, usize) {
    cx.update(|_, app| {
        let cache = &bar.read(app).presentation;
        (cache.rebuilds, cache.measurements, cache.renders)
    })
}

fn wheel(cx: &mut VisualTestContext, position: gpui::Point<Pixels>, y: f32, phase: TouchPhase) {
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.0), px(y))),
                touch_phase: phase,
                modifiers: Default::default(),
            }),
            app,
        );
        let _ = window.draw(app);
    });
}

#[gpui::test]
fn forced_accessibility_history_frames_reuse_264_tab_presentations(cx: &mut TestAppContext) {
    let (view, cx, _) = mount(cx);
    let bar = bar(&view, cx);
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        // Install live handlers before the first packet, and retain that policy
        // through programmatic repositioning and all scheduled release frames.
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        let before = counters(&bar, cx);
        let close_before = cx.update(|_, app| bar.read(app).presentation.close_builds);
        let visuals_before = cx.update(|_, app| bar.read(app).presentation.visual_builds.get());
        assert_eq!(before.0, TAB_COUNT);
        assert_eq!(before.1, TAB_COUNT);
        let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
        let scroll = cx.update(|_, app| history.read(app).history_scroll.clone());
        let position = scroll.0.borrow().base_handle.bounds().center();
        for ix in 0..8 {
            wheel(
                cx,
                position,
                -20.0,
                if ix == 0 {
                    TouchPhase::Started
                } else {
                    TouchPhase::Moved
                },
            );
        }
        assert!(scroll.0.borrow().base_handle.offset().y < px(-100.0));
        scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.0), px(0.0)));
        cx.update(|_, app| history.update(app, |_, cx| cx.notify()));
        test_support::redraw(cx);
        for _ in 0..4 {
            test_support::redraw(cx);
        }
        let after = counters(&bar, cx);
        assert_eq!(
            cx.update(|_, app| bar.read(app).presentation.close_builds),
            close_before,
            "history redraws must not construct unused close controls for 264 idle tabs"
        );
        assert_eq!((after.0, after.1), (before.0, before.1));
        assert!(
            after.2 >= before.2 + 10,
            "AX redraws must actually render the tab strip"
        );
        let visuals_after = cx.update(|_, app| bar.read(app).presentation.visual_builds.get());
        assert!(visuals_after > visuals_before);
        assert!(
            visuals_after - visuals_before < (after.2 - before.2) * TAB_COUNT / 4,
            "history frames must build only visible tab visuals"
        );
        cx.update(|window, app| {
            assert!(window.is_a11y_active());
            assert!(window.a11y_tree().is_some());
            let bar = bar.read(app);
            assert_eq!(bar.presentation.tabs.len(), TAB_COUNT);
            assert!((0..TAB_COUNT).all(|ix| bar.tab_scroll.tab_bounds(ix).is_some()));
        });
    });

    // Reveal and activate a previously offscreen tab through its existing
    // pointer handler; memoization must preserve the full strip and its input.
    cx.update(|_, app| {
        bar.update(app, |bar, cx| {
            bar.tab_scroll.scroll_to_tab(TAB_COUNT - 1);
            cx.notify();
        })
    });
    for _ in 0..2 {
        cx.update(|window, app| {
            window.simulate_next_frame(app);
            let _ = window.draw(app);
        });
    }
    let last = cx.update(|_, app| {
        let bar = bar.read(app);
        let viewport = bar.tab_scroll.viewport();
        let last = bar.tab_scroll.tab_bounds(TAB_COUNT - 1).unwrap();
        assert_eq!(bar.tab_scroll.scrolled(), bar.tab_scroll.max_scroll());
        assert!(viewport.contains(&last.center()));
        // GPUI's existing extent excludes the leading tab margin. Preserve
        // that geometry here while checking the far tab is usable at the end.
        assert!(
            last.right() <= viewport.right() + px(components::Tab::HORIZONTAL_MARGIN_PX) + px(0.5),
            "far tab: scrolled={:?} max={:?} viewport={:?} last={:?} pending={:?}",
            bar.tab_scroll.scrolled(),
            bar.tab_scroll.max_scroll(),
            viewport,
            last,
            bar.pending_reveal,
        );
        last
    });
    assert!(cx.debug_bounds("repo_tab_label_264").is_some());
    let painted = cx.debug_bounds("repo_tab_264").unwrap();
    assert!(
        painted.contains(&last.center()),
        "actual={painted:?}, logical={last:?}"
    );
    cx.simulate_click(last.center(), gpui::Modifiers::default());
    test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).store.snapshot().active_repo,
            Some(RepoId(TAB_COUNT as u64))
        )
    });
}

#[gpui::test]
fn tab_presentations_refresh_changed_fields_fonts_and_closed_tabs(cx: &mut TestAppContext) {
    let (view, cx, mut state) = mount(cx);
    let bar = bar(&view, cx);
    let before = counters(&bar, cx);
    state.repos[1].spec.workdir = "/tmp/tab-cache/a-renamed-repository".into();
    state.repos[1].incoming = RepoIncomingState::Current(RepoIncomingSnapshot {
        tracking: HeadTracking::Tracking {
            branch: "main".into(),
            upstream: Upstream {
                remote: "origin".into(),
                branch: "main".into(),
            },
            divergence: Some(UpstreamDivergence {
                ahead: 0,
                behind: 12,
            }),
        },
        checked_at: UNIX_EPOCH,
    });
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(view, Arc::new(state.clone()), cx);
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    let changed = counters(&bar, cx);
    assert_eq!(changed.0, before.0 + 1);
    assert_eq!(changed.1, before.1 + 2);
    cx.update(|_, app| {
        let tab = &bar.read(app).presentation.tabs[1];
        assert_eq!(tab.label.as_ref(), "a-renamed-repository");
        assert_eq!(tab.incoming_label.as_deref(), Some("↓12"));
        assert!(tab.tooltip.contains("12 commits behind origin/main"));
    });
    cx.update(|window, app| {
        // Change the real inherited font, rather than clearing the cache.
        crate::font_preferences::set_current(
            app,
            crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY.into(),
            crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY.into(),
            true,
        );
        window.refresh();
    });
    test_support::redraw(cx);
    let fonts = counters(&bar, cx);
    assert_eq!(fonts.0, changed.0);
    assert_eq!(fonts.1, changed.1 + TAB_COUNT + 1);
    test_support::redraw(cx);
    assert_eq!(counters(&bar, cx).1, fonts.1);

    // A theme changes foreground paint while shaping stays the same. With
    // forced AX every draw renders the bar, including after the first change.
    for theme in [AppTheme::gitcomet_light(), AppTheme::gitcomet_dark()] {
        cx.update(|_, app| view.update(app, |view, cx| view.set_theme(theme, cx)));
        let renders = counters(&bar, cx).2;
        for _ in 0..3 {
            test_support::redraw(cx);
            let after_theme = counters(&bar, cx);
            assert_eq!(
                after_theme.1, fonts.1,
                "theme paint must not invalidate widths"
            );
            assert_eq!(after_theme.0, fonts.0);
        }
        assert!(counters(&bar, cx).2 >= renders + 3);
    }

    state.repos.swap(0, 1);
    state.repos.pop();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(view, Arc::new(state.clone()), cx);
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert_eq!(
        counters(&bar, cx).0,
        fonts.0,
        "reorder reuses entries and close drops them"
    );
    cx.update(|_, app| {
        let cache = &bar.read(app).presentation;
        assert_eq!(cache.tabs.len(), TAB_COUNT - 1);
        assert_eq!(cache.tabs[0].id, RepoId(2));
        assert!(
            !cache
                .tabs
                .iter()
                .any(|tab| tab.id == RepoId(TAB_COUNT as u64))
        );
    });
}

#[gpui::test]
fn lazy_tab_visuals_preserve_all_bounds_and_accessibility_when_resized(cx: &mut TestAppContext) {
    let (view, cx, _) = mount(cx);
    let bar = bar(&view, cx);
    for width in [260.0, 480.0, 980.0, 1600.0] {
        cx.simulate_resize(gpui::size(px(width), px(900.0)));
        test_support::redraw(cx);
        // Compare the complete AX tree and all shell boxes against building
        // every decorative subtree, including the offscreen ones.
        let renders_before = cx.update(|_, app| {
            bar.update(app, |bar, cx| {
                let renders = bar.presentation.renders;
                bar.presentation.force_visuals = true;
                bar.presentation.visual_builds.set(0);
                cx.notify();
                renders
            })
        });
        test_support::redraw(cx);
        let (all_bounds, max, mut full_ax) = cx.update(|window, app| {
            let bar = bar.read(app);
            assert!(window.is_a11y_active());
            assert_eq!(
                bar.presentation.visual_builds.get(),
                TAB_COUNT * (bar.presentation.renders - renders_before)
            );
            (
                (0..TAB_COUNT)
                    .map(|ix| bar.tab_scroll.tab_bounds(ix).unwrap())
                    .collect::<Vec<_>>(),
                bar.tab_scroll.max_scroll(),
                window.a11y_tree().unwrap().clone(),
            )
        });
        let old_label = cx.debug_bounds("repo_tab_label_text_1").unwrap();
        cx.update(|_, app| {
            bar.update(app, |bar, cx| {
                bar.presentation.force_visuals = false;
                bar.presentation.visual_builds.set(0);
                cx.notify();
            });
        });
        test_support::redraw(cx);
        let mut culled_ax = cx.update(|window, app| {
            let bar = bar.read(app);
            let count = bar.presentation.visual_builds.get();
            if bar.tab_scroll.viewport().size.width > px(0.0) {
                assert!(
                    count > 0 && count < TAB_COUNT / 4,
                    "built {count} at width {width}"
                );
            } else {
                assert_eq!(count, 0, "collapsed strip should not build visuals");
            }
            assert_eq!(bar.tab_scroll.max_scroll(), max);
            assert_eq!(
                (0..TAB_COUNT)
                    .map(|ix| bar.tab_scroll.tab_bounds(ix).unwrap())
                    .collect::<Vec<_>>(),
                all_bounds,
            );
            window.a11y_tree().unwrap().clone()
        });
        if let Some(label) = cx.debug_bounds("repo_tab_label_text_1") {
            assert_eq!(label, old_label);
        }
        assert!(
            !full_ax.nodes.is_empty(),
            "AX comparison needs a populated tree"
        );
        full_ax.nodes.sort_by_key(|(id, _)| *id);
        culled_ax.nodes.sort_by_key(|(id, _)| *id);
        assert_eq!(culled_ax, full_ax, "AX changed at width {width}");
    }
}

#[gpui::test]
fn lazy_tab_visuals_follow_horizontal_wheels_and_pending_reveal(cx: &mut TestAppContext) {
    let (view, cx, _) = mount(cx);
    let bar = bar(&view, cx);
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        let viewport = cx.update(|_, app| bar.read(app).tab_scroll.viewport());
        let position = viewport.center();
        let send = |cx: &mut VisualTestContext, x: f32, phase| {
            cx.update(|window, app| {
                window.dispatch_event(
                    gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(px(x), px(0.0))),
                        touch_phase: phase,
                        modifiers: Default::default(),
                    }),
                    app,
                );
                let _ = window.draw(app);
            });
        };
        send(cx, -400.25, TouchPhase::Started);
        assert!(cx.update(|_, app| bar.read(app).tab_scroll.scrolled()) > px(300.0));
        // Check fresh current-frame boxes, rather than a visibility decision
        // based on the previous frame's scroll offset.
        let assert_visible_labels = |cx: &mut VisualTestContext| {
            let bounds = cx.update(|_, app| {
                let bar = bar.read(app);
                let viewport = bar.tab_scroll.viewport();
                (0..TAB_COUNT)
                    .filter_map(|ix| {
                        let bounds = bar.tab_scroll.tab_bounds(ix).unwrap();
                        viewport.contains(&bounds.center()).then_some(ix + 1)
                    })
                    .collect::<Vec<_>>()
            });
            assert!(!bounds.is_empty());
            for id in bounds {
                assert!(
                    cx.debug_bounds(Box::leak(format!("repo_tab_label_{id}").into_boxed_str()))
                        .is_some(),
                    "missing tab {id}"
                );
            }
        };
        assert_visible_labels(cx);
        send(cx, 0.0, TouchPhase::Ended);
        cx.update(|_, app| {
            bar.update(app, |bar, cx| {
                bar.tab_scroll.scroll_to_tab(0);
                cx.notify();
            })
        });
        test_support::redraw(cx);
        assert_visible_labels(cx);
        // Exercise the view's automatic pending reveal, not just the handle.
        cx.update(|_, app| {
            bar.update(app, |bar, cx| {
                bar.pending_reveal = Some((RepoId(TAB_COUNT as u64), 0));
                cx.notify();
            })
        });
        for _ in 0..4 {
            cx.executor().advance_clock(Duration::from_millis(200));
            cx.update(|window, app| {
                window.simulate_next_frame(app);
                let _ = window.draw(app);
            });
        }
        assert_visible_labels(cx);
        assert!(cx.debug_bounds("repo_tab_label_264").is_some());
        cx.update(|_, app| {
            let bar = bar.read(app);
            assert_eq!(bar.tab_scroll.scrolled(), bar.tab_scroll.max_scroll());
            assert!(bar.pending_reveal.is_none());
        });
    });
}

fn tab_frame(cx: &mut VisualTestContext, delay_ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(delay_ms));
    cx.update(|window, app| {
        window.simulate_next_frame(app);
        window.refresh();
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn forced_accessibility_close_controls_are_lazy_and_keep_their_pointer_action(
    cx: &mut TestAppContext,
) {
    let (view, cx, _) = mount(cx);
    let bar = bar(&view, cx);
    let before = cx.update(|_, app| {
        let bar = bar.read(app);
        (
            bar.presentation.close_builds,
            (0..TAB_COUNT)
                .map(|ix| bar.tab_scroll.tab_bounds(ix).unwrap())
                .collect::<Vec<_>>(),
        )
    });
    for _ in 0..3 {
        test_support::redraw(cx);
    }
    assert_eq!(
        cx.update(|_, app| bar.read(app).presentation.close_builds),
        before.0
    );
    assert!(cx.debug_bounds("repo_tab_close_1").is_none());
    cx.simulate_mouse_move(before.1[0].center(), None, gpui::Modifiers::default());
    test_support::redraw(cx);
    let close = cx
        .debug_bounds("repo_tab_close_1")
        .expect("hover must build the real close action");
    cx.update(|window, app| {
        assert!(window.is_a11y_active());
        assert!(!window.a11y_tree().unwrap().nodes.is_empty());
        let bar = bar.read(app);
        assert!(bar.presentation.close_builds > before.0);
        assert_eq!(
            (0..TAB_COUNT)
                .map(|ix| bar.tab_scroll.tab_bounds(ix).unwrap())
                .collect::<Vec<_>>(),
            before.1
        );
    });
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        assert!(
            !view
                .read(app)
                .store
                .snapshot()
                .repos
                .iter()
                .any(|repo| repo.id == RepoId(1)),
            "the lazily built close button must keep its existing nested action"
        );
    });
}

#[gpui::test]
fn tab_slide_owners_survive_refresh_reorder_and_parent_scroll_but_leave_with_closed_tabs(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut state) = mount(cx);
    let bar = bar(&view, cx);
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        tab_frame(cx, 150);
        let first = cx.debug_bounds("repo_tab_1").unwrap();
        let (slide, drag_state, closed, closed_drag) = cx.update(|_, app| {
            let tabs = &bar.read(app).presentation.tabs;
            (
                tabs[0].slide.clone(),
                tabs[0].drag.state.clone(),
                Rc::downgrade(&tabs[TAB_COUNT - 1].slide),
                Rc::downgrade(&tabs[TAB_COUNT - 1].drag.state),
            )
        });
        state.repos.swap(0, 1);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                test_support::push_test_state(view, Arc::new(state.clone()), cx)
            })
        });
        cx.run_until_parked();
        tab_frame(cx, 0);
        let reordered = cx.debug_bounds("repo_tab_1").unwrap();
        assert_eq!(
            reordered.left(),
            first.left(),
            "reorder starts at the previous visual position"
        );
        let target = cx.update(|_, app| {
            let bar = bar.read(app);
            assert!(Rc::ptr_eq(&slide, &bar.presentation.tabs[1].slide));
            let target = bar.tab_scroll.tab_bounds(1).unwrap();
            assert_ne!(target.left(), first.left());
            target
        });
        tab_frame(cx, 50);
        let halfway = cx.debug_bounds("repo_tab_1").unwrap();
        assert!(halfway.left() > first.left() && halfway.left() < target.left());
        state.repos[1].spec.workdir = "/tmp/tab-cache/renamed-while-sliding".into();
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                test_support::push_test_state(view, Arc::new(state.clone()), cx)
            })
        });
        cx.run_until_parked();
        tab_frame(cx, 0);
        cx.update(|_, app| {
            let tab = &bar.read(app).presentation.tabs[1];
            assert!(Rc::ptr_eq(&slide, &tab.slide));
            assert!(Rc::ptr_eq(&drag_state, &tab.drag.state));
        });
        assert_eq!(
            cx.debug_bounds("repo_tab_1").unwrap().left(),
            halfway.left(),
            "presentation refresh must not restart or lose motion"
        );
        tab_frame(cx, 150);
        assert_eq!(cx.debug_bounds("repo_tab_1").unwrap().left(), target.left());
        for scale in [1.0, 1.25, 2.0] {
            cx.update(|window, _| window.set_scale_factor(scale));
            tab_frame(cx, 150);
            tab_frame(cx, 150);
            let logical_x = slide.borrow().as_ref().unwrap().layout_x;
            let painted = cx.debug_bounds("repo_tab_1").unwrap();
            cx.update(|_, app| {
                bar.update(app, |bar, cx| {
                    assert!(bar.tab_scroll.scroll_by(px(33.25)));
                    cx.notify();
                })
            });
            tab_frame(cx, 0);
            assert_eq!(
                slide.borrow().as_ref().unwrap().layout_x,
                logical_x,
                "fractional parent scroll must not be mistaken for a reorder at scale {scale}"
            );
            assert!(cx.debug_bounds("repo_tab_1").unwrap().left() < painted.left());
        }
        let removed = state.repos.pop().unwrap();
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                test_support::push_test_state(view, Arc::new(state.clone()), cx)
            })
        });
        cx.run_until_parked();
        tab_frame(cx, 150);
        tab_frame(cx, 0);
        assert!(
            closed.upgrade().is_none(),
            "closed tabs must release their retained animation owner"
        );
        assert!(
            closed_drag.upgrade().is_none(),
            "closed tabs release idle drag state"
        );
        state.repos.push(removed);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                test_support::push_test_state(view, Arc::new(state.clone()), cx)
            })
        });
        cx.run_until_parked();
        tab_frame(cx, 0);
        cx.update(|window, app| {
            assert!(window.is_a11y_active());
            let bar = bar.read(app);
            let reopened = &bar.presentation.tabs[TAB_COUNT - 1].slide;
            assert!(!std::rc::Weak::ptr_eq(&closed, &Rc::downgrade(reopened)));
            assert_eq!(reopened.borrow().as_ref().unwrap().from_offset_x, px(0.0));
            assert!((0..TAB_COUNT).all(|ix| bar.tab_scroll.tab_bounds(ix).is_some()));
        });
    });
}

#[test]
fn cloned_tab_drag_state_is_shared_and_a_new_gesture_resets_previous_direction() {
    let drag = RepoTabDrag::new(RepoId(1));
    let clone = drag.clone();
    drag.begin(px(20.0), px(100.0));
    clone.state.tab_width.set(px(126.0));
    assert_eq!(drag.center_x(px(120.0)), px(163.0));
    assert_eq!(clone.update_direction(px(140.0), px(12.0)), 1);
    assert_eq!(drag.update_direction(px(135.0), px(12.0)), 1);
    assert_eq!(clone.update_direction(px(120.0), px(12.0)), -1);
    drag.begin(px(8.0), px(200.0));
    assert_eq!(clone.state.direction.get(), 0);
    assert_eq!(clone.state.tab_width.get(), px(0.0));
    assert_eq!(clone.center_x(px(200.0)), px(200.0));
    assert_eq!(clone.update_direction(px(210.0), px(12.0)), 1);
    assert_eq!(drag.state.cursor_offset_x.get(), px(8.0));
}

#[gpui::test]
fn forced_accessibility_tab_drag_reorders_and_releases_the_retained_slide(cx: &mut TestAppContext) {
    let (view, cx, _) = mount(cx);
    let bar = bar(&view, cx);
    // Tab dragging and its slide clock are independent of UiRuntime. Keep
    // unrelated history cache rebuilds on the deterministic test executor.
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::deterministic(), || {
        tab_frame(cx, 150);
        let first = cx.debug_bounds("repo_tab_1").unwrap();
        let second = cx.debug_bounds("repo_tab_2").unwrap();
        let start = first.center();
        cx.simulate_mouse_move(start, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            start + point(px(20.0), px(0.0)),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        let drag = cx.update(|_, app| {
            app.active_drag()
                .expect("pointer movement starts the actual tab drag")
                .value
                .downcast_ref::<RepoTabDrag>()
                .unwrap()
                .clone()
        });
        // GPUI captures the offset at the threshold-crossing mouse move.
        assert_eq!(
            drag.state.cursor_offset_x.get(),
            start.x + px(20.0) - first.left()
        );
        let destination = point(second.right() - px(10.0), start.y);
        cx.simulate_mouse_move(
            destination,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        test_support::drain_store_worker(&view, cx);
        let reordered = cx.update(|_, app| view.read(app).store.snapshot());
        assert_eq!(
            (reordered.repos[0].id, reordered.repos[1].id),
            (RepoId(2), RepoId(1)),
            "drag must still use its shared width/anchor and dispatch a real reorder"
        );
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                test_support::push_test_state(view, reordered.clone(), cx)
            })
        });
        cx.run_until_parked();
        tab_frame(cx, 0);
        assert!(drag.state.tab_width.get() > px(0.0));
        let held = cx.debug_bounds("repo_tab_1").unwrap();
        assert!(
            (held.left() - (destination.x - drag.state.cursor_offset_x.get())).abs() <= px(1.0)
        );
        cx.simulate_mouse_up(
            destination,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        tab_frame(cx, 0);
        let released = cx.debug_bounds("repo_tab_1").unwrap();
        assert!(
            (released.left() - held.left()).abs() <= px(1.0),
            "release starts from the held visual position"
        );
        tab_frame(cx, 150);
        let settled = cx.debug_bounds("repo_tab_1").unwrap();
        cx.update(|window, app| {
            assert!(window.is_a11y_active());
            assert!(!app.has_active_drag());
            let bar = bar.read(app);
            assert!(bar.repo_tab_drag_visual.is_none());
            let target = bar.tab_scroll.tab_bounds(1).unwrap();
            assert!((settled.left() - target.left()).abs() <= px(1.0));
        });
        // A later gesture reuses the owner's allocation and must initialize
        // its cursor offset again, rather than retaining the previous grab.
        let next_start = point(settled.left() + px(25.0), settled.center().y);
        cx.simulate_mouse_down(
            next_start,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_move(
            next_start + point(px(20.0), px(0.0)),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.update(|_, app| {
            let again = app
                .active_drag()
                .unwrap()
                .value
                .downcast_ref::<RepoTabDrag>()
                .unwrap();
            assert!(Rc::ptr_eq(&drag.state, &again.state));
            assert_eq!(again.state.cursor_offset_x.get(), px(45.0));
        });
        cx.simulate_mouse_up(
            next_start + point(px(20.0), px(0.0)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        tab_frame(cx, 150);
    });
}
