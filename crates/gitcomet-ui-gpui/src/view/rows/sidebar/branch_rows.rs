//! Branch sidebar row assembly: sections, branches, remotes, stashes,
//! worktrees and submodules.

use super::*;

mod annex;
mod branch;
mod headers;
mod items;

const STASH_ICON_PATH: &str = crate::view::icons::STASH_ICON_PATH;

const BRANCH_TREE_BASE_PAD_PX: f32 = 8.0;
const BRANCH_TREE_DEPTH_STEP_PX: f32 = 14.0;
const BRANCH_TREE_TOGGLE_SLOT_PX: f32 = 14.0;
const BRANCH_TREE_ICON_SLOT_PX: f32 = 16.0;
const BRANCH_TREE_GAP_PX: f32 = 6.0;
const BRANCH_BADGE_GAP_PX: f32 = 3.0;
/// Widest a branch row's worktree pill may grow before its label starts
/// truncating. Wide enough for the folder names worktrees usually carry,
/// narrow enough that the pill always fits the pane — which is what
/// keeps every row's badge on one right edge.
const BRANCH_WORKTREE_BADGE_MAX_W_PX: f32 = 140.0;
/// Gap between a row's trailing badge and the row's right edge, shared
/// by every row (headers included) so the badges land on one edge.
/// The expanded sidebar adds its content inset here so full-width
/// backgrounds still leave room for the overlay scrollbar.
const BRANCH_ROW_TRAILING_PAD_PX: f32 = 4.0;

/// What every row of one `render_sidebar_rows` call shares, built once per
/// call. Its methods are the shared row pieces (icons, slots, indents).
struct SidebarRowContext {
    repo_id: RepoId,
    /// The list's surface, before a row's pinned override.
    surface: SidebarRowSurface,
    theme: AppTheme,
    ui_scale_percent: u32,
    scale: ui_scale::UiScale,
    is_collapsed_popover: bool,
    header_activation: controls::ControlActivation,
    filter_query: std::rc::Rc<crate::view::sidebar_search::SidebarSearch>,
    pin_count: usize,
    row_keys: std::rc::Rc<[SharedString]>,
    worktree_badges: crate::view::sidebar_presentation::WorktreeBadgeIndex,
    repo_workdir: Option<std::path::PathBuf>,
    worktree_badge_palette: WorktreeBadgePalette,
    icon_primary: gpui::Rgba,
    icon_current: gpui::Rgba,
    icon_muted: gpui::Rgba,
    selected_branch_commit_id: Option<CommitId>,
    selected_commit: Option<CommitId>,
    content_inset: f32,
}

/// One row's list index and resolved background.
#[derive(Clone, Copy)]
struct SidebarRowSlot {
    ix: usize,
    stuck: bool,
    row_style: components::InteractiveRowStyle,
    row_surface: gpui::Rgba,
}

impl SidebarRowContext {
    fn new(
        this: &mut SidebarPaneView,
        repo_id: RepoId,
        surface: SidebarRowSurface,
        presentation: SidebarPresentation,
        ui_scale_percent: u32,
    ) -> Self {
        let is_collapsed_popover = surface == SidebarRowSurface::Rail;
        let theme = this.theme;
        Self {
            repo_id,
            surface,
            theme,
            ui_scale_percent,
            scale: ui_scale_percent.into(),
            is_collapsed_popover,
            header_activation: if is_collapsed_popover {
                controls::ControlActivation::Composite
            } else {
                controls::ControlActivation::Action
            },
            filter_query: presentation.search,
            pin_count: presentation.pins.len(),
            row_keys: presentation.row_keys,
            worktree_badges: presentation.worktree_badges,
            repo_workdir: this.active_repo().map(|r| r.spec.workdir.clone()),
            worktree_badge_palette: worktree_badge_palette(theme),
            icon_primary: theme.colors.foreground.secondary,
            icon_current: theme.colors.accent.foreground,
            icon_muted: with_alpha(
                theme.colors.foreground.secondary,
                if theme.is_dark { 0.70 } else { 0.78 },
            ),
            selected_branch_commit_id: this.sidebar_selected_tip(),
            selected_commit: this
                .active_repo()
                .and_then(|repo| repo.history_state.selected_commit.clone()),
            content_inset: if is_collapsed_popover {
                0.0
            } else {
                components::ROW_HIGHLIGHT_INSET_PX
            },
        }
    }

