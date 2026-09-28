use super::*;
use crate::kit::{ScrollbarAxis, ScrollbarDriver};
use crate::view::rows::benchmarks::{
    bench_app_state, build_synthetic_commits, build_synthetic_repo_state,
};
use gpui::InputEvent as _;

/// Draws the production sidebar and its production scrollbar driver. Repository
/// construction and the first row-cache build happen outside the timed frames.
pub struct SidebarStickyFrameFixture {
    window: gpui::WindowHandle<SidebarBenchView>,
    cx: gpui::TestAppContext,
    step: usize,
}

struct SidebarBenchView {
    _root: Entity<GitCometView>,
    pane: Entity<SidebarPaneView>,
    rows: Rc<[BranchSidebarRow]>,
    pins: Rc<[BranchSidebarRow]>,
}

impl Render for SidebarBenchView {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(320.0)).h_full().child(self.pane.clone())
    }
}

impl SidebarStickyFrameFixture {
    pub fn new(branches: usize, remotes: usize, stashes: usize, pins: usize) -> Self {
        let mut repo = build_synthetic_repo_state(
            branches / 2,
            branches / 2,
            remotes,
            20,
            20,
            stashes,
            &build_synthetic_commits(1),
        );
        repo.open = Loadable::Ready(());
        let mut pinned = BTreeSet::new();
        if let Loadable::Ready(branches) = &repo.branches {
            for branch in branches.iter().take(pins / 2) {
                pinned.insert(branch_sidebar::branch_pin_storage_key(
                    BranchSection::Local,
                    &branch.name,
                ));
            }
            if let Some(branch) = branches.get(1) {
                repo.head_branch = Loadable::Ready(branch.name.clone());
            }
        }
        if let Loadable::Ready(branches) = &repo.remote_branches {
            for branch in branches.iter().take(pins / 2) {
                pinned.insert(branch_sidebar::branch_pin_storage_key(
                    BranchSection::Remote,
                    &format!("{}/{}", branch.remote, branch.name),
                ));
            }
        }
        let selected = repo.remote_branches.ready().and_then(|branches| {
            branches.get(branches.len() / 2).map(|branch| {
                (
                    BranchMenuTarget::remote(&branch.remote, &branch.name),
                    branch.target.clone(),
                )
            })
        });
        repo.history_state.selected_commit = selected.as_ref().map(|(_, tip)| tip.clone());
        let path = repo.spec.workdir.clone();
        let id = repo.id;
        let state = Arc::new(bench_app_state(vec![repo], Some(id)));
        let (store, events) =
            AppStore::new_test(Arc::new(gitcomet_core::services::UnavailableGitBackend));
        store.replace_snapshot_for_test(state.clone());
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(|window, cx| {
            let root = cx.new(|cx| {
                GitCometView::new_with_config(
                    store,
                    events,
                    GitCometViewConfig::normal(None),
                    window,
                    cx,
                )
            });
            let pane = root.read(cx).sidebar_pane.clone();
            let presentation = pane.update(cx, |pane, cx| {
                pane.state = state;
                if let Some((selected, _)) = selected {
                    pane.set_selected_branch(id, selected, None, cx);
                }
                pane.sidebar_pinned_branches_by_repo.insert(path, pinned);
                pane.sidebar_presentation_cache = SidebarPresentationCache::default();
                let presentation = pane.branch_sidebar_presentation_cached().unwrap();
                cx.notify();
                presentation
            });
            SidebarBenchView {
                _root: root,
                pane,
                rows: presentation.rows,
                pins: presentation.pins,
            }
        });
        let mut fixture = Self {
            window,
            cx,
            step: 0,
        };
        fixture.draw();
        fixture
    }

    /// One real GPUI frame after a wheel gesture or scrollbar drag step.
    /// Returns the number of row elements rendered, including sticky headers.
    pub fn run_frame(&mut self, drag: bool) -> usize {
        self.step = self.step.wrapping_add(1);
        self.window
            .update(&mut self.cx, |view, window, cx| {
                let scroll = view.pane.read(cx).branches_scroll.clone();
                view.pane.update(cx, |pane, _| pane.rendered_rows = 0);
                if drag {
                    let fraction = (self.step % 97) as f32 / 96.0;
                    scroll.set_axis_offset(
                        ScrollbarAxis::Vertical,
                        -scroll.max_offset(ScrollbarAxis::Vertical) * fraction,
                    );
                } else {
                    let handle = scroll.0.borrow();
                    let direction = if -handle.base_handle.offset().y + px(300.0)
                        >= handle.base_handle.max_offset().y
                    {
                        300.0
                    } else {
                        -48.0
                    };
                    let event = gpui::ScrollWheelEvent {
                        position: handle.base_handle.bounds().center(),
                        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(direction))),
                        modifiers: Default::default(),
                        touch_phase: gpui::TouchPhase::Moved,
                    };
                    drop(handle);
                    window.dispatch_event(event.to_platform_input(), cx);
                }
                // TestAppContext paints dirty windows when this update ends.
                // Request exactly one frame for either input path.
                window.refresh();
            })
            .unwrap();
        self.window
            .update(&mut self.cx, |view, _, cx| {
                view.pane.update(cx, |pane, _| {
                    let presentation = pane.branch_sidebar_presentation_cached().unwrap();
                    assert!(
                        Rc::ptr_eq(&presentation.rows, &view.rows),
                        "scroll rebuilt main rows"
                    );
                    assert!(
                        Rc::ptr_eq(&presentation.pins, &view.pins),
                        "scroll rebuilt pinned rows"
                    );
                    assert!(
                        pane.rendered_rows > 0 && pane.rendered_rows < 200,
                        "sidebar rendering must remain bounded"
                    );
                    pane.rendered_rows
                })
            })
            .unwrap()
    }

    /// Arithmetic only: no view update, row construction, or heap allocation.
    pub fn geometry_step(scroll: f32) -> f32 {
        let headers = [0, 1, 25, 1_200, 1_201, 3_000, 9_000, 18_000];
        headers
            .into_iter()
            .enumerate()
            .map(|(rank, row)| {
                crate::view::sidebar_sticky::row_y(row, rank, headers.len(), scroll, 640.0, 24.0)
            })
            .sum()
    }

    fn draw(&mut self) {
        self.cx
            .update_window(self.window.into(), |_, window, app| {
                window.refresh();
                window.draw(app).clear(app);
            })
            .unwrap();
    }
}

#[cfg(test)]
#[test]
fn sidebar_sticky_frame_fixture_keeps_large_lists_bounded() {
    let _guard = crate::test_support::lock_visual_test();
    for (refs, remotes, stashes, pins) in [(1_000, 2, 50, 8), (20_000, 100, 50_000, 2_000)] {
        let mut fixture = SidebarStickyFrameFixture::new(refs, remotes, stashes, pins);
        for drag in [false, true, false, true] {
            assert!(fixture.run_frame(drag) > 0);
        }
    }
}
