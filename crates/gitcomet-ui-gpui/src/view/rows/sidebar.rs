use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::ui_scale;
use crate::view::components::InteractiveRowExt as _;
use crate::view::sidebar_presentation::SidebarPresentation;
use crate::view::sidebar_sticky::SidebarRowSurface;
use gitcomet_core::domain::LogScope;
use gitcomet_core::domain::SubmoduleStatus;
use palette::IntoColor;
use std::num::NonZeroU32;

pub(in crate::view) const WORKTREE_ICON_PATH: &str = "icons/git_worktree.svg";

pub(in crate::view) fn sidebar_row_background(
    theme: AppTheme,
    surface: SidebarRowSurface,
    row: &BranchSidebarRow,
    stuck: bool,
) -> gpui::Rgba {
    match surface {
        SidebarRowSurface::Rail => theme.colors.surface.raised,
        SidebarRowSurface::Pins => theme.colors.surface.panel,
        _ if !stuck
            && matches!(
                row,
                BranchSidebarRow::GroupHeader { .. } | BranchSidebarRow::RemoteHeader { .. }
            ) =>
        {
            theme.colors.surface.chrome
        }
        _ if crate::view::sidebar_sticky::header_key(row).is_some() => theme.colors.surface.panel,
        _ => theme.colors.surface.chrome,
    }
}

/// Row height of every continuous list in the sidebar. The tabs swap lists in
/// place, so a differing rhythm would make the rows jump.
const SIDEBAR_TREE_ROW_HEIGHT_PX: f32 = 24.0;
const SIDEBAR_TREE_COMFORTABLE_ROW_HEIGHT_PX: f32 = 32.0;

/// Unscaled height shared by both sidebar modes. Round the density ramp before
/// scaling so natural rows and sticky overlays use the same slot geometry.
pub(in crate::view) fn sidebar_list_row_height_px(theme: AppTheme) -> f32 {
    theme
        .metrics
        .row_height(
            SIDEBAR_TREE_ROW_HEIGHT_PX,
            SIDEBAR_TREE_COMFORTABLE_ROW_HEIGHT_PX,
        )
        .round()
}

pub(in crate::view) fn sidebar_list_row_height(
    theme: AppTheme,
    ui_scale_percent: u32,
) -> gpui::Pixels {
    ui_scale::design_px_from_percent(sidebar_list_row_height_px(theme), ui_scale_percent)
}

const STASH_ICON_PATH: &str = crate::view::icons::STASH_ICON_PATH;

pub(in crate::view) fn listed_worktree_paths_by_branch(
    repo: &RepoState,
) -> FxHashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return FxHashMap::default();
    };

    let mut worktree_paths = FxHashMap::default();
    for worktree in worktrees.iter() {
        if worktree.path == repo.spec.workdir {
            continue;
        }

        let Some(branch) = worktree.branch.clone() else {
            continue;
        };

        worktree_paths
            .entry(branch)
            .or_insert_with(|| worktree.path.clone());
    }

    worktree_paths
}

fn branch_worktree_badge_path(
    listed_worktree_path: Option<&std::path::Path>,
    active_worktree_path: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    listed_worktree_path
        .map(std::path::Path::to_path_buf)
        .or_else(|| active_worktree_path.map(std::path::Path::to_path_buf))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::view) struct WorktreeBadgePalette {
    pub(in crate::view) bg: gpui::Rgba,
    active_bg: gpui::Rgba,
    pub(in crate::view) border: gpui::Rgba,
    pub(in crate::view) hover_border: gpui::Rgba,
    open_border: gpui::Rgba,
    open_hover_border: gpui::Rgba,
    active_border: gpui::Rgba,
    pub(in crate::view) icon: gpui::Rgba,
    open_icon: gpui::Rgba,
    active_icon: gpui::Rgba,
    pub(in crate::view) text: gpui::Rgba,
    pub(in crate::view) hover_text: gpui::Rgba,
    open_text: gpui::Rgba,
    active_text: gpui::Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorktreeBadgeColors {
    bg: gpui::Rgba,
    border: gpui::Rgba,
    hover_border: gpui::Rgba,
    icon: gpui::Rgba,
    text: gpui::Rgba,
}

/// Shared chip palette for the sidebar badges (workspace/worktree/upstream),
/// with a transparent body and a quiet border at rest. Badges that refer to an
/// open workspace use the main canvas surface on light themes and a lighter,
/// raised surface on dark themes. Their text stays high-contrast and the icon
/// retains the workspace state color.
pub(in crate::view) fn worktree_badge_palette(theme: AppTheme) -> WorktreeBadgePalette {
    let transparent = gpui::rgba(0x00000000);
    let text = theme.colors.foreground.emphasis;
    WorktreeBadgePalette {
        bg: transparent,
        active_bg: if theme.is_dark {
            theme.colors.surface.raised
        } else {
            theme.colors.surface.canvas
        },
        border: with_alpha(theme.colors.stroke.default, 0.90),
        hover_border: with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.55 } else { 0.40 },
        ),
        open_border: with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.56 } else { 0.34 },
        ),
        open_hover_border: with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.72 } else { 0.46 },
        ),
        active_border: with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.84 } else { 0.68 },
        ),
        icon: theme.colors.foreground.secondary,
        open_icon: theme.colors.accent.foreground,
        active_icon: theme.colors.accent.foreground,
        text,
        hover_text: text,
        open_text: text,
        active_text: text,
    }
}

fn worktree_badge_colors(
    palette: WorktreeBadgePalette,
    is_open: bool,
    menu_active: bool,
) -> WorktreeBadgeColors {
    WorktreeBadgeColors {
        bg: if is_open {
            palette.active_bg
        } else {
            palette.bg
        },
        border: if menu_active {
            palette.active_border
        } else if is_open {
            palette.open_border
        } else {
            palette.border
        },
        hover_border: if is_open {
            palette.open_hover_border
        } else {
            palette.hover_border
        },
        icon: if menu_active {
            palette.active_icon
        } else if is_open {
            palette.open_icon
        } else {
            palette.icon
        },
        text: if menu_active {
            palette.active_text
        } else if is_open {
            palette.open_text
        } else {
            palette.text
        },
    }
}

/// A configured upstream is the active remote relationship, so its status chip
/// uses the same colors as a worktree badge whose workspace is open.
fn upstream_badge_colors(palette: WorktreeBadgePalette) -> WorktreeBadgeColors {
    worktree_badge_colors(palette, true, false)
}

pub(super) fn worktree_branch_badge_label(
    branch: Option<&SharedString>,
    detached: bool,
    open_repo: Option<&RepoState>,
) -> Option<SharedString> {
    if let Some(open_repo) = open_repo {
        if open_repo.detached_head_commit.is_some() {
            return Some("(detached)".into());
        }

        match &open_repo.head_branch {
            Loadable::Ready(head_branch) if head_branch != "HEAD" => {
                return Some(SharedString::new(head_branch.as_str()));
            }
            Loadable::Ready(_) if detached => return Some("(detached)".into()),
            _ => {}
        }
    }

    branch
        .cloned()
        .or_else(|| detached.then(|| "(detached)".into()))
}

/// `"{branch} · {folder}"` — the branch says what is checked out, the folder says
/// which worktree it is checked out in, and only the pair is unambiguous when
/// several worktrees sit on related branches.
///
/// Falls back to the folder alone when there is no branch to name, and to the
/// branch alone when the path has no final component.
pub(in crate::view) fn worktree_origin_label(
    branch: Option<&str>,
    detached: bool,
    path: &std::path::Path,
) -> SharedString {
    let branch =
        worktree_branch_badge_label(branch.map(SharedString::new).as_ref(), detached, None);
    let folder = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    match (branch, folder) {
        (Some(branch), Some(folder)) => SharedString::new(format!("{branch} · {folder}")),
        (Some(branch), None) => branch,
        (None, Some(folder)) => SharedString::new(folder),
        (None, None) => "worktree".into(),
    }
}

/// Height of a worktree badge. Comfortable lifts it with the rows and title
/// bars it sits in.
const WORKTREE_BADGE_HEIGHT_PX: f32 = 18.0;
const WORKTREE_BADGE_COMFORTABLE_HEIGHT_PX: f32 = 24.0;

pub(in crate::view) fn worktree_badge_height(scale: impl Into<ui_scale::UiScale>) -> Pixels {
    scale.into().row_height(
        WORKTREE_BADGE_HEIGHT_PX,
        WORKTREE_BADGE_COMFORTABLE_HEIGHT_PX,
    )
}

/// Marks content belonging to another worktree in history, details and diff
/// headers. Worktree chips share their interaction profile; callers supply the
/// action.
pub(in crate::view) fn worktree_origin_chip(
    id: impl Into<gpui::ElementId>,
    theme: AppTheme,
    label: SharedString,
    icon_size: Pixels,
    height: Pixels,
    max_width: Pixels,
    pad_x: Pixels,
) -> gpui::Stateful<gpui::Div> {
    let palette = worktree_badge_palette(theme);
    div()
        .id(id)
        .control_interaction(
            worktree_badge_interaction(theme),
            controls::InteractionState::default(),
        )
        .flex()
        .items_center()
        .gap_1()
        .flex_shrink_0()
        .max_w(max_width)
        .px(pad_x)
        .h(height)
        .rounded(px(theme.radii.control))
        .border_1()
        .border_color(palette.border)
        .bg(palette.bg)
        .child(svg_icon(WORKTREE_ICON_PATH, palette.icon, icon_size))
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(palette.text)
                .line_clamp(1)
                .whitespace_nowrap()
                .child(label),
        )
}

