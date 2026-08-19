use super::*;
use crate::kit::{TextInput, TextInputOptions};
use crate::view::components::{
    PICKER_LIST_MAX_HEIGHT_PX, PickerPrompt, PickerPromptGeometry, PickerPromptItem,
    PickerPromptLayout, picker_prompt_layout,
};
use crate::view::panels::{benchmark_branch_checkout_rows, benchmark_workspace_rows};
use gpui::{Entity, IntoElement, Render, ScrollHandle, Window, px};
use rustc_hash::{FxHashMap, FxHasher};

/// Which badge picker a run measures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerPromptKind {
    /// The branch badge's checkout picker: sectioned rows over every local and
    /// remote branch, each with an author / date / summary detail line.
    BranchCheckout,
    /// The workspace badge's picker: one row per worktree plus a create row.
    Workspace,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PickerPromptFrameMetrics {
    pub local_branches: u64,
    pub remote_branches: u64,
    pub worktrees: u64,
    /// Rows the query left in the list.
    pub rows_matched: u64,
    /// Rows the frame actually built elements for. The gap between this and
    /// `rows_matched` is what windowing saves.
    pub rows_rendered: u64,
    /// Text parts across the rendered rows, and how many of those carry a
    /// tooltip — each tooltip adds an element id, a hover listener and a hitbox
    /// the pointer is tested against on every move.
    pub text_parts: u64,
    pub tooltip_parts: u64,
    pub content_height_px: u64,
}

/// Drives the row model and a real window draw of one badge picker.
///
/// The two costs are measured apart because they change apart: the row model is
/// rebuilt only when the repository data changes (`popover::rows_cache`), while
/// the element tree is rebuilt on every frame — including the frames a hover
/// moving from row to row causes, which is what made these pickers feel slow.
pub struct PickerPromptFrameFixture {
    // Declared before `cx`: the window handle must go before the test context,
    // which asserts on drop that nothing still holds an entity of its app.
    window: gpui::WindowHandle<PickerPromptBenchView>,
    cx: gpui::TestAppContext,
    repo: RepoState,
    kind: PickerPromptKind,
    query: String,
    local_branches: usize,
    remote_branches: usize,
    worktrees: usize,
}

impl PickerPromptFrameFixture {
    pub fn new(kind: PickerPromptKind, refs: usize, worktrees: usize) -> Self {
        // Split the ref budget the way a real repository does: a few local
        // branches, many remote-tracking ones.
        let local_branches = (refs / 4).max(1);
        let remote_branches = refs.saturating_sub(local_branches);
        let commits = build_synthetic_commits(1);
        let mut repo = build_synthetic_repo_state(
            local_branches,
            remote_branches,
            1,
            worktrees,
            0,
            0,
            &commits,
        );
        repo.open = Loadable::Ready(());
        repo.ref_metadata = Loadable::Ready(Arc::new(build_synthetic_ref_metadata(&repo)));

        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(|window, cx| PickerPromptBenchView::new(window, cx));

        Self {
            window,
            cx,
            repo,
            kind,
            query: String::new(),
            local_branches,
            remote_branches,
            worktrees,
        }
    }

    pub fn with_query(mut self, query: &str) -> Self {
        self.query = query.to_string();
        self
    }

    /// Cost of building the row model and filtering it — what a frame used to
    /// repeat, and what now happens once per change to the repository.
    pub fn run_rows_build(&mut self) -> u64 {
        self.run_rows_build_with_metrics().0
    }

    pub fn run_rows_build_with_metrics(&mut self) -> (u64, PickerPromptFrameMetrics) {
        let (items, layout) = self.build_rows();
        let mut metrics = self.base_metrics();
        metrics.rows_matched = layout.item_indices.len() as u64;
        metrics.rows_rendered = metrics.rows_matched;
        self.count_parts(&items, &layout, 0..layout.item_indices.len(), &mut metrics);
        (hash_picker_rows(&items, &layout), metrics)
    }

    /// Cost of one frame of the picker as it ships: the row model is prebuilt
    /// (as the cache makes it), and only the rows the viewport can show are
    /// turned into elements.
    pub fn run_frame(&mut self) -> u64 {
        self.run_frame_with_metrics().0
    }

    pub fn run_frame_with_metrics(&mut self) -> (u64, PickerPromptFrameMetrics) {
        self.draw_frame(true)
    }

    /// Cost of the same frame with every matched row turned into an element:
    /// the behaviour before the list was windowed. Reached by handing the picker
    /// a viewport tall enough to hold the whole list, which is the same test
    /// `PickerPromptGeometry::window` applies — a list that fits is never
    /// windowed.
    pub fn run_frame_full_list(&mut self) -> u64 {
        self.run_frame_full_list_with_metrics().0
    }

    pub fn run_frame_full_list_with_metrics(&mut self) -> (u64, PickerPromptFrameMetrics) {
        self.draw_frame(false)
    }

    fn draw_frame(&mut self, windowed: bool) -> (u64, PickerPromptFrameMetrics) {
        let (items, layout) = self.build_rows();
        let items: Rc<[PickerPromptItem]> = Rc::from(items);
        let layout = Rc::new(layout);
        let hash = hash_picker_rows(&items, &layout);

        let geometry = PickerPromptGeometry::new(&items, &layout, 100u32);
        let max_height = if windowed {
            px(PICKER_LIST_MAX_HEIGHT_PX)
        } else {
            geometry.total_height()
        };
        let rendered = geometry.visible_rows(px(0.0), max_height);
        let mut metrics = self.base_metrics();
        metrics.rows_matched = layout.item_indices.len() as u64;
        metrics.rows_rendered = rendered.len() as u64;
        metrics.content_height_px = f32::from(geometry.total_height()).round() as u64;
        self.count_parts(&items, &layout, rendered, &mut metrics);

        let query = self.query.clone();
        self.window
            .update(&mut self.cx, |view, _window, cx| {
                view.set_rows(
                    Rc::clone(&items),
                    Rc::clone(&layout),
                    max_height,
                    &query,
                    cx,
                );
            })
            .expect("picker benchmark window should stay open");
        self.cx
            .update_window(self.window.into(), |_, window, app| {
                // A hover moving between rows notifies the popover host, so the
                // frame this measures is a full re-render of the picker.
                window.refresh();
                let _ = window.draw(app);
            })
            .expect("picker benchmark window should stay open");

        (hash, metrics)
    }