    fn px(&self, value: f32) -> Pixels {
        self.scale.px(value)
    }

    fn row_height(&self) -> Pixels {
        sidebar_list_row_height(self.theme, self.ui_scale_percent)
    }

    fn trailing_pad(&self) -> Pixels {
        self.px(self.content_inset + BRANCH_ROW_TRAILING_PAD_PX)
    }

    fn indent_px(&self, depth: usize) -> Pixels {
        self.px(self.content_inset
            + BRANCH_TREE_BASE_PAD_PX
            + depth as f32 * BRANCH_TREE_DEPTH_STEP_PX)
    }

    fn svg_icon(&self, path: &'static str, color: gpui::Rgba, size_px: f32) -> gpui::Svg {
        crate::view::icons::svg_icon(path, color, self.px(size_px))
    }

    fn svg_spinner(
        &self,
        id: (&'static str, u64),
        color: gpui::Rgba,
        size_px: f32,
    ) -> impl IntoElement + use<> {
        crate::view::icons::svg_spinner(id, color, self.px(size_px))
    }

    fn svg_collapse(&self, collapsed: bool) -> gpui::Svg {
        self.svg_icon(
            if collapsed {
                "icons/arrow_right.svg"
            } else {
                "icons/chevron_down.svg"
            },
            self.icon_muted,
            12.0,
        )
    }

    fn tree_toggle_slot(&self, collapsed: Option<bool>) -> gpui::Div {
        div()
            .w(self.px(BRANCH_TREE_TOGGLE_SLOT_PX))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when_some(collapsed, |this, collapsed| {
                this.child(self.svg_collapse(collapsed))
            })
    }

    fn tree_icon_slot(&self, path: &'static str, color: gpui::Rgba, size_px: f32) -> gpui::Div {
        div()
            .w(self.px(BRANCH_TREE_ICON_SLOT_PX))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(self.svg_icon(path, color, size_px))
    }

    fn branch_tree_color(&self, section: BranchSection) -> gpui::Rgba {
        match section {
            BranchSection::Local => self.theme.colors.foreground.primary,
            BranchSection::Remote => self.theme.colors.foreground.secondary,
        }
    }

    fn paint_header(&self, row: AnyElement, background: gpui::Rgba) -> AnyElement {
        if self.is_collapsed_popover {
            row
        } else {
            div().w_full().bg(background).child(row).into_any_element()
        }
    }

    /// The muted message an empty or unloaded section shows.
    fn placeholder_row(&self, id: &'static str, ix: usize, message: SharedString) -> AnyElement {
        div()
            .id((id, ix))
            .h(self.row_height())
            .w_full()
            .px_2()
            .text_size(self.theme.ui_text(14.0))
            .text_color(self.theme.colors.foreground.secondary)
            .child(message)
            .into_any_element()
    }

    fn divergence_badge(
        &self,
        icon_path: &'static str,
        color: gpui::Rgba,
        count: NonZeroU32,
        debug_selector: Option<String>,
    ) -> gpui::Div {
        let theme = self.theme;
        let mut badge = div()
            .flex()
            .items_center()
            .gap_1()
            .text_size(theme.ui_text(12.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .child(self.svg_icon(icon_path, color, 11.0))
            .child(crate::view::branch_sidebar::branch_sidebar_divergence_label(count));
        if let Some(debug_selector) = debug_selector {
            badge = badge.debug_selector(move || debug_selector.clone());
        }
        badge
    }

    fn upstream_badge(&self, debug_selector: Option<String>) -> gpui::Div {
        let theme = self.theme;
        // The active upstream is a ref relationship, but it
        // uses the same compact control-shaped chip as the
        // worktree badge so the sidebar's status badges read as
        // one family.
        let colors = upstream_badge_colors(self.worktree_badge_palette);
        let mut badge = div()
            .flex()
            .items_center()
            .gap(self.px(3.0))
            .px(self.px(6.0))
            .rounded(px(theme.radii.control))
            .text_size(theme.ui_text(11.0))
            .text_color(colors.text)
            .bg(colors.bg)
            .border_1()
            .border_color(colors.border)
            .child(self.svg_icon("icons/cloud.svg", colors.icon, 9.0))
            .child("Upstream");
        if let Some(debug_selector) = debug_selector {
            badge = badge.debug_selector(move || debug_selector.clone());
        }
        badge
    }
}

impl SidebarPaneView {
    pub(in crate::view) fn render_branch_sidebar_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let surface = if this.collapsed_popover_section.is_some() {
            SidebarRowSurface::Rail
        } else {
            SidebarRowSurface::Tree
        };
        let Some(presentation) = this.branch_sidebar_presentation_cached() else {
            return Vec::new();
        };
        Self::render_sidebar_rows(
            this,
            range.map(|ix| (ix, false)),
            surface,
            presentation,
            _window,
            cx,
        )
    }

    pub(in crate::view) fn render_sidebar_rows(
        this: &mut Self,
        // Decorated rows may still be at their natural position. The flag
        // identifies rows actually held at an edge, not just eligible ones.
        range: impl Iterator<Item = (usize, bool)>,
        surface: SidebarRowSurface,
        presentation: SidebarPresentation,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        #[cfg(any(test, feature = "benchmarks"))]
        {
            this.rendered_rows += range.size_hint().0;
        }
        let ui_scale_percent = ui_scale::current(cx).percent;

        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        let rows = presentation.rows.clone();
        let ctx = SidebarRowContext::new(this, repo_id, surface, presentation, ui_scale_percent);
        let decorated: std::rc::Rc<[usize]> = if surface == SidebarRowSurface::Tree {
            this.decorated_sidebar_rows()
        } else {
            std::rc::Rc::from([])
        };
        range
            .filter_map(|(ix, stuck)| {
                rows.get(ix).cloned().map(|row| {
                    (
                        ix,
                        if decorated.binary_search(&ix).is_ok() {
                            BranchSidebarRow::SectionSpacer
                        } else {
                            row
                        },
                        stuck,
                    )
                })
            })
            .map(|(ix, row, stuck)| {
                let surface = if ix < ctx.pin_count && surface != SidebarRowSurface::Rail {
                    SidebarRowSurface::Pins
                } else {
                    surface
                };
                let row_surface = sidebar_row_background(ctx.theme, surface, &row, stuck);
                let row_style = components::InteractiveRowStyle::new(ctx.theme, row_surface).flat();
                let slot = SidebarRowSlot {
                    ix,
                    stuck,
                    row_style,
                    row_surface,
                };
                (row, slot)
            })
            // One method per row kind keeps each kind's builder temporaries in
            // its own stack frame (opt-level 0 gives every temporary a slot).
            .map(|(row, slot)| match row {
                BranchSidebarRow::ContributionHeader {
                    section,
                    title,
                    collapsed,
                    ..
                } => this.contributed_header(section, title, collapsed, cx),
                BranchSidebarRow::ContributionItem { section, row, .. } => {
                    this.contributed_row(section, row, cx)
                }
                BranchSidebarRow::SectionHeader {
                    section,
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => Self::sidebar_section_header_row(
                    this,
                    &ctx,
                    slot,
                    section,
                    collapsed,
                    collapse_key,
                    cx,
                ),
                // A full slot: uniform_list sizes every row from item 0.
                BranchSidebarRow::SectionSpacer => div()
                    .id(("branch_section_spacer", slot.ix))
                    .h(ctx.row_height())
                    .w_full()
                    .into_any_element(),
                BranchSidebarRow::StashHeader {
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => Self::sidebar_stash_header_row(this, &ctx, slot, collapsed, collapse_key, cx),
                BranchSidebarRow::StashPlaceholder { message } => {
                    ctx.placeholder_row("stash_placeholder", slot.ix, message)
                }
                BranchSidebarRow::StashItem {
                    index,
                    message,
                    tooltip,
                    ..
                } => Self::sidebar_stash_item_row(this, &ctx, slot, index, message, tooltip, cx),
                BranchSidebarRow::Placeholder {
                    section: _,
                    message,
                } => ctx.placeholder_row("branch_placeholder", slot.ix, message),
                BranchSidebarRow::WorktreesHeader {
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => Self::sidebar_worktrees_header_row(
                    this,
                    &ctx,
                    slot,
                    collapsed,
                    collapse_key,
                    cx,
                ),
                BranchSidebarRow::WorktreePlaceholder { message } => {
                    ctx.placeholder_row("worktree_placeholder", slot.ix, message)
                }
                BranchSidebarRow::WorktreeItem {
                    path,
                    branch,
                    detached,
                    is_active,
                } => Self::sidebar_worktree_item_row(
                    this, &ctx, slot, path, branch, detached, is_active, cx,
                ),
                BranchSidebarRow::SubmodulesHeader {
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => Self::sidebar_submodules_header_row(
                    this,
                    &ctx,
                    slot,
                    collapsed,
                    collapse_key,
                    cx,
                ),
                BranchSidebarRow::AnnexHeader {
                    collapsed,
                    collapse_key,
                    summary,
                } => Self::sidebar_annex_header_row(
                    this,
                    &ctx,
                    slot,
                    collapsed,
                    collapse_key,
                    summary,
                    cx,
                ),
                BranchSidebarRow::AnnexPlaceholder {
                    message,
                    can_init,
                    can_restage,
                } => Self::sidebar_annex_placeholder_row(
                    &ctx,
                    slot,
                    message,
                    can_init,
                    can_restage,
                    cx,
                ),
                BranchSidebarRow::AnnexRepositoryItem {
                    uuid,
                    name,
                    detail,
                    here,
                    untrusted,
                } => Self::sidebar_annex_repository_row(
                    this, &ctx, slot, uuid, name, detail, here, untrusted, cx,
                ),
                BranchSidebarRow::SubmodulePlaceholder { message, can_load } => {
                    Self::sidebar_submodule_placeholder_row(&ctx, slot.ix, message, can_load, cx)
                }
                BranchSidebarRow::SubmoduleItem {
                    path,
                    status,
                    recorded_head,
                    checked_out_head,
                } => Self::sidebar_submodule_item_row(
                    this,
                    &ctx,
                    slot,
                    path,
                    status,
                    recorded_head,
                    checked_out_head,
                    cx,
                ),
                BranchSidebarRow::RemoteHeader {
                    name,
                    collapsed,
                    collapse_key,
                } => Self::sidebar_remote_header_row(
                    this,
                    &ctx,
                    slot,
                    name,
                    collapsed,
                    collapse_key,
                    cx,
                ),
                BranchSidebarRow::GroupHeader {
                    label,
                    path,
                    remote,
                    section,
                    depth,
                    collapsed,
                    collapse_key,
                } => Self::sidebar_group_header_row(
                    this,
                    &ctx,
                    slot,
                    label,
                    path,
                    remote,
                    section,
                    depth,
                    collapsed,
                    collapse_key,
                    cx,
                ),
                BranchSidebarRow::Branch {
                    name,
                    target,
                    section,
                    depth,
                    muted,
                    divergence_ahead,
                    divergence_behind,
                    is_head,
                    is_upstream,
                } => Self::sidebar_branch_row(
                    this,
                    &ctx,
                    slot,
                    name,
                    target,
                    section,
                    depth,
                    muted,
                    divergence_ahead,
                    divergence_behind,
                    is_head,
                    is_upstream,
                    cx,
                ),
            })
            .collect()
    }
}