pub(in crate::view) fn worktree_badge_interaction(theme: AppTheme) -> controls::InteractionStyle {
    let palette = worktree_badge_palette(theme);
    controls::InteractionStyle::accent(theme)
        .hover(
            gpui::StyleRefinement::default()
                .bg(theme.hover_overlay())
                .border_color(palette.hover_border)
                .text_color(palette.hover_text),
        )
        .pressed(
            gpui::StyleRefinement::default()
                .bg(theme.active_overlay())
                .border_color(palette.hover_border)
                .text_color(palette.hover_text),
        )
        .resting_background(palette.bg)
}

/// Render a sidebar label, bold-accent highlighting the first case-insensitive
/// occurrence of the branch filter `query` (already trimmed and lowercased).
/// With no query or no match the plain label is returned so the unfiltered
/// sidebar renders exactly as before.
///
/// `text_size`/`font_weight` must repeat what the surrounding row already sets:
/// TruncatedText resolves unset text styles inside a deferred measure closure
/// that doesn't see ancestor styling, so an unset size would fall back to the
/// 1rem window default and the label would grow as soon as it matched.
fn search_label_highlights(
    search: &crate::view::sidebar_search::SidebarSearch,
    label: &str,
    color: gpui::Rgba,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let mut ranges = Vec::new();
    search.matcher.find_ranges_into(label, &mut ranges, 16);
    ranges
        .into_iter()
        .map(|range| {
            (
                range,
                gpui::HighlightStyle {
                    color: Some(color.into_color()),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
            )
        })
        .collect()
}

fn filtered_label_element<V: 'static>(
    label: SharedString,
    source: Option<&str>,
    search: &crate::view::sidebar_search::SidebarSearch,
    text_color: gpui::Rgba,
    highlight_color: gpui::Rgba,
    text_size: gpui::AbsoluteLength,
    font_weight: FontWeight,
    cx: &gpui::Context<V>,
) -> AnyElement {
    let mut ranges = Vec::new();
    let source = source.unwrap_or(&label);
    search.matcher.find_ranges_into(source, &mut ranges, 16);
    if source.ends_with(label.as_ref()) {
        let offset = source.len() - label.len();
        ranges = ranges
            .into_iter()
            .filter_map(|range| {
                (range.end > offset).then(|| range.start.saturating_sub(offset)..range.end - offset)
            })
            .collect();
    }
    if !ranges.is_empty() {
        let highlight = gpui::HighlightStyle {
            color: Some(highlight_color.into_color()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        components::TruncatedText::new(label, text_size)
            .text_color(text_color)
            .font_weight(font_weight)
            .highlights(ranges.into_iter().map(|range| (range, highlight)))
            .render(cx)
            .into_any_element()
    } else {
        label.into_any_element()
    }
}

pub(in crate::view) fn active_worktree_paths_by_branch(
    repo: &RepoState,
    open_repos: &[RepoState],
) -> FxHashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return FxHashMap::default();
    };

    // Preserve the first open repository for a path, as the former linear
    // lookup did, while avoiding a worktrees × open-repositories scan.
    let mut open_by_path = FxHashMap::default();
    for open_repo in open_repos {
        open_by_path
            .entry(&open_repo.spec.workdir)
            .or_insert(open_repo);
    }
    let mut active_worktrees = FxHashMap::default();
    for worktree in worktrees.iter() {
        let Some(open_repo) = open_by_path.get(&worktree.path) else {
            continue;
        };

        let branch = if open_repo.detached_head_commit.is_some() {
            None
        } else {
            match &open_repo.head_branch {
                Loadable::Ready(head_branch) if head_branch != "HEAD" => Some(head_branch.clone()),
                Loadable::Ready(_) => None,
                _ => worktree.branch.clone(),
            }
        };
        let Some(branch) = branch else {
            continue;
        };

        active_worktrees
            .entry(branch)
            .or_insert_with(|| worktree.path.clone());
    }

    active_worktrees
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LocalBranchDoubleClickAction {
    CheckoutBranch { name: String },
    OpenWorktree { path: std::path::PathBuf },
}

fn local_branch_double_click_action(
    branch: &str,
    badge_worktree_path: Option<&std::path::Path>,
) -> LocalBranchDoubleClickAction {
    match badge_worktree_path {
        Some(path) => LocalBranchDoubleClickAction::OpenWorktree {
            path: path.to_path_buf(),
        },
        None => LocalBranchDoubleClickAction::CheckoutBranch {
            name: branch.to_string(),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BranchHistoryRevealTarget {
    commit_id: CommitId,
    fallback_scope: Option<LogScope>,
}

pub(in crate::view) fn branch_commit_id(
    repo: &RepoState,
    target: &BranchMenuTarget,
) -> Option<CommitId> {
    match target {
        BranchMenuTarget::Local { name } => match &repo.branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|branch| branch.name == *name)
                .map(|branch| branch.target.clone()),
            _ => None,
        },
        BranchMenuTarget::Remote { remote, branch } => match &repo.remote_branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|candidate| candidate.remote == *remote && candidate.name == *branch)
                .map(|candidate| candidate.target.clone()),
            _ => None,
        },
    }
}

fn branch_click_history_reveal_target(
    repo: &RepoState,
    target: &BranchMenuTarget,
    is_head: bool,
) -> Option<BranchHistoryRevealTarget> {
    let commit_id = branch_commit_id(repo, target)?;

    let fallback_scope = match target.section() {
        BranchSection::Local if is_head => Some(LogScope::FullReachable),
        BranchSection::Local | BranchSection::Remote => Some(LogScope::AllBranches),
    };

    Some(BranchHistoryRevealTarget {
        commit_id,
        fallback_scope,
    })
}

fn branch_row_is_selected(
    selected_branch: Option<&SelectedBranch>,
    repo_id: RepoId,
    target: &BranchMenuTarget,
    selected_commit: Option<&CommitId>,
    selected_branch_commit_id: Option<&CommitId>,
) -> bool {
    selected_branch.is_some_and(|selected_branch| {
        selected_branch.repo_id == repo_id
            && selected_branch.target == *target
            && selected_branch_commit_id.is_some_and(|commit_id| selected_commit == Some(commit_id))
    })
}