    fn build_rows(&self) -> (Vec<PickerPromptItem>, PickerPromptLayout) {
        // A fixed clock keeps the relative dates on the detail line — and so the
        // shaped text and its cache keys — identical across iterations.
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let items = match self.kind {
            PickerPromptKind::BranchCheckout => {
                benchmark_branch_checkout_rows(&self.repo, &self.query, now)
            }
            PickerPromptKind::Workspace => benchmark_workspace_rows(&self.repo, &self.query),
        };
        let layout = picker_prompt_layout(&items, &self.query);
        (items, layout)
    }

    fn base_metrics(&self) -> PickerPromptFrameMetrics {
        PickerPromptFrameMetrics {
            local_branches: self.local_branches as u64,
            remote_branches: self.remote_branches as u64,
            worktrees: self.worktrees as u64,
            ..PickerPromptFrameMetrics::default()
        }
    }

    fn count_parts(
        &self,
        items: &[PickerPromptItem],
        layout: &PickerPromptLayout,
        rendered: Range<usize>,
        metrics: &mut PickerPromptFrameMetrics,
    ) {
        for display_ix in rendered {
            let Some(item) = layout
                .item_indices
                .get(display_ix)
                .and_then(|ix| items.get(*ix))
            else {
                continue;
            };
            metrics.text_parts += item.debug_part_count() as u64;
            metrics.tooltip_parts += item.debug_tooltip_part_count() as u64;
        }
    }
}

/// Author / date / summary for every ref the picker will list, as the checkout
/// picker's detail lines need it.
fn build_synthetic_ref_metadata(
    repo: &RepoState,
) -> FxHashMap<String, gitcomet_core::domain::RefMetadata> {
    let mut metadata = FxHashMap::default();
    let mut insert = |name: String, ix: usize| {
        metadata.insert(
            name,
            gitcomet_core::domain::RefMetadata {
                author: format!("Author {}", ix % 32),
                committed_at: 1_700_000_000 - (ix as i64 * 3_600),
                summary: format!("Refactor the {ix} thing so the next one lands cleanly"),
            },
        );
    };

    if let Loadable::Ready(branches) = &repo.branches {
        for (ix, branch) in branches.iter().enumerate() {
            insert(branch.name.clone(), ix);
        }
    }
    if let Loadable::Ready(remote_branches) = &repo.remote_branches {
        for (ix, remote_branch) in remote_branches.iter().enumerate() {
            insert(
                format!("{}/{}", remote_branch.remote, remote_branch.name),
                ix,
            );
        }
    }
    metadata
}

fn hash_picker_rows(items: &[PickerPromptItem], layout: &PickerPromptLayout) -> u64 {
    let mut hasher = FxHasher::default();
    layout.item_indices.len().hash(&mut hasher);
    for (display_ix, item_ix) in layout.item_indices.iter().enumerate() {
        display_ix.hash(&mut hasher);
        item_ix.hash(&mut hasher);
        if let Some(item) = items.get(*item_ix) {
            item.debug_display_text().hash(&mut hasher);
            item.debug_secondary_text().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Renders nothing but one picker, so a drawn frame measures the picker rather
/// than the rest of the application around it.
pub struct PickerPromptBenchView {
    theme: AppTheme,
    query_input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
    items: Rc<[PickerPromptItem]>,
    layout: Rc<PickerPromptLayout>,
    /// Viewport the list is drawn into. A benchmark measuring an unwindowed
    /// frame makes this as tall as the whole list.
    max_height: Pixels,
}

impl PickerPromptBenchView {
    fn new(window: &mut Window, cx: &mut gpui::Context<Self>) -> Self {
        let query_input = cx.new(|cx| {
            TextInput::new(
                TextInputOptions {
                    placeholder: "Filter branches".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        Self {
            theme: AppTheme::gitcomet_dark(),
            query_input,
            scroll_handle: ScrollHandle::new(),
            items: Rc::from(Vec::new()),
            layout: Rc::new(PickerPromptLayout::default()),
            max_height: px(PICKER_LIST_MAX_HEIGHT_PX),
        }
    }

    fn set_rows(
        &mut self,
        items: Rc<[PickerPromptItem]>,
        layout: Rc<PickerPromptLayout>,
        max_height: Pixels,
        query: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        self.items = items;
        self.layout = layout;
        self.max_height = max_height;
        let current = self.query_input.read(cx).text().to_string();
        if current != query {
            let query = query.to_string();
            self.query_input.update(cx, |input, cx| {
                input.set_text(&query, cx);
            });
        }
        cx.notify();
    }
}

impl Render for PickerPromptBenchView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        PickerPrompt::new(self.query_input.clone(), self.scroll_handle.clone())
            .prebuilt_items(Rc::clone(&self.items), Rc::clone(&self.layout))
            .max_height(self.max_height)
            .render(self.theme, 100u32, cx, |_, _, _, _, _| {})
    }
}