impl SidebarPaneView {
    pub(in super::super) fn render_branch_sidebar_rows(
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
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = ui_scale::scaler(ui_scale_percent);

        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        let is_collapsed_popover = surface == SidebarRowSurface::Rail;
        let header_activation = if is_collapsed_popover {
            controls::ControlActivation::Composite
        } else {
            controls::ControlActivation::Action
        };
        let filter_query = presentation.search.clone();
        let rows = presentation.rows.clone();
        let pin_count = presentation.pins.len();
        let worktree_badges = presentation.worktree_badges;
        let repo_workdir = this.active_repo().map(|r| r.spec.workdir.clone());
        let theme = this.theme;
        let worktree_badge_palette = worktree_badge_palette(theme);
        let icon_primary = theme.colors.foreground.secondary;
        let icon_current = theme.colors.accent.foreground;
        let icon_muted = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.70 } else { 0.78 },
        );
        let selected_branch_commit_id = this.sidebar_selected_tip();
        let selected_commit = this
            .active_repo()
            .and_then(|repo| repo.history_state.selected_commit.clone());

        let svg_icon = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_icon(path, color, scaled_px(size_px))
        };
        let svg_spinner = |id: (&'static str, u64), color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_spinner(id, color, scaled_px(size_px))
        };
        let svg_collapse = |collapsed: bool| {
            svg_icon(
                if collapsed {
                    "icons/arrow_right.svg"
                } else {
                    "icons/chevron_down.svg"
                },
                icon_muted,
                12.0,
            )
        };
        let tree_toggle_slot = |collapsed: Option<bool>| {
            div()
                .w(scaled_px(BRANCH_TREE_TOGGLE_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when_some(collapsed, |this, collapsed| {
                    this.child(svg_collapse(collapsed))
                })
        };
        let tree_icon_slot = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            div()
                .w(scaled_px(BRANCH_TREE_ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, color, size_px))
        };
        let branch_tree_color = |section: BranchSection| match section {
            BranchSection::Local => theme.colors.foreground.primary,
            BranchSection::Remote => theme.colors.foreground.secondary,
        };

        let content_inset = if is_collapsed_popover {
            0.0
        } else {
            components::ROW_HIGHLIGHT_INSET_PX
        };
        let indent_px = |depth: usize| {
            scaled_px(
                content_inset + BRANCH_TREE_BASE_PAD_PX + depth as f32 * BRANCH_TREE_DEPTH_STEP_PX,
            )
        };

        let paint_header = |row: AnyElement, background: gpui::Rgba| {
            if is_collapsed_popover {
                row
            } else {
                div().w_full().bg(background).child(row).into_any_element()
            }
        };
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
                let surface = if ix < pin_count && surface != SidebarRowSurface::Rail {
                    SidebarRowSurface::Pins
                } else {
                    surface
                };
                let row_surface = sidebar_row_background(theme, surface, &row, stuck);
                let row_style = components::InteractiveRowStyle::new(theme, row_surface).flat();
                (ix, row, row_style, row_surface, stuck)
            })
            .map(|(ix, row, row_style, row_surface, stuck)| match row {
                BranchSidebarRow::SectionHeader {
                    section,
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => {
                    let (icon_path, label): (&'static str, SharedString) = match section {
                        BranchSection::Local => ("icons/computer.svg", "Local Branches".into()),
                        BranchSection::Remote => ("icons/cloud.svg", "Remote Branches".into()),
                    };
                    let tooltip = label.clone();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString =
                        format!("branch_section_menu_{}_{}", repo_id.0, section_key).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(collapse_key.clone())
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .child(
                            tree_toggle_slot(is_collapsed_popover.then_some(collapsed))
                                .debug_selector(move || format!("sidebar_header_toggle_{ix}")),
                        )
                        .child(
                            tree_icon_slot(icon_path, icon_primary, 14.0)
                                .debug_selector(move || format!("sidebar_header_icon_{ix}")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(theme.ui_text(14.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.colors.foreground.primary)
                                .child(label),
                        )
                        .gitcomet_tooltip(theme, tooltip.clone())
                        .on_activate(
                            false,
                            header_activation,
                            cx.listener(move |this, e: &ClickEvent, _w, cx| {
                                if !e.standard_click() || e.click_count() != 1 {
                                    return;
                                }
                                if is_collapsed_popover {
                                    this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                                } else {
                                    this.navigate_sidebar_row(ix, cx);
                                }
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::BranchSectionMenu { repo_id, section })
                                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .map(|row| paint_header(row.into_any_element(), row_surface))
                        .into_any_element()
                }
                // A full slot: uniform_list sizes every row from item 0.
                BranchSidebarRow::SectionSpacer => div()
                    .id(("branch_section_spacer", ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .w_full()
                    .into_any_element(),
                BranchSidebarRow::StashHeader {
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => {
                    let show_stash_spinner = this.active_repo().is_some_and(|r| {
                        matches!(r.stashes, Loadable::Loading)
                            || (!collapsed && matches!(r.stashes, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("stash_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(collapse_key.clone())
                        .debug_selector(move || format!("stash_section_{ix}"))
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(is_collapsed_popover.then_some(collapsed)))
                        .child(tree_icon_slot(STASH_ICON_PATH, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(theme.ui_text(14.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.colors.foreground.primary)
                                .child("Stashes"),
                        )
                        .when(show_stash_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || format!("stash_spinner_{}", repo_id.0))
                                    .child(svg_spinner(
                                        ("stash_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .gitcomet_tooltip(theme, "Stashes (Right-click for actions)".into())
                        .on_activate(
                            false,
                            header_activation,
                            cx.listener(move |this, e: &ClickEvent, _w, cx| {
                                if !e.standard_click() || e.click_count() != 1 {
                                    return;
                                }
                                if is_collapsed_popover {
                                    this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                                } else {
                                    this.navigate_sidebar_row(ix, cx);
                                }
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    PopoverKind::StashPrompt
                                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .map(|row| paint_header(row.into_any_element(), row_surface))
                        .into_any_element()
                }
                BranchSidebarRow::StashPlaceholder { message } => div()
                    .id(("stash_placeholder", ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .w_full()
                    .px_2()
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::StashItem {
                    index,
                    message,
                    tooltip,
                    ..
                } => {
                    let tooltip = tooltip.clone();
                    let stash_message_for_menu = message.as_ref().to_owned();
                    let context_menu_invoker: SharedString =
                        format!("stash_menu_{}_{}", repo_id.0, index).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let stash_message_for_right_click = stash_message_for_menu.clone();
                    let row_group: SharedString =
                        format!("stash_row_{}_{}", repo_id.0, index).into();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(("stash_sidebar_row", index))
                        .debug_selector(move || format!("stash_sidebar_row_{index}"))
                        .relative()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(STASH_ICON_PATH, icon_primary, 14.0))
                        .child(
                            components::FadingText::new(
                                div()
                                    .text_size(theme.ui_text(14.0))
                                    .child(filtered_label_element(
                                        message.clone(),
                                        None,
                                        &filter_query,
                                        theme.colors.foreground.primary,
                                        theme.colors.accent.foreground,
                                        theme.ui_text(14.0).into(),
                                        FontWeight::NORMAL,
                                        cx,
                                    )),
                                row_style.resolved_background(row_state),
                            )
                            .hover_bg(
                                row_group.clone(),
                                row_style.resolved_hover_background(row_state),
                            )
                            .render(ui_scale_percent)
                            .flex_1(),
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Composite,
                            cx.listener(move |this, e: &ClickEvent, _w, cx| {
                                if !e.standard_click() || e.click_count() < 2 {
                                    return;
                                }
                                this.store.dispatch(Msg::ApplyStash { repo_id, index });
                                cx.notify();
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::StashMenu {
                                        repo_id,
                                        index,
                                        message: stash_message_for_right_click.clone(),
                                    })
                                    .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .gitcomet_tooltip(theme, tooltip.clone())
                        .into_any_element()
                }
                BranchSidebarRow::Placeholder {
                    section: _,
                    message,
                } => div()
                    .id(("branch_placeholder", ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .w_full()
                    .px_2()
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::WorktreesHeader {
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => {
                    let show_worktrees_spinner = this.active_repo().is_some_and(|r| {
                        r.worktrees_in_flight > 0
                            || matches!(r.worktrees, Loadable::Loading)
                            || (!collapsed && matches!(r.worktrees, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("worktrees_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(collapse_key.clone())
                        .debug_selector(move || format!("worktrees_section_{ix}"))
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(is_collapsed_popover.then_some(collapsed)))
                        .child(tree_icon_slot(WORKTREE_ICON_PATH, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(theme.ui_text(14.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.colors.foreground.primary)
                                .child("Worktrees"),
                        )
                        .when(show_worktrees_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || {
                                        format!("worktrees_spinner_{}", repo_id.0)
                                    })
                                    .child(svg_spinner(
                                        ("worktrees_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .gitcomet_tooltip(theme, "Worktrees (Add / Refresh / Open / Remove)".into())
                        .on_activate(
                            false,
                            header_activation,
                            cx.listener(move |this, e: &ClickEvent, _w, cx| {
                                if !e.standard_click() || e.click_count() != 1 {
                                    return;
                                }
                                if is_collapsed_popover {
                                    this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                                } else {
                                    this.navigate_sidebar_row(ix, cx);
                                }
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::SectionMenu,
                                    ))
                                    .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .map(|row| paint_header(row.into_any_element(), row_surface))
                        .into_any_element()
                }
                BranchSidebarRow::WorktreePlaceholder { message } => div()
                    .id(("worktree_placeholder", ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .w_full()
                    .px_2()
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::WorktreeItem {
                    path,
                    branch,
                    detached,
                    is_active,
                } => {
                    let branch = branch.clone();
                    let path_for_open = path.clone();
                    let path_for_menu = path.clone();
                    let branch_for_menu = branch.as_ref().map(|name| name.to_string());
                    let path_label = this.cached_path_display(&path);
                    let context_menu_invoker: SharedString =
                        format!("worktree_menu_{}_{}", repo_id.0, path.display()).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let open_worktree_repo = this.open_repo_for_workdir(&path);
                    let worktree_tab_open = open_worktree_repo.is_some();
                    let branch_badge_label =
                        worktree_branch_badge_label(branch.as_ref(), detached, open_worktree_repo);
                    let branch_badge_colors = worktree_badge_colors(
                        worktree_badge_palette,
                        worktree_tab_open,
                        context_menu_active,
                    );
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("worktree_row_{}_{}", repo_id.0, ix).into();
                    let row_debug_selector = row_group.as_ref().to_owned();
                    let active_background = with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.18 } else { 0.12 },
                    );
                    let row_state = components::InteractiveRowState::default()
                        .selected(is_active, active_background)
                        .open(context_menu_active);

                    div()
                        .id(("worktree_item", ix))
                        .debug_selector(move || row_debug_selector.clone())
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(WORKTREE_ICON_PATH, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(theme.ui_text(14.0))
                                .flex()
                                .items_center()
                                .overflow_hidden()
                                .gap(scaled_px(5.0))
                                .child(
                                    div()
                                        .debug_selector(move || format!("worktree_path_label_{ix}"))
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .child(
                                            components::TruncatedText::path(
                                                path_label.clone(),
                                                theme.ui_text(14.0),
                                            )
                                            .id(("worktree_path_text", ix))
                                            .highlights(search_label_highlights(
                                                &filter_query,
                                                &path_label,
                                                theme.colors.accent.foreground,
                                            ))
                                            // Set the color explicitly: TruncatedText
                                            // resolves an unset color from the ambient text
                                            // style inside a deferred measure closure, which
                                            // doesn't see ancestor `text_color` — so in the
                                            // collapsed popover it would render near-black.
                                            .text_color(theme.colors.foreground.primary)
                                            .full_text_tooltip(this.tooltip_host.clone())
                                            .render(cx),
                                        ),
                                )
                                .when_some(branch_badge_label.clone(), |row, badge_label| {
                                    row.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(scaled_px(3.0))
                                            .px(scaled_px(6.0))
                                            .h(worktree_badge_height(
                                                ui_scale::UiScale::from_percent(ui_scale_percent)
                                                    .with_appearance(theme.metrics),
                                            ))
                                            // Same control radius as the branch
                                            // rows' worktree badge; the two are
                                            // the same chip in two lists.
                                            .rounded(px(theme.radii.control))
                                            .border_1()
                                            .border_color(branch_badge_colors.border)
                                            .bg(branch_badge_colors.bg)
                                            .text_size(theme.ui_text(11.0))
                                            .text_color(branch_badge_colors.text)
                                            .id(("worktree_branch_badge", ix))
                                            .debug_selector(move || {
                                                format!("worktree_branch_badge_{ix}")
                                            })
                                            .max_w_1_2()
                                            .min_w(px(0.0))
                                            .overflow_hidden()
                                            .child(svg_icon(
                                                "icons/git_branch.svg",
                                                branch_badge_colors.icon,
                                                9.0,
                                            ))
                                            .child(
                                                div()
                                                    .debug_selector(move || {
                                                        format!("worktree_branch_badge_label_{ix}")
                                                    })
                                                    .min_w(px(0.0))
                                                    .overflow_hidden()
                                                    .child(
                                                        components::TruncatedText::new(
                                                            badge_label.clone(),
                                                            theme.ui_text(11.0),
                                                        )
                                                        .id(("worktree_branch_badge_text", ix))
                                                        .highlights(search_label_highlights(
                                                            &filter_query,
                                                            &badge_label,
                                                            theme.colors.accent.foreground,
                                                        ))
                                                        // Explicit color: TruncatedText resolves an
                                                        // unset color from the ambient text style in
                                                        // a deferred measure closure that misses the
                                                        // pill's `.text_color`, rendering near-black
                                                        // in the collapsed popover.
                                                        .text_color(branch_badge_colors.text)
                                                        .full_text_tooltip(
                                                            this.tooltip_host.clone(),
                                                        )
                                                        .render(cx),
                                                    ),
                                            ),
                                    )
                                }),
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Composite,
                            cx.listener(move |this, e: &ClickEvent, window, cx| {
                                if !e.standard_click() {
                                    return;
                                }
                                if e.click_count() >= 2 {
                                    crate::app::open_repository_from_view(
                                        cx,
                                        window.window_handle().window_id(),
                                        path_for_open.clone(),
                                    );
                                    cx.notify();
                                    return;
                                }
                                // Single click mirrors a branch row: scroll the log
                                // to this worktree and select its row, without
                                // leaving the tab.
                                this.reveal_worktree_in_history(
                                    repo_id,
                                    path_for_open.clone(),
                                    is_active,
                                    cx,
                                );
                                cx.notify();
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                            branch: branch_for_menu.clone(),
                                        },
                                    ))
                                    .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulesHeader {
                    top_border: _,
                    collapsed,
                    collapse_key,
                } => {
                    let show_submodules_spinner = this
                        .active_repo()
                        .is_some_and(|r| matches!(r.submodules, Loadable::Loading));
                    let context_menu_invoker: SharedString =
                        format!("submodules_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(collapse_key.clone())
                        .debug_selector(move || format!("submodules_section_{ix}"))
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(is_collapsed_popover.then_some(collapsed)))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(theme.ui_text(14.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.colors.foreground.primary)
                                .child("Submodules"),
                        )
                        .when(show_submodules_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || {
                                        format!("submodules_spinner_{}", repo_id.0)
                                    })
                                    .child(svg_spinner(
                                        ("submodules_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .gitcomet_tooltip(theme, "Submodules (Add / Update / Open / Remove)".into())
                        .on_activate(
                            false,
                            header_activation,
                            cx.listener(move |this, e: &ClickEvent, _w, cx| {
                                if !e.standard_click() || e.click_count() != 1 {
                                    return;
                                }
                                if is_collapsed_popover {
                                    this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                                } else {
                                    this.navigate_sidebar_row(ix, cx);
                                }
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::SectionMenu,
                                    ))
                                    .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .map(|row| paint_header(row.into_any_element(), row_surface))
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulePlaceholder { message, can_load } => div()
                    .id(("submodule_placeholder", ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .w_full()
                    .pl_2()
                    .pr_1()
                    .flex()
                    .items_center()
                    .gap(scaled_px(6.0))
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(message),
                    )
                    .when(can_load, |row| {
                        row.child(
                            components::Button::new(
                                format!("submodule_placeholder_load_{}", repo_id.0),
                                "Load",
                            )
                            .borderless()
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _window, _cx| {
                                    this.store.dispatch(Msg::LoadSubmodules { repo_id });
                                },
                            ),
                        )
                    })
                    .into_any_element(),
                BranchSidebarRow::SubmoduleItem {
                    path,
                    status,
                    recorded_head,
                    checked_out_head,
                } => {
                    let path_for_open = path.clone();
                    let path_for_menu = path.clone();
                    let repo_workdir_for_open = repo_workdir.clone();
                    let path_label = this.cached_path_display(&path);
                    let (icon_color, badge_label, can_open, tooltip) = {
                        let badge_label = match status {
                            SubmoduleStatus::NotInitialized => Some("Not loaded"),
                            SubmoduleStatus::HeadMismatch => Some("Head mismatch"),
                            SubmoduleStatus::MergeConflict => Some("Conflict"),
                            SubmoduleStatus::MissingMapping => Some("Missing mapping"),
                            SubmoduleStatus::Unknown(_) => Some("Unknown"),
                            SubmoduleStatus::UpToDate => None,
                        };
                        let icon_color = match status {
                            SubmoduleStatus::NotInitialized => with_alpha(
                                theme.colors.foreground.secondary,
                                if theme.is_dark { 0.78 } else { 0.92 },
                            ),
                            SubmoduleStatus::HeadMismatch => theme.colors.status.warning.foreground,
                            SubmoduleStatus::MergeConflict | SubmoduleStatus::MissingMapping => {
                                theme.colors.status.danger.foreground
                            }
                            SubmoduleStatus::UpToDate | SubmoduleStatus::Unknown(_) => icon_primary,
                        };
                        let can_open = !matches!(
                            status,
                            SubmoduleStatus::NotInitialized
                                | SubmoduleStatus::MergeConflict
                                | SubmoduleStatus::MissingMapping
                        );
                        let checked_out = checked_out_head
                            .as_ref()
                            .map(|head| head.as_ref())
                            .unwrap_or("not loaded");
                        let tooltip: SharedString = format!(
                            "{}\nRecorded: {}\nChecked out: {}",
                            path.display(),
                            recorded_head.as_ref(),
                            checked_out,
                        )
                        .into();
                        (icon_color, badge_label, can_open, tooltip)
                    };
                    let context_menu_invoker: SharedString =
                        format!("submodule_menu_{}_{}", repo_id.0, path.display()).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(("submodule_item", ix))
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot("icons/box.svg", icon_color, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(theme.ui_text(14.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .debug_selector(move || format!("submodule_label_{ix}"))
                                .child(filtered_label_element(
                                    path_label,
                                    None,
                                    &filter_query,
                                    theme.colors.foreground.primary,
                                    theme.colors.accent.foreground,
                                    theme.ui_text(14.0).into(),
                                    FontWeight::NORMAL,
                                    cx,
                                )),
                        )
                        .when_some(badge_label, |row, badge_label| {
                            row.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(scaled_px(3.0))
                                    .px(scaled_px(4.0))
                                    .py(scaled_px(0.0))
                                    .rounded(px(theme.radii.pill))
                                    .border_1()
                                    .border_color(if context_menu_active {
                                        theme.colors.stroke.default
                                    } else {
                                        with_alpha(
                                            theme.colors.foreground.secondary,
                                            if theme.is_dark { 0.32 } else { 0.24 },
                                        )
                                    })
                                    .bg(with_alpha(
                                        theme.colors.surface.panel,
                                        if theme.is_dark { 0.9 } else { 0.7 },
                                    ))
                                    .text_size(theme.ui_text(11.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(badge_label),
                            )
                        })
                        .on_activate(
                            false,
                            controls::ControlActivation::Composite,
                            cx.listener(move |_this, e: &ClickEvent, window, cx| {
                                if !e.standard_click() || e.click_count() < 2 {
                                    return;
                                }
                                if !can_open {
                                    return;
                                }
                                let Some(base) = repo_workdir_for_open.clone() else {
                                    return;
                                };
                                crate::app::open_repository_from_view(
                                    cx,
                                    window.window_handle().window_id(),
                                    base.join(&path_for_open),
                                );
                                cx.notify();
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                        },
                                    ))
                                    .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .gitcomet_tooltip(theme, tooltip.clone())
                        .into_any_element()
                }
                BranchSidebarRow::RemoteHeader {
                    name,
                    collapsed,
                    collapse_key,
                } => {
                    let remote_color = branch_tree_color(BranchSection::Remote);
                    let remote_name: String = name.as_ref().to_owned();
                    let context_menu_invoker: SharedString =
                        format!("remote_menu_{}_{}", repo_id.0, remote_name).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let remote_name_for_right_click: String = name.as_ref().to_owned();
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("remote_header_row_{}_{}", repo_id.0, remote_name).into();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(collapse_key.clone())
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .text_size(theme.ui_text(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(remote_color)
                        .child(if is_collapsed_popover {
                            tree_toggle_slot(Some(collapsed)).into_any_element()
                        } else {
                            let key = collapse_key.clone();
                            tree_toggle_slot(Some(collapsed))
                                .id(("sidebar_group_toggle", ix))
                                .debug_selector(move || format!("sidebar_group_toggle_{ix}"))
                                .h_full()
                                .on_activate(
                                    false,
                                    controls::ControlActivation::Nested,
                                    cx.listener(move |this, _, _, cx| {
                                        if stuck {
                                            this.navigate_sidebar_row(ix, cx);
                                        } else {
                                            this.toggle_active_repo_collapse_key(key.clone(), cx);
                                        }
                                    }),
                                )
                                .into_any_element()
                        })
                        .child(tree_icon_slot(
                            super::super::file_icons::folder_icon(!collapsed),
                            remote_color,
                            14.0,
                        ))
                        .child(
                            components::FadingText::new(
                                div().child(name),
                                row_style.resolved_background(row_state),
                            )
                            .hover_bg(
                                row_group.clone(),
                                row_style.resolved_hover_background(row_state),
                            )
                            .render(ui_scale_percent)
                            .flex_1(),
                        )
                        .on_activate(
                            false,
                            header_activation,
                            cx.listener(move |this, e: &ClickEvent, _w, cx| {
                                if !e.standard_click() || e.click_count() != 1 {
                                    return;
                                }
                                if stuck {
                                    this.navigate_sidebar_row(ix, cx);
                                } else {
                                    this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                                }
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::remote(
                                        repo_id,
                                        RemotePopoverKind::Menu {
                                            name: remote_name_for_right_click.clone(),
                                        },
                                    ))
                                    .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .map(|row| paint_header(row.into_any_element(), row_surface))
                        .into_any_element()
                }
                BranchSidebarRow::GroupHeader {
                    label,
                    path,
                    remote,
                    section,
                    depth,
                    collapsed,
                    collapse_key,
                } => {
                    let from_pins = ix < pin_count;
                    let pinned_root = from_pins && depth == 0;
                    let prefix = if from_pins { "pinned_" } else { "" };
                    let row_group: SharedString =
                        format!("{prefix}branch_group_row_{}_{}", repo_id.0, ix).into();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString = format!(
                        "{prefix}branch_group_menu_{}_{}_{}_{}",
                        repo_id.0,
                        section_key,
                        remote.as_deref().unwrap_or_default(),
                        path
                    )
                    .into();
                    let context_menu_invoker: SharedString = if from_pins {
                        format!("{context_menu_invoker}_{ix}").into()
                    } else {
                        context_menu_invoker
                    };
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let menu_kind = PopoverKind::BranchGroupMenu {
                        repo_id,
                        section,
                        remote: remote.as_ref().map(|remote| remote.to_string()),
                        path: path.to_string(),
                    };
                    let menu_kind_for_right_click = menu_kind.clone();
                    let row_state =
                        components::InteractiveRowState::default().open(context_menu_active);

                    div()
                        .id(("branch_group", ix))
                        .debug_selector(move || format!("{prefix}branch_group_{ix}"))
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .pl(indent_px(usize::from(depth)))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .text_size(theme.ui_text(12.0))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(if from_pins {
                            theme.colors.foreground.primary
                        } else {
                            theme.colors.foreground.secondary
                        })
                        .child(if is_collapsed_popover {
                            tree_toggle_slot(Some(collapsed)).into_any_element()
                        } else {
                            let key = collapse_key.clone();
                            tree_toggle_slot(Some(collapsed))
                                .id(("sidebar_group_toggle", ix))
                                .debug_selector(move || {
                                    format!("{prefix}sidebar_group_toggle_{ix}")
                                })
                                .h_full()
                                .on_activate(
                                    false,
                                    controls::ControlActivation::Nested,
                                    cx.listener(move |this, _, _, cx| {
                                        if stuck {
                                            this.navigate_sidebar_row(ix, cx);
                                        } else {
                                            this.toggle_active_repo_collapse_key(key.clone(), cx);
                                        }
                                    }),
                                )
                                .into_any_element()
                        })
                        .child(
                            tree_icon_slot(
                                if pinned_root {
                                    "icons/pin.svg"
                                } else {
                                    super::super::file_icons::folder_icon(!collapsed)
                                },
                                icon_primary,
                                14.0,
                            )
                            .when(pinned_root, |icon| {
                                icon.debug_selector(move || {
                                    format!("sidebar_group_pin_marker_{ix}")
                                })
                            }),
                        )
                        .child(
                            components::FadingText::new(
                                filtered_label_element(
                                    label,
                                    Some(&remote.as_ref().map_or_else(
                                        || format!("{path}/"),
                                        |remote| format!("{remote}/{path}/"),
                                    )),
                                    &filter_query,
                                    if from_pins {
                                        theme.colors.foreground.primary
                                    } else {
                                        theme.colors.foreground.secondary
                                    },
                                    theme.colors.accent.foreground,
                                    gpui::rems(0.75).into(),
                                    FontWeight::NORMAL,
                                    cx,
                                ),
                                row_style.resolved_background(row_state),
                            )
                            .hover_bg(
                                row_group.clone(),
                                row_style.resolved_hover_background(row_state),
                            )
                            .render(ui_scale_percent)
                            .flex_1(),
                        )
                        .on_activate(
                            false,
                            header_activation,
                            cx.listener(move |this, e: &ClickEvent, _w, cx| {
                                if !e.standard_click() || e.click_count() != 1 {
                                    return;
                                }
                                if stuck {
                                    this.navigate_sidebar_row(ix, cx);
                                } else {
                                    this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                                }
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    menu_kind_for_right_click
                                        .clone()
                                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .map(|row| paint_header(row.into_any_element(), row_surface))
                        .into_any_element()
                }
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
                } => {
                    let full_name_for_checkout: SharedString = name.clone();
                    let surface = if ix < pin_count {
                        SidebarRowSurface::Pins
                    } else {
                        surface
                    };
                    let pin_key = (ix < pin_count).then(|| presentation.row_keys[ix].clone());
                    let selected_branch = this.selected_branch_for_row(pin_key.as_ref()).cloned();
                    let full_name_for_menu: SharedString = name.clone();
                    let full_name_for_tooltip: SharedString = name.clone();
                    let target_for_reveal = target.clone();
                    let target_for_checkout = target.clone();
                    let target_for_menu = target.clone();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let menu_prefix = if surface == SidebarRowSurface::Pins {
                        "pinned_"
                    } else {
                        ""
                    };
                    let context_menu_invoker: SharedString = format!(
                        "{menu_prefix}branch_menu_{}_{}_{}",
                        repo_id.0,
                        section_key,
                        full_name_for_menu.as_ref()
                    )
                    .into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let full_label = (surface == SidebarRowSurface::Pins && depth == 0)
                        || matches!(surface, SidebarRowSurface::Sticky { compact: true });
                    let label: SharedString = if full_label {
                        name.clone()
                    } else {
                        super::super::branch_sidebar::branch_sidebar_branch_label(name.as_ref())
                            .to_owned()
                            .into()
                    };
                    let badge_worktree_path = (section == BranchSection::Local)
                        .then(|| worktree_badges.listed_path(name.as_ref()).cloned())
                        .flatten();
                    let active_worktree_path = (section == BranchSection::Local)
                        .then(|| worktree_badges.active_path(name.as_ref()).cloned())
                        .flatten();
                    let worktree_badge_path = branch_worktree_badge_path(
                        badge_worktree_path.as_deref(),
                        active_worktree_path.as_deref(),
                    );
                    let branch_selected = branch_row_is_selected(
                        selected_branch.as_ref(),
                        repo_id,
                        &target,
                        selected_commit.as_ref(),
                        selected_branch_commit_id.as_ref(),
                    );
                    let has_worktree = worktree_badge_path.is_some();
                    let has_active_worktree = active_worktree_path.is_some();
                    let show_worktree_badge = has_worktree;
                    let worktree_row_menu_invoker: Option<SharedString> =
                        worktree_badge_path.as_ref().map(|path| {
                            format!(
                                "{menu_prefix}worktree_menu_{}_{}",
                                repo_id.0,
                                path.display()
                            )
                            .into()
                        });
                    let worktree_menu_active =
                        worktree_row_menu_invoker.as_ref().is_some_and(|invoker| {
                            this.active_context_menu_invoker.as_ref() == Some(invoker)
                        });
                    let row_group: SharedString = if surface == SidebarRowSurface::Pins {
                        format!("pinned_branch_row_{}_{}", repo_id.0, ix).into()
                    } else {
                        format!("branch_row_{}_{}", repo_id.0, ix).into()
                    };
                    let row_debug_selector = row_group.as_ref().to_owned();
                    let branch_text_color = if surface == SidebarRowSurface::Pins {
                        theme.colors.foreground.primary
                    } else if muted {
                        theme.colors.foreground.secondary
                    } else {
                        branch_tree_color(section)
                    };
                    let branch_selected_bg = selected_branch_row_bg(theme);
                    let branch_selected_label_color = if branch_selected {
                        selected_branch_label_color(theme)
                    } else {
                        branch_text_color
                    };
                    let branch_icon_color = if is_head {
                        icon_current
                    } else if muted {
                        icon_muted
                    } else {
                        icon_primary
                    };
                    let badge_gap_px = scaled_px(BRANCH_BADGE_GAP_PX);
                    let divergence_badge =
                        |icon_path: &'static str,
                         color: gpui::Rgba,
                         count: NonZeroU32,
                         debug_selector: Option<String>| {
                            let mut badge = div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(theme.ui_text(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color)
                                .child(svg_icon(icon_path, color, 11.0))
                                .child(
                                    super::super::branch_sidebar::branch_sidebar_divergence_label(
                                        count,
                                    ),
                                );
                            if let Some(debug_selector) = debug_selector {
                                badge = badge.debug_selector(move || debug_selector.clone());
                            }
                            badge
                        };
                    let upstream_badge = |debug_selector: Option<String>| {
                        // The active upstream is a ref relationship, but it
                        // uses the same compact control-shaped chip as the
                        // worktree badge so the sidebar's status badges read as
                        // one family.
                        let colors = upstream_badge_colors(worktree_badge_palette);
                        let mut badge = div()
                            .flex()
                            .items_center()
                            .gap(scaled_px(3.0))
                            .px(scaled_px(6.0))
                            .rounded(px(theme.radii.control))
                            .text_size(theme.ui_text(11.0))
                            .text_color(colors.text)
                            .bg(colors.bg)
                            .border_1()
                            .border_color(colors.border)
                            .child(svg_icon("icons/cloud.svg", colors.icon, 9.0))
                            .child("Upstream");
                        if let Some(debug_selector) = debug_selector {
                            badge = badge.debug_selector(move || debug_selector.clone());
                        }
                        badge
                    };
                    let head_highlight = with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.18 } else { 0.12 },
                    );
                    let row_state = components::InteractiveRowState::default()
                        .selected(
                            is_head || branch_selected,
                            if branch_selected {
                                branch_selected_bg
                            } else {
                                head_highlight
                            },
                        )
                        .open(context_menu_active);

                    let mut row = div()
                        .id(("branch_item", ix))
                        .debug_selector(move || row_debug_selector.clone())
                        .relative()
                        .h(sidebar_list_row_height(theme, ui_scale_percent))
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(if full_label { 0 } else { usize::from(depth) }))
                        .pr(scaled_px(content_inset + BRANCH_ROW_TRAILING_PAD_PX))
                        .bg(row_surface)
                        .interactive_row(row_style, row_state)
                        .text_color(branch_text_color)
                        .child(tree_toggle_slot(None).when(
                            surface == SidebarRowSurface::Pins && depth == 0,
                            |slot| {
                                slot.debug_selector(move || format!("sidebar_pin_marker_{ix}"))
                                    .child(svg_icon("icons/pin.svg", icon_primary, 12.0))
                            },
                        ))
                        .child(
                            tree_icon_slot("icons/git_branch.svg", branch_icon_color, 14.0)
                                .debug_selector(move || {
                                    format!("sidebar_branch_icon_{surface:?}_{ix}")
                                }),
                        )
                        .child(
                            // Long branch names run into the trailing badges;
                            // fade them into the row instead of slicing a glyph.
                            components::FadingText::new(
                                div()
                                    .text_size(theme.ui_text(14.0))
                                    .text_color(branch_selected_label_color)
                                    .child(filtered_label_element(
                                        label,
                                        Some(&name),
                                        &filter_query,
                                        branch_selected_label_color,
                                        theme.colors.accent.foreground,
                                        gpui::rems(0.875).into(),
                                        FontWeight::NORMAL,
                                        cx,
                                    )),
                                row_style.resolved_background(row_state),
                            )
                            .hover_bg(
                                row_group.clone(),
                                row_style.resolved_hover_background(row_state),
                            )
                            .render(ui_scale_percent)
                            .flex_1(),
                        );

                    let show_branch_badges = divergence_behind.is_some()
                        || divergence_ahead.is_some()
                        || (is_upstream && section == BranchSection::Remote)
                        || show_worktree_badge;
                    let mut end_accessories = div()
                        .ml_auto()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(badge_gap_px);

                    if divergence_behind.is_some() || divergence_ahead.is_some() {
                        if let Some(behind) = divergence_behind {
                            let color = theme.colors.status.warning.foreground;
                            end_accessories = end_accessories.child(divergence_badge(
                                "icons/arrow_down.svg",
                                color,
                                behind,
                                Some(format!("branch_pull_badge_{ix}")),
                            ));
                        }
                        if let Some(ahead) = divergence_ahead {
                            let color = theme.colors.status.success.foreground;
                            end_accessories = end_accessories.child(divergence_badge(
                                "icons/arrow_up.svg",
                                color,
                                ahead,
                                Some(format!("branch_push_badge_{ix}")),
                            ));
                        }
                    }

                    if is_upstream && section == BranchSection::Remote {
                        end_accessories = end_accessories
                            .child(upstream_badge(Some(format!("branch_upstream_badge_{ix}"))));
                    }

                    if show_worktree_badge {
                        let Some(worktree_badge_path) = worktree_badge_path.clone() else {
                            unreachable!("workspace badge requires a worktree path");
                        };
                        let worktree_menu_invoker_for_click = worktree_row_menu_invoker.clone();
                        let worktree_menu_invoker_for_right_click =
                            worktree_row_menu_invoker.clone();
                        let worktree_path_for_menu = worktree_badge_path.clone();
                        let worktree_path_for_open = worktree_badge_path.clone();
                        let worktree_path_for_right_click = worktree_badge_path.clone();
                        let worktree_badge_label =
                            super::super::path_display::repo_path_name(&worktree_badge_path);
                        let worktree_badge_tooltip: SharedString =
                            worktree_badge_path.display().to_string().into();
                        let branch_name_for_click = name.to_string();
                        let branch_name_for_right_click = branch_name_for_click.clone();
                        let badge_colors = worktree_badge_colors(
                            worktree_badge_palette,
                            has_active_worktree,
                            worktree_menu_active,
                        );
                        let worktree_badge = div()
                            .id(("branch_worktree_badge", ix))
                            .debug_selector(move || format!("branch_worktree_badge_{ix}"))
                            .flex()
                            .items_center()
                            .gap(scaled_px(3.0))
                            .px(scaled_px(6.0))
                            .h(worktree_badge_height(
                                ui_scale::UiScale::from_percent(ui_scale_percent)
                                    .with_appearance(theme.metrics),
                            ))
                            // Squared off on the control radius the buttons and
                            // tabs use, matching the upstream status chip rather
                            // than the fully-round decorative pills.
                            .rounded(px(theme.radii.control))
                            .border_1()
                            .border_color(badge_colors.border)
                            .bg(badge_colors.bg)
                            .text_size(theme.ui_text(11.0))
                            .text_color(badge_colors.text)
                            .cursor(CursorStyle::PointingHand)
                            // A worktree folder can outrun the pane. Cap and
                            // truncate the pill rather than let it push the
                            // row's other badges off the trailing edge. An
                            // absolute cap, not a percentage: the pill's
                            // containing block is the accessory run, which is
                            // itself sized by this pill.
                            .max_w(scaled_px(BRANCH_WORKTREE_BADGE_MAX_W_PX))
                            .overflow_hidden()
                            .child(svg_icon(WORKTREE_ICON_PATH, badge_colors.icon, 9.0))
                            .child(
                                div().min_w(px(0.0)).overflow_hidden().child(
                                    components::TruncatedText::new(
                                        worktree_badge_label,
                                        theme.ui_text(11.0),
                                    )
                                    .id(("branch_worktree_badge_text", ix))
                                    // Explicit color: TruncatedText resolves an
                                    // unset one from the ambient text style in a
                                    // deferred measure closure that never sees the
                                    // pill's `.text_color`.
                                    .text_color(badge_colors.text)
                                    .render(cx),
                                ),
                            )
                            .control_interaction(
                                worktree_badge_interaction(theme),
                                controls::InteractionState::default()
                                    .selected(has_active_worktree, worktree_badge_palette.active_bg)
                                    .open(worktree_menu_active),
                            )
                            .on_activate(
                                false,
                                controls::ControlActivation::Nested,
                                cx.listener(move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    if e.click_count() >= 2 {
                                        crate::app::open_repository_from_view(
                                            cx,
                                            window.window_handle().window_id(),
                                            worktree_path_for_open.clone(),
                                        );
                                        cx.notify();
                                        return;
                                    }
                                    let Some(invoker) = worktree_menu_invoker_for_click.clone()
                                    else {
                                        return;
                                    };

                                    this.open_popover_at(
                                        (PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::Menu {
                                                path: worktree_path_for_menu.clone(),
                                                branch: Some(branch_name_for_click.clone()),
                                            },
                                        ))
                                        .invoked_by(invoker),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .on_pointer_click(
                                MouseButton::Right,
                                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    let Some(invoker) =
                                        worktree_menu_invoker_for_right_click.clone()
                                    else {
                                        return;
                                    };

                                    this.open_popover_at(
                                        (PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::Menu {
                                                path: worktree_path_for_right_click.clone(),
                                                branch: Some(branch_name_for_right_click.clone()),
                                            },
                                        ))
                                        .invoked_by(invoker),
                                        e.position,
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .gitcomet_tooltip(theme, worktree_badge_tooltip.clone());
                        end_accessories = end_accessories.child(worktree_badge);
                    }

                    if show_branch_badges {
                        row = row.child(end_accessories);
                    }

                    row = row
                        .on_activate(
                            false,
                            controls::ControlActivation::Composite,
                            cx.listener(move |this, e: &ClickEvent, window, cx| {
                                if !e.standard_click() {
                                    return;
                                }
                                if e.click_count() == 1 {
                                    let Some(target) = this.active_repo().and_then(|repo| {
                                        branch_click_history_reveal_target(
                                            repo,
                                            &target_for_reveal,
                                            is_head,
                                        )
                                    }) else {
                                        return;
                                    };
                                    this.select_branch_and_reveal_tip(
                                        repo_id,
                                        target_for_reveal.clone(),
                                        target.commit_id,
                                        target.fallback_scope,
                                        pin_key.clone(),
                                        cx,
                                    );
                                    this.retain_sidebar_click_target(ix, surface, e);
                                    return;
                                }
                                if e.click_count() < 2 {
                                    return;
                                }
                                match section {
                                    BranchSection::Local => {
                                        match local_branch_double_click_action(
                                            full_name_for_checkout.as_ref(),
                                            badge_worktree_path.as_deref(),
                                        ) {
                                            LocalBranchDoubleClickAction::CheckoutBranch {
                                                name,
                                            } => {
                                                this.store.dispatch(Msg::CheckoutBranch {
                                                    repo_id,
                                                    name,
                                                });
                                                this.rebuild_diff_cache(cx);
                                                cx.notify();
                                            }
                                            LocalBranchDoubleClickAction::OpenWorktree { path } => {
                                                crate::app::open_repository_from_view(
                                                    cx,
                                                    window.window_handle().window_id(),
                                                    path,
                                                );
                                                cx.notify();
                                            }
                                        }
                                    }
                                    BranchSection::Remote => {
                                        if let Some((remote, branch)) =
                                            target_for_checkout.remote_parts()
                                        {
                                            this.open_popover_at(
                                                PopoverKind::CheckoutRemoteBranchPrompt {
                                                    repo_id,
                                                    remote: remote.to_string(),
                                                    branch: branch.to_string(),
                                                },
                                                e.position(),
                                                window,
                                                cx,
                                            );
                                            cx.notify();
                                        }
                                    }
                                }
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_popover_at(
                                    (PopoverKind::BranchMenu {
                                        repo_id,
                                        target: target_for_menu.clone(),
                                    })
                                    .invoked_by(context_menu_invoker_for_right_click.clone()),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .gitcomet_tooltip(
                            theme,
                            super::super::branch_sidebar::branch_sidebar_branch_tooltip(
                                full_name_for_tooltip.as_ref(),
                                is_upstream,
                            ),
                        );

                    row.into_any_element()
                }
            })
            .collect()
    }
}

impl DetailsPaneView {
    pub(in super::super) fn render_commit_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let Loadable::Ready(details) = &repo.history_state.commit_details else {
            return Vec::new();
        };

        let theme = this.theme;
        let ui_scale_percent = this.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let repo_id = repo.id;
        let has_active_menu = this.active_context_menu_invoker.is_some();
        let file_rows = this.cached_commit_file_rows(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let projection = this.cached_commit_file_projection(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let plan = this.cached_commit_file_plan(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let is_tree = plan.is_tree();
        let visible_signature = this.commit_files_visible_signature(
            repo_id,
            repo.history_state.commit_details_rev,
            &range,
            projection.source_indices.len(),
        );
        // A tree shows leaf names, which have no shared prefix to align, and a
        // row that reported into the group would anchor it on the shortest one.
        let path_alignment_group = (!is_tree).then(|| {
            this.commit_files_path_alignment_group
                .visible_rows(visible_signature)
        });

        let rows: Vec<(usize, crate::view::rows::FileListRow)> = range
            .filter_map(|row_ix| {
                plan.row_at(crate::view::rows::RowIx(row_ix))
                    .map(|row| (row_ix, row))
            })
            .collect();

        rows.into_iter()
            .filter_map(|(ix, row)| {
                let (ordinal, depth) = match row {
                    crate::view::rows::FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        subtree: _,
                        additions,
                        deletions,
                    } => {
                        return Some(
                            crate::view::rows::directory_row(
                                crate::view::rows::DirectoryRowProps {
                                    theme,
                                    ui_scale_percent,
                                    id: ("commit_file_dir", ix).into(),
                                    label: &label,
                                    depth,
                                    collapsed,
                                    additions,
                                    deletions,
                                    row_height: sidebar_list_row_height(theme, ui_scale_percent),
                                    row_group: None,
                                    detail: crate::view::rows::directory_row_detail_for_width(
                                        // No width probe on this list.
                                        gpui::Pixels::MAX,
                                        depth,
                                        additions.is_some() || deletions.is_some(),
                                        ui_scale_percent,
                                    ),
                                },
                            )
                            .debug_selector(move || format!("commit_file_dir_{}_{}", repo_id.0, ix))
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &ClickEvent, _window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    this.toggle_file_list_dir(
                                        repo_id,
                                        crate::view::rows::FileListId::CommitFiles,
                                        Arc::clone(&key),
                                        Arc::clone(&chain),
                                        collapsed,
                                        cx,
                                    );
                                }),
                            )
                            .into_any_element(),
                        );
                    }
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                };
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) =
                    details.files.get(source_ix).zip(file_rows.get(source_ix))?;
                let visuals = presentation.visuals;
                let path_label = if is_tree {
                    SharedString::from(
                        f.path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| presentation.label.to_string()),
                    )
                } else {
                    presentation.label.clone()
                };
                let commit_id = details.id.clone();
                let (icon, color) = if f.is_submodule {
                    (visuals.icon, visuals.color(&theme))
                } else {
                    crate::view::rows::file_row_icon(&f.path, f.kind, &theme)
                };
                // The change kind rides the row wash and a badge on the icon's corner.
                let tint = crate::view::rows::file_kind_row_tint(f.kind, &theme);
                let badge = crate::view::rows::file_row_kind_badge(f.kind, &theme);

                let context_menu_active = has_active_menu && {
                    let invoker: SharedString = format!(
                        "commit_file_menu_{}_{}_{}",
                        repo_id.0,
                        commit_id.as_ref(),
                        f.path.display()
                    )
                    .into();
                    this.active_context_menu_invoker.as_ref() == Some(&invoker)
                };
                let selected = repo
                    .diff_state
                    .diff_target
                    .as_ref()
                    .is_some_and(|t| match t {
                        DiffTarget::Commit {
                            commit_id: t_commit_id,
                            path: Some(t_path),
                            ..
                        } => t_commit_id == &commit_id && t_path == &f.path,
                        _ => false,
                    });
                let row_group: SharedString = format!("commit_file_row_{ix}").into();
                let interaction = crate::view::rows::FileRowInteraction::new(
                    theme,
                    tint,
                    selected,
                    context_menu_active,
                );
                let badge_disc = interaction.badge_disc(row_group.clone());

                let commit_id_for_click = commit_id.clone();
                let commit_id_for_menu = commit_id.clone();
                // One owned copy shared by both handlers instead of one each.
                let path_for_click: Arc<std::path::PathBuf> = Arc::new(f.path.clone());
                let path_for_menu = Arc::clone(&path_for_click);
                // A rename's old side loads from where the file came from.
                let old_path_for_click = f.old_path.clone();
                let tooltip = path_label.clone();

                let row = div()
                    .id(("commit_file", ix))
                    // Only so the badge disc can follow the row's hover fill.
                    .group(row_group.clone())
                    .debug_selector(move || format!("commit_file_{}_{}", repo_id.0, ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .pl(if is_tree {
                        crate::view::rows::file_row_indent_px(depth, ui_scale_percent)
                    } else {
                        scaled_px(8.0)
                    })
                    .pr(scaled_px(8.0))
                    .w_full()
                    .map(|row| interaction.apply(row))
                    .child(crate::view::rows::file_row_icon_slot(
                        icon,
                        color,
                        badge,
                        badge_disc,
                        14.0,
                        16.0,
                        ui_scale_percent,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .line_height(theme.ui_text(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                match path_alignment_group.clone() {
                                    Some(group) => components::TruncatedText::aligned_path(
                                        path_label,
                                        theme.ui_text(14.0),
                                        group,
                                    ),
                                    // A tree row's label is a bare file name, so
                                    // there is no path to align against.
                                    None => components::TruncatedText::new(
                                        path_label,
                                        theme.ui_text(14.0),
                                    ),
                                }
                                .render(cx),
                            ),
                    )
                    .when(f.additions.is_some() || f.deletions.is_some(), |row| {
                        row.child(div().flex_none().child(components::diff_stat(
                            theme,
                            ui_scale_percent,
                            f.additions.unwrap_or(0) as usize,
                            f.deletions.unwrap_or(0) as usize,
                        )))
                    })
                    .on_activate(
                        false,
                        controls::ControlActivation::Composite,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            let target = DiffTarget::commit(
                                commit_id_for_click.clone(),
                                Some((*path_for_click).clone()),
                            )
                            .with_old_path(old_path_for_click.clone());
                            let selected = this.active_repo().is_some_and(|repo| {
                                repo.id == repo_id
                                    && repo.diff_state.diff_target.as_ref() == Some(&target)
                            });

                            if selected {
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                            } else {
                                this.focus_diff_panel(window, cx);
                                this.store.dispatch(Msg::SelectDiff { repo_id, target });
                            }
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone());
                let row = row.on_pointer_click(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        let invoker: SharedString = format!(
                            "commit_file_menu_{}_{}_{}",
                            repo_id.0,
                            commit_id_for_menu.as_ref(),
                            path_for_menu.display()
                        )
                        .into();
                        this.open_popover_at(
                            (PopoverKind::CommitFileMenu {
                                repo_id,
                                commit_id: commit_id_for_menu.clone(),
                                path: (*path_for_menu).clone(),
                            })
                            .invoked_by(invoker),
                            e.position,
                            window,
                            cx,
                        );
                        cx.notify();
                    }),
                );

                Some(row.into_any_element())
            })
            .collect()
    }

    /// Render the changed-file rows for an active two-point comparison. Mirrors
    /// [`Self::render_commit_file_rows`] but sources the file list from
    /// `history_state.range_files` and builds `DiffTarget::CommitRange` targets,
    /// so clicking a file loads its diff through the normal diff pipeline.
    /// Changed files of a linked worktree that is not this tab.
    ///
    /// Clicking one opens it through the inline foreign-diff machinery — the
    /// same path submodule diffs take — so the diff renders here rather than
    /// forcing a tab switch.
    pub(in super::super) fn render_worktree_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let repo_id = repo.id;
        let worktree_dirty_rev = repo.worktree_dirty_rev;
        let Some(summary) = this.selected_worktree_summary() else {
            return Vec::new();
        };
        // Derived once per scan, not per frame: this list is virtualized, but the
        // inputs behind it are one entry per changed file.
        let inputs = this.cached_worktree_file_inputs(repo_id, worktree_dirty_rev, summary);
        let files = &inputs.files;

        let theme = this.theme;
        let ui_scale_percent = this.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let file_rows =
            this.cached_worktree_file_rows(repo_id, worktree_dirty_rev, &summary.path, files);
        let projection =
            this.cached_worktree_file_projection(repo_id, worktree_dirty_rev, &summary.path, files);
        let plan =
            this.cached_worktree_file_plan(repo_id, worktree_dirty_rev, &summary.path, files);
        let is_tree = plan.is_tree();
        let selected_ix_now = repo
            .diff_state
            .inline_submodule_diff
            .as_ref()
            .filter(|inline| inline.submodule_repo_path == summary.path)
            .map(|inline| inline.selected_ix);
        let visible_signature = this.worktree_files_visible_signature(
            repo_id,
            worktree_dirty_rev,
            &summary.path,
            &range,
            files.len(),
        );
        let path_alignment_group = (!is_tree).then(|| {
            this.worktree_files_path_alignment_group
                .visible_rows(visible_signature)
        });
        let worktree_path = summary.path.clone();
        let origin = gitcomet_state::model::ForeignDiffOrigin::Worktree {
            branch: summary.branch.clone(),
            detached: summary.detached,
        };

        let rows: Vec<(usize, crate::view::rows::FileListRow)> = range
            .filter_map(|row_ix| {
                plan.row_at(crate::view::rows::RowIx(row_ix))
                    .map(|row| (row_ix, row))
            })
            .collect();

        rows.into_iter()
            .filter_map(|(ix, row)| {
                let (ordinal, depth) = match row {
                    crate::view::rows::FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        subtree: _,
                        additions,
                        deletions,
                    } => {
                        return Some(
                            crate::view::rows::directory_row(
                                crate::view::rows::DirectoryRowProps {
                                    theme,
                                    ui_scale_percent,
                                    id: ("worktree_file_dir", ix).into(),
                                    label: &label,
                                    depth,
                                    collapsed,
                                    additions,
                                    deletions,
                                    row_height: sidebar_list_row_height(theme, ui_scale_percent),
                                    row_group: None,
                                    detail: crate::view::rows::directory_row_detail_for_width(
                                        // No width probe on this list.
                                        gpui::Pixels::MAX,
                                        depth,
                                        additions.is_some() || deletions.is_some(),
                                        ui_scale_percent,
                                    ),
                                },
                            )
                            .debug_selector(move || {
                                format!("worktree_file_dir_{}_{}", repo_id.0, ix)
                            })
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &ClickEvent, _window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    this.toggle_file_list_dir(
                                        repo_id,
                                        crate::view::rows::FileListId::WorktreeFiles,
                                        Arc::clone(&key),
                                        Arc::clone(&chain),
                                        collapsed,
                                        cx,
                                    );
                                }),
                            )
                            .into_any_element(),
                        );
                    }
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                };
                // `source_ix` indexes `inputs.entries`, which the reducer
                // re-derives independently. Sorting the display must not change
                // the index a click sends.
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) = files.get(source_ix).zip(file_rows.get(source_ix))?;
                let visuals = presentation.visuals;
                let path_label = if is_tree {
                    SharedString::from(
                        f.path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| presentation.label.to_string()),
                    )
                } else {
                    presentation.label.clone()
                };
                let (icon, color) = if f.is_submodule {
                    (visuals.icon, visuals.color(&theme))
                } else {
                    crate::view::rows::file_row_icon(&f.path, f.kind, &theme)
                };
                // The change kind rides the row wash and a badge on the icon's corner.
                let tint = crate::view::rows::file_kind_row_tint(f.kind, &theme);
                let badge = crate::view::rows::file_row_kind_badge(f.kind, &theme);
                let selected = selected_ix_now == Some(source_ix);
                let tooltip = path_label.clone();
                let ix_for_click = source_ix;
                let inputs_for_click = Arc::clone(&inputs);
                let worktree_path_for_click = worktree_path.clone();
                let origin_for_click = origin.clone();

                let row_group: SharedString = format!("worktree_file_row_{ix}").into();
                let interaction =
                    crate::view::rows::FileRowInteraction::new(theme, tint, selected, false);
                let badge_disc = interaction.badge_disc(row_group.clone());

                let row = div()
                    .id(("worktree_file", ix))
                    // Only so the badge disc can follow the row's hover fill.
                    .group(row_group.clone())
                    .debug_selector(move || format!("worktree_file_{}_{}", repo_id.0, ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .pl(if is_tree {
                        crate::view::rows::file_row_indent_px(depth, ui_scale_percent)
                    } else {
                        scaled_px(8.0)
                    })
                    .pr(scaled_px(8.0))
                    .w_full()
                    .map(|row| interaction.apply(row))
                    .child(crate::view::rows::file_row_icon_slot(
                        icon,
                        color,
                        badge,
                        badge_disc,
                        14.0,
                        16.0,
                        ui_scale_percent,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .line_height(theme.ui_text(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                match path_alignment_group.clone() {
                                    Some(group) => components::TruncatedText::aligned_path(
                                        path_label,
                                        theme.ui_text(14.0),
                                        group,
                                    ),
                                    // A tree row's label is a bare file name, so
                                    // there is no path to align against.
                                    None => components::TruncatedText::new(
                                        path_label,
                                        theme.ui_text(14.0),
                                    ),
                                }
                                .render(cx),
                            ),
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Composite,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            this.focus_diff_panel(window, cx);
                            this.store.dispatch(Msg::OpenInlineSubmoduleDiff {
                                repo_id,
                                origin: origin_for_click.clone(),
                                submodule_repo_path: worktree_path_for_click.clone(),
                                parent_submodule_path: worktree_path_for_click.clone(),
                                entries: Arc::clone(&inputs_for_click.entries),
                                selected_ix: ix_for_click,
                            });
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone());

                Some(row.into_any_element())
            })
            .collect()
    }

    pub(in super::super) fn render_range_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let Some(range_selection) = repo.history_state.range_selection.clone() else {
            return Vec::new();
        };
        let Loadable::Ready(files) = &repo.history_state.range_files else {
            return Vec::new();
        };
        let files = files.clone();

        let theme = this.theme;
        let ui_scale_percent = this.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let repo_id = repo.id;
        let from = range_selection.from.clone();
        let to = range_selection.to.clone();
        let file_rows =
            this.cached_range_file_rows(repo_id, repo.history_state.range_files_rev, &files);
        let projection =
            this.cached_range_file_projection(repo_id, repo.history_state.range_files_rev, &files);
        let plan = this.cached_range_file_plan(repo_id, repo.history_state.range_files_rev, &files);
        let is_tree = plan.is_tree();
        let visible_signature = this.range_files_visible_signature(
            repo_id,
            repo.history_state.range_files_rev,
            &range,
            files.len(),
        );
        let path_alignment_group = (!is_tree).then(|| {
            this.range_files_path_alignment_group
                .visible_rows(visible_signature)
        });

        let rows: Vec<(usize, crate::view::rows::FileListRow)> = range
            .filter_map(|row_ix| {
                plan.row_at(crate::view::rows::RowIx(row_ix))
                    .map(|row| (row_ix, row))
            })
            .collect();

        rows.into_iter()
            .filter_map(|(ix, row)| {
                let (ordinal, depth) = match row {
                    crate::view::rows::FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        subtree: _,
                        additions,
                        deletions,
                    } => {
                        return Some(
                            crate::view::rows::directory_row(
                                crate::view::rows::DirectoryRowProps {
                                    theme,
                                    ui_scale_percent,
                                    id: ("range_file_dir", ix).into(),
                                    label: &label,
                                    depth,
                                    collapsed,
                                    additions,
                                    deletions,
                                    row_height: sidebar_list_row_height(theme, ui_scale_percent),
                                    row_group: None,
                                    detail: crate::view::rows::directory_row_detail_for_width(
                                        // No width probe on this list.
                                        gpui::Pixels::MAX,
                                        depth,
                                        additions.is_some() || deletions.is_some(),
                                        ui_scale_percent,
                                    ),
                                },
                            )
                            .debug_selector(move || format!("range_file_dir_{}_{}", repo_id.0, ix))
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &ClickEvent, _window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    this.toggle_file_list_dir(
                                        repo_id,
                                        crate::view::rows::FileListId::RangeFiles,
                                        Arc::clone(&key),
                                        Arc::clone(&chain),
                                        collapsed,
                                        cx,
                                    );
                                }),
                            )
                            .into_any_element(),
                        );
                    }
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                };
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) = files.get(source_ix).zip(file_rows.get(source_ix))?;
                let visuals = presentation.visuals;
                let path_label = if is_tree {
                    SharedString::from(
                        f.path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| presentation.label.to_string()),
                    )
                } else {
                    presentation.label.clone()
                };
                let (icon, color) = if f.is_submodule {
                    (visuals.icon, visuals.color(&theme))
                } else {
                    crate::view::rows::file_row_icon(&f.path, f.kind, &theme)
                };
                // The change kind rides the row wash and a badge on the icon's corner.
                let tint = crate::view::rows::file_kind_row_tint(f.kind, &theme);
                let badge = crate::view::rows::file_row_kind_badge(f.kind, &theme);
                let target = DiffTarget::commit_range(from.clone(), to.clone(), None).for_change(f);
                let selected = repo.diff_state.diff_target.as_ref() == Some(&target);
                let target_for_click = target.clone();
                let tooltip = path_label.clone();

                let row_group: SharedString = format!("range_file_row_{ix}").into();
                let interaction =
                    crate::view::rows::FileRowInteraction::new(theme, tint, selected, false);
                let badge_disc = interaction.badge_disc(row_group.clone());

                let row = div()
                    .id(("range_file", ix))
                    // Only so the badge disc can follow the row's hover fill.
                    .group(row_group.clone())
                    .debug_selector(move || format!("range_file_{}_{}", repo_id.0, ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .pl(if is_tree {
                        crate::view::rows::file_row_indent_px(depth, ui_scale_percent)
                    } else {
                        scaled_px(8.0)
                    })
                    .pr(scaled_px(8.0))
                    .w_full()
                    .map(|row| interaction.apply(row))
                    .child(crate::view::rows::file_row_icon_slot(
                        icon,
                        color,
                        badge,
                        badge_disc,
                        14.0,
                        16.0,
                        ui_scale_percent,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .line_height(theme.ui_text(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                match path_alignment_group.clone() {
                                    Some(group) => components::TruncatedText::aligned_path(
                                        path_label,
                                        theme.ui_text(14.0),
                                        group,
                                    ),
                                    // A tree row's label is a bare file name, so
                                    // there is no path to align against.
                                    None => components::TruncatedText::new(
                                        path_label,
                                        theme.ui_text(14.0),
                                    ),
                                }
                                .render(cx),
                            ),
                    )
                    .when(f.additions.is_some() || f.deletions.is_some(), |row| {
                        row.child(div().flex_none().child(components::diff_stat(
                            theme,
                            ui_scale_percent,
                            f.additions.unwrap_or(0) as usize,
                            f.deletions.unwrap_or(0) as usize,
                        )))
                    })
                    .on_activate(
                        false,
                        controls::ControlActivation::Composite,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            let selected = this.active_repo().is_some_and(|repo| {
                                repo.id == repo_id
                                    && repo.diff_state.diff_target.as_ref()
                                        == Some(&target_for_click)
                            });
                            if selected {
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                            } else {
                                this.focus_diff_panel(window, cx);
                                this.store.dispatch(Msg::SelectDiff {
                                    repo_id,
                                    target: target_for_click.clone(),
                                });
                            }
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone());

                Some(row.into_any_element())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
