use super::*;
use crate::ui_scale;
use crate::view::components::InteractiveRowExt as _;
use gitcomet_core::domain::LogScope;
use gitcomet_core::domain::SubmoduleStatus;
use palette::IntoColor;
use std::num::NonZeroU32;

pub(in crate::view) const WORKTREE_ICON_PATH: &str = "icons/git_worktree.svg";
const STASH_ICON_PATH: &str = crate::view::icons::STASH_ICON_PATH;

pub(in crate::view) fn listed_workspace_paths_by_branch(
    repo: &RepoState,
) -> HashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return HashMap::default();
    };

    let mut worktree_paths = HashMap::default();
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

fn branch_workspace_badge_path(
    listed_workspace_path: Option<&std::path::Path>,
    active_workspace_path: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    listed_workspace_path
        .map(std::path::Path::to_path_buf)
        .or_else(|| active_workspace_path.map(std::path::Path::to_path_buf))
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
    pub(in crate::view) text: gpui::Rgba,
    pub(in crate::view) hover_text: gpui::Rgba,
    open_text: gpui::Rgba,
    active_text: gpui::Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorktreeBadgeColors {
    border: gpui::Rgba,
    hover_border: gpui::Rgba,
    text: gpui::Rgba,
    hover_text: gpui::Rgba,
}

/// Shared chip palette for the sidebar badges (workspace/worktree/upstream),
/// matching the history table's ref chips: an elevated pill with a quiet
/// border at rest, accent-tinted when the badge refers to an open workspace.
pub(in crate::view) fn worktree_badge_palette(theme: AppTheme) -> WorktreeBadgePalette {
    WorktreeBadgePalette {
        bg: theme.colors.surface.raised,
        active_bg: with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.16 } else { 0.10 },
        ),
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
        text: theme.colors.foreground.secondary,
        hover_text: theme.colors.foreground.primary,
        open_text: theme.colors.accent.foreground,
        active_text: theme.colors.accent.foreground,
    }
}

fn worktree_badge_colors(
    palette: WorktreeBadgePalette,
    is_open: bool,
    menu_active: bool,
) -> WorktreeBadgeColors {
    WorktreeBadgeColors {
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
        text: if menu_active {
            palette.active_text
        } else if is_open {
            palette.open_text
        } else {
            palette.text
        },
        hover_text: if is_open {
            palette.open_text
        } else {
            palette.hover_text
        },
    }
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

/// The chip that marks content as belonging to another worktree. Shown on the
/// history row, and again in the details and diff headers so it is never unclear
/// whose changes are on screen.
///
/// Deliberately style-only: callers add the id, cursor, hover and click when the
/// chip is an affordance rather than a label.
pub(in crate::view) fn worktree_origin_chip(
    theme: AppTheme,
    label: SharedString,
    icon_size: Pixels,
    height: Pixels,
    max_width: Pixels,
    pad_x: Pixels,
) -> gpui::Div {
    let palette = worktree_badge_palette(theme);
    div()
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
        .child(svg_icon(WORKTREE_ICON_PATH, palette.text, icon_size))
        .child(
            div()
                .text_xs()
                .text_color(palette.text)
                .line_clamp(1)
                .whitespace_nowrap()
                .child(label),
        )
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
fn filtered_label_element<V: 'static>(
    label: SharedString,
    query: &str,
    text_color: gpui::Rgba,
    highlight_color: gpui::Rgba,
    text_size: gpui::AbsoluteLength,
    font_weight: FontWeight,
    cx: &gpui::Context<V>,
) -> AnyElement {
    // `to_ascii_lowercase` preserves byte length, so the match offset is valid
    // in the original (mixed-case) label.
    if !query.is_empty()
        && let Some(start) = label.to_ascii_lowercase().find(query)
    {
        let range = start..start + query.len();
        let highlight = gpui::HighlightStyle {
            color: Some(highlight_color.into_color()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        components::TruncatedText::new(label)
            .text_color(text_color)
            .text_size(text_size)
            .font_weight(font_weight)
            .highlights([(range, highlight)])
            .render(cx)
            .into_any_element()
    } else {
        label.into_any_element()
    }
}

pub(in crate::view) fn active_workspace_paths_by_branch(
    repo: &RepoState,
    open_repos: &[RepoState],
) -> HashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return HashMap::default();
    };

    let mut active_workspaces = HashMap::default();
    for worktree in worktrees.iter() {
        let Some(open_repo) = open_repos
            .iter()
            .find(|open_repo| open_repo.spec.workdir == worktree.path)
        else {
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

        active_workspaces
            .entry(branch)
            .or_insert_with(|| worktree.path.clone());
    }

    active_workspaces
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LocalBranchDoubleClickAction {
    CheckoutBranch { name: String },
    OpenWorkspace { path: std::path::PathBuf },
}

fn local_branch_double_click_action(
    branch: &str,
    workspace_path: Option<&std::path::Path>,
) -> LocalBranchDoubleClickAction {
    match workspace_path {
        Some(path) => LocalBranchDoubleClickAction::OpenWorkspace {
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

fn branch_commit_id(repo: &RepoState, section: BranchSection, name: &str) -> Option<CommitId> {
    match section {
        BranchSection::Local => match &repo.branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|branch| branch.name == name)
                .map(|branch| branch.target.clone()),
            _ => None,
        },
        BranchSection::Remote => {
            let (remote, branch_name) = name.split_once('/')?;
            match &repo.remote_branches {
                Loadable::Ready(branches) => branches
                    .iter()
                    .find(|branch| branch.remote == remote && branch.name == branch_name)
                    .map(|branch| branch.target.clone()),
                _ => None,
            }
        }
    }
}

fn branch_click_history_reveal_target(
    repo: &RepoState,
    section: BranchSection,
    name: &str,
    is_head: bool,
) -> Option<BranchHistoryRevealTarget> {
    let commit_id = branch_commit_id(repo, section, name)?;

    let fallback_scope = match section {
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
    section: BranchSection,
    name: &str,
    selected_commit: Option<&CommitId>,
    selected_branch_commit_id: Option<&CommitId>,
) -> bool {
    selected_branch.is_some_and(|selected_branch| {
        selected_branch.repo_id == repo_id
            && selected_branch.section == section
            && selected_branch.name == name
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
        const BRANCH_TREE_BASE_PAD_PX: f32 = 6.0;
        const BRANCH_TREE_DEPTH_STEP_PX: f32 = 8.0;
        const BRANCH_TREE_TOGGLE_SLOT_PX: f32 = 12.0;
        const BRANCH_TREE_ICON_SLOT_PX: f32 = 16.0;
        const BRANCH_TREE_GAP_PX: f32 = 4.0;
        const BRANCH_BADGE_GAP_PX: f32 = 3.0;
        /// Widest a branch row's worktree pill may grow before its label starts
        /// truncating. Wide enough for the folder names worktrees usually carry,
        /// narrow enough that the pill always fits the pane — which is what
        /// keeps every row's badge on one right edge.
        const BRANCH_WORKTREE_BADGE_MAX_W_PX: f32 = 140.0;
        /// Gap between a row's trailing badge and the row's right edge, shared
        /// by every row (headers included) so the badges land on one edge.
        /// This is as tight as the badges can sit: the list insets its rows by
        /// `ROW_HIGHLIGHT_INSET_PX` while the overlay scrollbar's thumb paints
        /// 4..10px in from the *pane* edge, which puts the thumb's leading edge
        /// exactly 4px inside the row. Any less and a visible thumb would paint
        /// over the badge.
        const BRANCH_ROW_TRAILING_PAD_PX: f32 = 4.0;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);

        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        // Prefer the transient section-scoped presentation set while rendering a
        // collapsed-sidebar popover; fall back to the full cached presentation.
        // Each surface highlights matches from its own filter: the popover's
        // toggled filter box, or the expanded sidebar's filter bar.
        let is_collapsed_popover = this.collapsed_popover_presentation.is_some();
        let filter_query = if is_collapsed_popover {
            this.collapsed_popover_filter_query
                .trim()
                .to_ascii_lowercase()
        } else {
            this.branch_filter_query.trim().to_ascii_lowercase()
        };
        let Some(presentation) = this
            .collapsed_popover_presentation
            .clone()
            .or_else(|| this.branch_sidebar_presentation_cached())
        else {
            return Vec::new();
        };
        let rows = presentation.rows;
        let workspace_badges = presentation.workspace_badges;
        let repo_workdir = this.active_repo().map(|r| r.spec.workdir.clone());
        let theme = this.theme;
        let worktree_badge_palette = worktree_badge_palette(theme);
        let icon_primary = theme.colors.accent.foreground;
        let icon_muted = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.72 } else { 0.82 },
        );
        let selected_branch = this.selected_branch().cloned();
        let (selected_commit, selected_branch_commit_id) =
            this.active_repo().map_or((None, None), |repo| {
                let selected_commit = repo.history_state.selected_commit.clone();
                let selected_branch_commit_id = selected_branch
                    .as_ref()
                    .filter(|selected| selected.repo_id == repo_id)
                    .and_then(|selected| {
                        branch_commit_id(repo, selected.section, selected.name.as_str())
                    });
                (selected_commit, selected_branch_commit_id)
            });

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
                10.0,
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

        let indent_px = |depth: usize| {
            scaled_px(BRANCH_TREE_BASE_PAD_PX + depth as f32 * BRANCH_TREE_DEPTH_STEP_PX)
        };

        // The same translucent interaction overlays are used on both surfaces.
        // The style also resolves those overlays for label fades, so the fade
        // and the row can never disagree about a semantic state.
        let row_surface = if is_collapsed_popover {
            theme.colors.surface.raised
        } else {
            theme.colors.surface.chrome
        };
        let row_style = components::InteractiveRowStyle::new(theme, row_surface);

        let top_divider = |color: gpui::Rgba| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(scaled_px(1.0))
                .bg(color)
        };

        range
            .filter_map(|ix| rows.get(ix).cloned().map(|r| (ix, r)))
            .map(|(ix, row)| match row {
                BranchSidebarRow::PinnedHeader {
                    section,
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let (label, selector_suffix): (SharedString, &'static str) = match section {
                        BranchSection::Local => ("Pinned Local Branches".into(), "local"),
                        BranchSection::Remote => ("Pinned Remote Branches".into(), "remote"),
                    };
                    let context_menu_invoker: SharedString =
                        format!("pinned_section_menu_{}_{selector_suffix}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let menu_kind = PopoverKind::PinnedSectionMenu { repo_id, section };
                    let menu_kind_for_right_click = menu_kind.clone();
                    div()
                        .id(("pinned_section", ix))
                        .debug_selector(move || format!("pinned_section_{selector_suffix}"))
                        .relative()
                        .h(scaled_px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(
                            row_style,
                            components::InteractiveRowState::default().open(context_menu_active),
                        )
                        .when(top_border, |d| {
                            d.child(top_divider(theme.colors.stroke.subtle))
                        })
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/pin.svg", icon_primary, 13.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.foreground.primary)
                                .child(label.clone()),
                        )
                        .gitcomet_tooltip(theme, label)
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    menu_kind_for_right_click.clone(),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SectionHeader {
                    section,
                    top_border,
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
                        .id(("branch_section", ix))
                        .relative()
                        .h(scaled_px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .when(top_border, |d| {
                            d.child(top_divider(theme.colors.stroke.subtle))
                        })
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot(icon_path, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.foreground.primary)
                                .child(label),
                        )
                        .gitcomet_tooltip(theme, tooltip.clone())
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::BranchSectionMenu { repo_id, section },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::FilterGroupHeader { section } => {
                    let (icon_path, label): (&'static str, SharedString) = match section {
                        BranchSection::Local => ("icons/computer.svg", "Local Branches".into()),
                        BranchSection::Remote => ("icons/cloud.svg", "Remote Branches".into()),
                    };
                    let selector_suffix = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    // Purely a divider between the two halves of a cross-section
                    // filter result: no collapse toggle, no menu, no hover.
                    div()
                        .id(("branch_filter_group", ix))
                        .debug_selector(move || format!("branch_filter_group_{selector_suffix}"))
                        .h(scaled_px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(icon_path, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.foreground.secondary)
                                .child(label),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SectionSpacer => div()
                    .id(("branch_section_spacer", ix))
                    .h(scaled_px(10.0))
                    .w_full()
                    .into_any_element(),
                BranchSidebarRow::StashHeader {
                    top_border,
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
                        .id(("stash_section", ix))
                        .debug_selector(move || format!("stash_section_{ix}"))
                        .relative()
                        .h(scaled_px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .when(top_border, |d| {
                            d.child(top_divider(theme.colors.stroke.subtle))
                        })
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot(STASH_ICON_PATH, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.foreground.primary)
                                .child("Stash"),
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::StashPrompt,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::StashPlaceholder { message } => div()
                    .id(("stash_placeholder", ix))
                    .h(scaled_px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.foreground.secondary)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::StashItem {
                    index,
                    message,
                    tooltip,
                    created_at: _,
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
                        .relative()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .h(scaled_px(24.0))
                        .w_full()
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(STASH_ICON_PATH, icon_primary, 12.0))
                        .child(
                            components::FadingText::new(
                                div().text_sm().child(message.clone()),
                                row_style.resolved_background(row_state),
                            )
                            .hover_bg(
                                row_group.clone(),
                                row_style.resolved_hover_background(row_state),
                            )
                            .render(ui_scale_percent)
                            .flex_1(),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            this.store.dispatch(Msg::ApplyStash { repo_id, index });
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::StashMenu {
                                        repo_id,
                                        index,
                                        message: stash_message_for_right_click.clone(),
                                    },
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
                    .h(scaled_px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.foreground.secondary)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::WorktreesHeader {
                    top_border,
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
                        .id(("worktrees_section", ix))
                        .debug_selector(move || format!("worktrees_section_{ix}"))
                        .relative()
                        .h(scaled_px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .when(top_border, |d| {
                            d.child(top_divider(theme.colors.stroke.subtle))
                        })
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot(WORKTREE_ICON_PATH, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::SectionMenu,
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::WorktreePlaceholder { message } => div()
                    .id(("worktree_placeholder", ix))
                    .h(scaled_px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
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
                        .h(scaled_px(22.0))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(WORKTREE_ICON_PATH, icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
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
                                            components::TruncatedText::path(path_label.clone())
                                                .id(("worktree_path_text", ix))
                                                .text_sm()
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
                                            // Same control radius as the branch
                                            // rows' worktree badge; the two are
                                            // the same chip in two lists.
                                            .rounded(px(theme.radii.control))
                                            .border_1()
                                            .border_color(branch_badge_colors.border)
                                            .bg(if worktree_tab_open {
                                                worktree_badge_palette.active_bg
                                            } else {
                                                worktree_badge_palette.bg
                                            })
                                            .text_size(scaled_px(11.0))
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
                                                branch_badge_colors.text,
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
                                                        )
                                                        .id(("worktree_branch_badge_text", ix))
                                                        .text_size(scaled_px(11.0))
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            if e.click_count() >= 2 {
                                this.store.dispatch(Msg::OpenRepo(path_for_open.clone()));
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
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                            branch: branch_for_menu.clone(),
                                        },
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulesHeader {
                    top_border,
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
                        .id(("submodules_section", ix))
                        .debug_selector(move || format!("submodules_section_{ix}"))
                        .relative()
                        .h(scaled_px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .when(top_border, |d| {
                            d.child(top_divider(theme.colors.stroke.subtle))
                        })
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::SectionMenu,
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulePlaceholder { message, can_load } => div()
                    .id(("submodule_placeholder", ix))
                    .h(scaled_px(24.0))
                    .w_full()
                    .pl_2()
                    .pr_1()
                    .flex()
                    .items_center()
                    .gap(scaled_px(6.0))
                    .text_sm()
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
                BranchSidebarRow::SubmoduleItem { path } => {
                    let path_for_open = path.clone();
                    let path_for_menu = path.clone();
                    let repo_workdir_for_open = repo_workdir.clone();
                    let path_label = this.cached_path_display(&path);
                    let submodule_info =
                        this.active_repo().and_then(|repo| match &repo.submodules {
                            Loadable::Ready(submodules) => submodules
                                .iter()
                                .find(|submodule| submodule.path == path)
                                .map(|submodule| {
                                    (
                                        submodule.status,
                                        submodule.recorded_head.clone(),
                                        submodule.checked_out_head.clone(),
                                    )
                                }),
                            _ => None,
                        });
                    let (icon_color, badge_label, can_open, tooltip) =
                        if let Some((status, recorded_head, checked_out_head)) = submodule_info {
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
                                SubmoduleStatus::HeadMismatch => {
                                    theme.colors.status.warning.foreground
                                }
                                SubmoduleStatus::MergeConflict
                                | SubmoduleStatus::MissingMapping => {
                                    theme.colors.status.danger.foreground
                                }
                                SubmoduleStatus::UpToDate | SubmoduleStatus::Unknown(_) => {
                                    icon_primary
                                }
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
                        } else {
                            (icon_primary, None, true, path_label.clone())
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
                        .h(scaled_px(22.0))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .interactive_row(row_style, row_state)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot("icons/box.svg", icon_color, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .debug_selector(move || format!("submodule_label_{ix}"))
                                .child(path_label),
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
                                    .text_size(scaled_px(11.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(badge_label),
                            )
                        })
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            if !can_open {
                                return;
                            }
                            let Some(base) = repo_workdir_for_open.clone() else {
                                return;
                            };
                            this.store
                                .dispatch(Msg::OpenRepo(base.join(&path_for_open)));
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                        },
                                    ),
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
                        .id(("branch_remote", ix))
                        .relative()
                        .h(scaled_px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(remote_color)
                        .child(tree_toggle_slot(Some(collapsed)))
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::remote(
                                        repo_id,
                                        RemotePopoverKind::Menu {
                                            name: remote_name_for_right_click.clone(),
                                        },
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
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
                    let group_icon_color = match section {
                        BranchSection::Local => icon_primary,
                        BranchSection::Remote => theme.colors.foreground.secondary,
                    };
                    let row_group: SharedString =
                        format!("branch_group_row_{}_{}", repo_id.0, ix).into();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString = format!(
                        "branch_group_menu_{}_{}_{}_{}",
                        repo_id.0,
                        section_key,
                        remote.as_deref().unwrap_or_default(),
                        path
                    )
                    .into();
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
                        .debug_selector(move || format!("branch_group_{ix}"))
                        .h(scaled_px(22.0))
                        .w_full()
                        .pl(indent_px(usize::from(depth)))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .interactive_row(row_style, row_state)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.colors.foreground.secondary)
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot(
                            super::super::file_icons::folder_icon(!collapsed),
                            group_icon_color,
                            14.0,
                        ))
                        .child(
                            components::FadingText::new(
                                filtered_label_element(
                                    label,
                                    &filter_query,
                                    theme.colors.foreground.secondary,
                                    theme.colors.accent.foreground,
                                    gpui::rems(0.75).into(),
                                    FontWeight::SEMIBOLD,
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    menu_kind_for_right_click.clone(),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::Branch {
                    name,
                    section,
                    depth,
                    muted,
                    divergence_ahead,
                    divergence_behind,
                    is_head,
                    is_upstream,
                } => {
                    let full_name_for_checkout: SharedString = name.clone();
                    let full_name_for_reveal: SharedString = name.clone();
                    let full_name_for_menu: SharedString = name.clone();
                    let full_name_for_tooltip: SharedString = name.clone();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString = format!(
                        "branch_menu_{}_{}_{}",
                        repo_id.0,
                        section_key,
                        full_name_for_menu.as_ref()
                    )
                    .into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let label: SharedString =
                        super::super::branch_sidebar::branch_sidebar_branch_label(name.as_ref())
                            .to_owned()
                            .into();
                    let workspace_path = (section == BranchSection::Local)
                        .then(|| workspace_badges.listed_path(name.as_ref()).cloned())
                        .flatten();
                    let active_workspace_path = (section == BranchSection::Local)
                        .then(|| workspace_badges.active_path(name.as_ref()).cloned())
                        .flatten();
                    let workspace_badge_path = branch_workspace_badge_path(
                        workspace_path.as_deref(),
                        active_workspace_path.as_deref(),
                    );
                    let branch_selected = branch_row_is_selected(
                        selected_branch.as_ref(),
                        repo_id,
                        section,
                        full_name_for_reveal.as_ref(),
                        selected_commit.as_ref(),
                        selected_branch_commit_id.as_ref(),
                    );
                    let has_worktree = workspace_badge_path.is_some();
                    let has_active_workspace = active_workspace_path.is_some();
                    let show_workspace_badge = has_worktree;
                    let workspace_row_menu_invoker: Option<SharedString> =
                        workspace_badge_path.as_ref().map(|path| {
                            format!("worktree_menu_{}_{}", repo_id.0, path.display()).into()
                        });
                    let workspace_menu_active =
                        workspace_row_menu_invoker.as_ref().is_some_and(|invoker| {
                            this.active_context_menu_invoker.as_ref() == Some(invoker)
                        });
                    let row_group: SharedString = format!("branch_row_{}_{}", repo_id.0, ix).into();
                    let row_debug_selector = row_group.as_ref().to_owned();
                    let branch_text_color = if muted {
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
                    let branch_icon_color = match section {
                        BranchSection::Local => {
                            if muted {
                                icon_muted
                            } else {
                                icon_primary
                            }
                        }
                        BranchSection::Remote => theme.colors.foreground.secondary,
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
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
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
                        // Accent-tinted variant of the shared badge chip, in
                        // line with the history table's tag chips.
                        let mut badge = div()
                            .px(scaled_px(6.0))
                            .rounded(px(theme.radii.pill))
                            .text_size(scaled_px(11.0))
                            .text_color(theme.colors.accent.foreground)
                            .bg(with_alpha(theme.colors.accent.foreground, 0.12))
                            .border_1()
                            .border_color(with_alpha(theme.colors.accent.foreground, 0.35))
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
                        .selected(is_head, head_highlight)
                        .selected(branch_selected, branch_selected_bg)
                        .open(context_menu_active);

                    let mut row = div()
                        .id(("branch_item", ix))
                        .debug_selector(move || row_debug_selector.clone())
                        .relative()
                        .h(if section == BranchSection::Local {
                            scaled_px(24.0)
                        } else {
                            scaled_px(22.0)
                        })
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(scaled_px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(usize::from(depth)))
                        .pr(scaled_px(BRANCH_ROW_TRAILING_PAD_PX))
                        .interactive_row(row_style, row_state)
                        .text_color(branch_text_color)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(
                            "icons/git_branch.svg",
                            branch_icon_color,
                            12.0,
                        ))
                        .child(
                            // Long branch names run into the trailing badges;
                            // fade them into the row instead of slicing a glyph.
                            components::FadingText::new(
                                div()
                                    .text_sm()
                                    .text_color(branch_selected_label_color)
                                    .child(filtered_label_element(
                                        label,
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
                        || show_workspace_badge;
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

                    if show_workspace_badge {
                        let Some(workspace_badge_path) = workspace_badge_path.clone() else {
                            unreachable!("workspace badge requires a worktree path");
                        };
                        let workspace_menu_invoker_for_click = workspace_row_menu_invoker.clone();
                        let workspace_menu_invoker_for_right_click =
                            workspace_row_menu_invoker.clone();
                        let workspace_path_for_menu = workspace_badge_path.clone();
                        let workspace_path_for_open = workspace_badge_path.clone();
                        let workspace_path_for_right_click = workspace_badge_path.clone();
                        let workspace_badge_label =
                            super::super::path_display::repo_path_name(&workspace_badge_path);
                        let worktree_badge_tooltip: SharedString =
                            workspace_badge_path.display().to_string().into();
                        let branch_name_for_click = name.to_string();
                        let branch_name_for_right_click = branch_name_for_click.clone();
                        let badge_colors = worktree_badge_colors(
                            worktree_badge_palette,
                            has_active_workspace,
                            workspace_menu_active,
                        );
                        let worktree_badge = div()
                            .id(("branch_workspace_badge", ix))
                            .debug_selector(move || format!("branch_workspace_badge_{ix}"))
                            .flex()
                            .items_center()
                            .gap(scaled_px(3.0))
                            .px(scaled_px(6.0))
                            // Squared off on the control radius the buttons and
                            // tabs use, rather than the fully-round `pill` the
                            // decorative chips (upstream, submodule) keep: this
                            // badge is a click target, so it reads as one.
                            .rounded(px(theme.radii.control))
                            .border_1()
                            .border_color(badge_colors.border)
                            .bg(if has_active_workspace {
                                worktree_badge_palette.active_bg
                            } else {
                                worktree_badge_palette.bg
                            })
                            .text_size(scaled_px(11.0))
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
                            .child(svg_icon(WORKTREE_ICON_PATH, badge_colors.text, 9.0))
                            .child(
                                div().min_w(px(0.0)).overflow_hidden().child(
                                    components::TruncatedText::new(workspace_badge_label)
                                        .id(("branch_workspace_badge_text", ix))
                                        .text_size(scaled_px(11.0))
                                        // Explicit color: TruncatedText resolves an
                                        // unset one from the ambient text style in a
                                        // deferred measure closure that never sees the
                                        // pill's `.text_color`.
                                        .text_color(badge_colors.text)
                                        .render(cx),
                                ),
                            )
                            .hover(move |s| {
                                if workspace_menu_active {
                                    s.bg(worktree_badge_palette.active_bg)
                                        .border_color(worktree_badge_palette.active_border)
                                        .text_color(worktree_badge_palette.active_text)
                                } else {
                                    s.bg(if has_active_workspace {
                                        worktree_badge_palette.active_bg
                                    } else {
                                        worktree_badge_palette.bg
                                    })
                                    .border_color(badge_colors.hover_border)
                                    .text_color(badge_colors.hover_text)
                                }
                            })
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                if !e.standard_click() {
                                    return;
                                }
                                cx.stop_propagation();
                                if e.click_count() >= 2 {
                                    this.store
                                        .dispatch(Msg::OpenRepo(workspace_path_for_open.clone()));
                                    cx.notify();
                                    return;
                                }
                                let Some(invoker) = workspace_menu_invoker_for_click.clone() else {
                                    return;
                                };
                                this.activate_context_menu_invoker(invoker, cx);
                                this.open_popover_at(
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::Menu {
                                            path: workspace_path_for_menu.clone(),
                                            branch: Some(branch_name_for_click.clone()),
                                        },
                                    ),
                                    e.position(),
                                    window,
                                    cx,
                                );
                            }))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    let Some(invoker) =
                                        workspace_menu_invoker_for_right_click.clone()
                                    else {
                                        return;
                                    };
                                    this.activate_context_menu_invoker(invoker, cx);
                                    this.open_popover_at(
                                        PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::Menu {
                                                path: workspace_path_for_right_click.clone(),
                                                branch: Some(branch_name_for_right_click.clone()),
                                            },
                                        ),
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            if e.click_count() == 1 {
                                let Some(target) = this.active_repo().and_then(|repo| {
                                    branch_click_history_reveal_target(
                                        repo,
                                        section,
                                        full_name_for_reveal.as_ref(),
                                        is_head,
                                    )
                                }) else {
                                    return;
                                };
                                this.set_selected_branch(
                                    repo_id,
                                    section,
                                    full_name_for_reveal.as_ref(),
                                    cx,
                                );
                                this.reveal_branch_commit_in_history(
                                    repo_id,
                                    section,
                                    full_name_for_reveal.as_ref(),
                                    target.commit_id,
                                    target.fallback_scope,
                                    cx,
                                );
                                cx.notify();
                                return;
                            }
                            if e.click_count() < 2 {
                                return;
                            }
                            match section {
                                BranchSection::Local => {
                                    match local_branch_double_click_action(
                                        full_name_for_checkout.as_ref(),
                                        workspace_path.as_deref(),
                                    ) {
                                        LocalBranchDoubleClickAction::CheckoutBranch { name } => {
                                            this.store
                                                .dispatch(Msg::CheckoutBranch { repo_id, name });
                                            this.rebuild_diff_cache(cx);
                                            cx.notify();
                                        }
                                        LocalBranchDoubleClickAction::OpenWorkspace { path } => {
                                            this.store.dispatch(Msg::OpenRepo(path));
                                            cx.notify();
                                        }
                                    }
                                }
                                BranchSection::Remote => {
                                    if let Some((remote, branch)) =
                                        full_name_for_checkout.as_ref().split_once('/')
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
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::BranchMenu {
                                        repo_id,
                                        section,
                                        name: full_name_for_menu.as_ref().to_owned(),
                                    },
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
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let repo_id = repo.id;
        let has_active_menu = this.active_context_menu_invoker.is_some();
        let file_rows = this.cached_commit_file_rows(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let visible_signature = this.commit_files_visible_signature(
            repo_id,
            repo.history_state.commit_details_rev,
            &range,
            details.files.len(),
        );
        let path_alignment_group = this
            .commit_files_path_alignment_group
            .visible_rows(visible_signature);

        range
            .filter_map(|ix| {
                details
                    .files
                    .get(ix)
                    .zip(file_rows.get(ix))
                    .map(|(f, row)| (ix, f, row.label.clone(), row.visuals))
            })
            .map(|(ix, f, path_label, visuals)| {
                let commit_id = details.id.clone();
                let icon = Some(visuals.icon);
                let color = visuals.color(&theme);

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
                        } => t_commit_id == &commit_id && t_path == &f.path,
                        _ => false,
                    });
                let commit_id_for_click = commit_id.clone();
                let path_for_click = f.path.clone();
                let commit_id_for_menu = commit_id.clone();
                let path_for_menu = f.path.clone();
                let tooltip = path_label.clone();

                let mut row = div()
                    .id(("commit_file", ix))
                    .debug_selector(move || format!("commit_file_{}_{}", repo_id.0, ix))
                    .h(scaled_px(24.0))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .px(scaled_px(8.0))
                    .w_full()
                    .rounded(px(theme.radii.row))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| {
                        if context_menu_active {
                            s.bg(theme.colors.interaction.pressed_background)
                        } else {
                            s.bg(theme.colors.interaction.hover_background)
                        }
                    })
                    .active(move |s| s.bg(theme.colors.interaction.pressed_background))
                    .child(
                        div()
                            .w(scaled_px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(icon, |this, icon| {
                                this.child(svg_icon(icon, color, scaled_px(14.0)))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .line_height(scaled_px(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                components::TruncatedText::aligned_path(
                                    path_label,
                                    path_alignment_group.clone(),
                                )
                                .text_sm()
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
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if !e.standard_click() {
                            return;
                        }
                        let target = DiffTarget::Commit {
                            commit_id: commit_id_for_click.clone(),
                            path: Some(path_for_click.clone()),
                        };
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
                    }))
                    .gitcomet_tooltip(theme, tooltip.clone());
                row = row.on_mouse_down(
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
                        this.activate_context_menu_invoker(invoker, cx);
                        this.open_popover_at(
                            PopoverKind::CommitFileMenu {
                                repo_id,
                                commit_id: commit_id_for_menu.clone(),
                                path: path_for_menu.clone(),
                            },
                            e.position,
                            window,
                            cx,
                        );
                        cx.notify();
                    }),
                );

                if selected {
                    row = row.bg(with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.16 } else { 0.10 },
                    ));
                }
                if context_menu_active {
                    row = row.bg(theme.colors.interaction.pressed_background);
                }

                row.into_any_element()
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
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let file_rows =
            this.cached_worktree_file_rows(repo_id, worktree_dirty_rev, &summary.path, files);
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
        let path_alignment_group = this
            .worktree_files_path_alignment_group
            .visible_rows(visible_signature);
        let worktree_path = summary.path.clone();
        let origin = gitcomet_state::model::ForeignDiffOrigin::Worktree {
            branch: summary.branch.clone(),
            detached: summary.detached,
        };

        range
            .filter_map(|ix| {
                files
                    .get(ix)
                    .zip(file_rows.get(ix))
                    .map(|(f, row)| (ix, f.clone(), row.label.clone(), row.visuals))
            })
            .map(|(ix, _f, path_label, visuals)| {
                let color = visuals.color(&theme);
                let selected = selected_ix_now == Some(ix);
                let tooltip = path_label.clone();
                let inputs_for_click = Arc::clone(&inputs);
                let worktree_path_for_click = worktree_path.clone();
                let origin_for_click = origin.clone();

                let mut row = div()
                    .id(("worktree_file", ix))
                    .debug_selector(move || format!("worktree_file_{}_{}", repo_id.0, ix))
                    .h(scaled_px(24.0))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .px(scaled_px(8.0))
                    .w_full()
                    .rounded(px(theme.radii.row))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(theme.colors.interaction.hover_background))
                    .active(move |s| s.bg(theme.colors.interaction.pressed_background))
                    .child(
                        div()
                            .w(scaled_px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(svg_icon(visuals.icon, color, scaled_px(14.0))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .line_height(scaled_px(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                components::TruncatedText::aligned_path(
                                    path_label,
                                    path_alignment_group.clone(),
                                )
                                .text_sm()
                                .render(cx),
                            ),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if !e.standard_click() {
                            return;
                        }
                        this.focus_diff_panel(window, cx);
                        this.store.dispatch(Msg::OpenInlineSubmoduleDiff {
                            repo_id,
                            origin: origin_for_click.clone(),
                            submodule_repo_path: worktree_path_for_click.clone(),
                            parent_submodule_path: worktree_path_for_click.clone(),
                            entries: inputs_for_click.entries.clone(),
                            selected_ix: ix,
                        });
                        cx.notify();
                    }))
                    .gitcomet_tooltip(theme, tooltip.clone());

                if selected {
                    row = row.bg(with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.16 } else { 0.10 },
                    ));
                }

                row.into_any_element()
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
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let repo_id = repo.id;
        let from = range_selection.from.clone();
        let to = range_selection.to.clone();
        let file_rows =
            this.cached_range_file_rows(repo_id, repo.history_state.range_files_rev, &files);
        let visible_signature = this.range_files_visible_signature(
            repo_id,
            repo.history_state.range_files_rev,
            &range,
            files.len(),
        );
        let path_alignment_group = this
            .range_files_path_alignment_group
            .visible_rows(visible_signature);

        range
            .filter_map(|ix| {
                files
                    .get(ix)
                    .zip(file_rows.get(ix))
                    .map(|(f, row)| (ix, f, row.label.clone(), row.visuals))
            })
            .map(|(ix, f, path_label, visuals)| {
                let icon = Some(visuals.icon);
                let color = visuals.color(&theme);
                let target = DiffTarget::CommitRange {
                    from_commit_id: from.clone(),
                    to_commit_id: to.clone(),
                    path: Some(f.path.clone()),
                };
                let selected = repo.diff_state.diff_target.as_ref() == Some(&target);
                let target_for_click = target.clone();
                let tooltip = path_label.clone();

                let mut row = div()
                    .id(("range_file", ix))
                    .debug_selector(move || format!("range_file_{}_{}", repo_id.0, ix))
                    .h(scaled_px(24.0))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .px(scaled_px(8.0))
                    .w_full()
                    .rounded(px(theme.radii.row))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(theme.colors.interaction.hover_background))
                    .active(move |s| s.bg(theme.colors.interaction.pressed_background))
                    .child(
                        div()
                            .w(scaled_px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(icon, |this, icon| {
                                this.child(svg_icon(icon, color, scaled_px(14.0)))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .line_height(scaled_px(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                components::TruncatedText::aligned_path(
                                    path_label,
                                    path_alignment_group.clone(),
                                )
                                .text_sm()
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
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if !e.standard_click() {
                            return;
                        }
                        let selected = this.active_repo().is_some_and(|repo| {
                            repo.id == repo_id
                                && repo.diff_state.diff_target.as_ref() == Some(&target_for_click)
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
                    }))
                    .gitcomet_tooltip(theme, tooltip.clone());

                if selected {
                    row = row.bg(with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.16 } else { 0.10 },
                    ));
                }

                row.into_any_element()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{
        Branch, Commit, CommitId, DiffTarget, LogPage, RemoteBranch, RepoSpec, Upstream,
        UpstreamDivergence, Worktree,
    };
    use gitcomet_core::services::{GitBackend, GitRepository, Result};
    use gitcomet_state::msg::{InternalMsg, Msg};
    use gitcomet_state::store::AppStore;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime};

    struct BlockingBackend;

    impl GitBackend for BlockingBackend {
        fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            loop {
                std::thread::park();
            }
        }
    }

    fn wait_until(
        cx: &mut gpui::VisualTestContext,
        description: &str,
        ready: impl Fn(&mut gpui::VisualTestContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
            if ready(cx) {
                return;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for {description}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn sync_view_for_tests(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    fn branch_row_index_for_name(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<GitCometView>,
        section: BranchSection,
        name: &str,
    ) -> usize {
        cx.update(|_window, app| {
            let sidebar_pane = view.read(app).sidebar_pane.clone();
            sidebar_pane.update(app, |pane, _cx| {
                let presentation = pane
                    .branch_sidebar_presentation_cached()
                    .expect("expected sidebar presentation");
                presentation
                    .rows
                    .iter()
                    .position(|row| {
                        matches!(
                            row,
                            BranchSidebarRow::Branch {
                                name: row_name,
                                section: row_section,
                                ..
                            } if *row_section == section && row_name.as_ref() == name
                        )
                    })
                    .unwrap_or_else(|| panic!("expected {section:?} branch row `{name}`"))
            })
        })
    }

    fn leak_selector(selector: String) -> &'static str {
        Box::leak(selector.into_boxed_str())
    }

    fn commit_id(id: &str) -> CommitId {
        CommitId(id.into())
    }

    /// The sequence number of the log walk the store has in flight, if any. A
    /// `LogLoaded` answering by hand has to carry it, or the reducer takes the
    /// reply for a superseded walk's and drops it.
    fn active_log_seq(store: &AppStore, repo_id: RepoId) -> gitcomet_state::model::LogLoadSeq {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.loads_in_flight.active_log_seq())
            .unwrap_or_default()
    }

    fn commit(id: &str) -> Commit {
        Commit {
            id: commit_id(id),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: id.into(),
            author: "author".into(),
            time: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn worktree_badge_colors_follow_open_and_menu_state() {
        let palette = worktree_badge_palette(AppTheme::gitcomet_dark());

        let closed = worktree_badge_colors(palette, false, false);
        assert_eq!(closed.border, palette.border);
        assert_eq!(closed.text, palette.text);

        let open = worktree_badge_colors(palette, true, false);
        assert_eq!(open.border, palette.open_border);
        assert_eq!(open.text, palette.open_text);

        let menu_active = worktree_badge_colors(palette, true, true);
        assert_eq!(menu_active.border, palette.active_border);
        assert_eq!(menu_active.text, palette.active_text);
    }

    #[test]
    fn worktree_origin_label_pairs_branch_with_folder() {
        assert_eq!(
            worktree_origin_label(Some("dev"), false, std::path::Path::new("/home/u/GitComet")),
            "dev · GitComet"
        );
    }

    #[test]
    fn worktree_origin_label_names_a_detached_worktree_by_its_folder() {
        assert_eq!(
            worktree_origin_label(None, true, std::path::Path::new("/home/u/GitComet2")),
            "(detached) · GitComet2"
        );
    }

    #[test]
    fn worktree_origin_label_falls_back_when_one_half_is_missing() {
        // No branch and no detached marker: the folder alone identifies it.
        assert_eq!(
            worktree_origin_label(None, false, std::path::Path::new("/home/u/GitComet3")),
            "GitComet3"
        );
        // A root path has no final component to name.
        assert_eq!(
            worktree_origin_label(Some("main"), false, std::path::Path::new("/")),
            "main"
        );
    }

    #[test]
    fn worktree_branch_badge_label_prefers_open_repo_head_branch() {
        let listed: SharedString = "feature/listed".into();
        let mut open_repo = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_repo.head_branch = Loadable::Ready("feature/live".to_string());

        let label = worktree_branch_badge_label(Some(&listed), false, Some(&open_repo))
            .expect("expected live branch badge label");
        assert_eq!(label.as_ref(), "feature/live");
    }

    #[test]
    fn worktree_branch_badge_label_reports_detached_open_repo() {
        let listed: SharedString = "feature/listed".into();
        let mut open_repo = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_repo.head_branch = Loadable::Ready("HEAD".to_string());
        open_repo.detached_head_commit = Some(commit_id("detached"));

        let label = worktree_branch_badge_label(Some(&listed), false, Some(&open_repo))
            .expect("expected detached branch badge label");
        assert_eq!(label.as_ref(), "(detached)");
    }

    #[test]
    fn listed_workspace_paths_by_branch_includes_closed_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo"),
                head: None,
                branch: Some("main".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: Some("feature".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-detached"),
                head: None,
                branch: None,
                detached: true,
            },
        ]));

        let paths = listed_workspace_paths_by_branch(&repo);

        assert_eq!(
            paths.get("feature"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
        assert!(!paths.contains_key("main"));
        assert!(!paths.contains_key("repo-detached"));
    }

    #[test]
    fn listed_workspace_paths_by_branch_prefers_first_branch_match() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-a"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-b"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
        ]));

        let paths = listed_workspace_paths_by_branch(&repo);

        assert_eq!(
            paths.get("feature/shared"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature-a"))
        );
    }

    #[test]
    fn listed_workspace_paths_returns_empty_when_worktrees_loading() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Loading;

        let paths = listed_workspace_paths_by_branch(&repo);

        assert!(paths.is_empty());
    }

    #[test]
    fn listed_workspace_paths_returns_empty_when_worktrees_not_loaded() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::NotLoaded;

        let paths = listed_workspace_paths_by_branch(&repo);

        assert!(paths.is_empty());
    }

    #[test]
    fn listed_workspace_paths_returns_empty_when_worktrees_error() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Error("failed to load".into());

        let paths = listed_workspace_paths_by_branch(&repo);

        assert!(paths.is_empty());
    }

    #[test]
    fn listed_workspace_paths_returns_empty_when_no_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![]));

        let paths = listed_workspace_paths_by_branch(&repo);

        assert!(paths.is_empty());
    }

    #[test]
    fn active_workspace_paths_by_branch_only_includes_open_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo"),
                head: None,
                branch: Some("main".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: Some("feature".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-detached"),
                head: None,
                branch: None,
                detached: true,
            },
        ]));

        let mut open_main = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        open_main.head_branch = Loadable::Ready("main".to_string());
        let mut open_feature = RepoState::new_opening(
            RepoId(3),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_feature.head_branch = Loadable::Ready("feature".to_string());

        let active = active_workspace_paths_by_branch(&repo, &[open_main, open_feature]);

        assert_eq!(
            active.get("main"),
            Some(&std::path::PathBuf::from("/tmp/repo"))
        );
        assert_eq!(
            active.get("feature"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
        assert!(!active.contains_key("repo-detached"));
    }

    #[test]
    fn active_workspace_paths_by_branch_skips_closed_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));

        let active = active_workspace_paths_by_branch(&repo, &[]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_by_branch_uses_open_repo_head_branch_for_live_updates() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }]));

        let mut open_worktree = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_worktree.head_branch = Loadable::Ready("feature/new".to_string());
        open_worktree.head_branch_rev = 1;

        let active = active_workspace_paths_by_branch(&repo, &[open_worktree]);

        assert!(!active.contains_key("feature/old"));
        assert_eq!(
            active.get("feature/new"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn active_workspace_paths_by_branch_falls_back_to_listed_branch_while_head_is_loading() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/listed".to_string()),
            detached: false,
        }]));

        let open_worktree = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );

        let active = active_workspace_paths_by_branch(&repo, &[open_worktree]);

        assert_eq!(
            active.get("feature/listed"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn active_workspace_paths_by_branch_hides_detached_open_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }]));

        let mut open_worktree = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_worktree.head_branch = Loadable::Ready("HEAD".to_string());
        open_worktree.head_branch_rev = 1;
        open_worktree.detached_head_commit = Some(CommitId("deadbeef".into()));

        let active = active_workspace_paths_by_branch(&repo, &[open_worktree]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_by_branch_keeps_first_listed_workspace_for_branch() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-a"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-b"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
        ]));

        let mut open_first = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature-a"),
            },
        );
        open_first.head_branch = Loadable::Ready("feature/shared".to_string());

        let mut open_second = RepoState::new_opening(
            RepoId(3),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature-b"),
            },
        );
        open_second.head_branch = Loadable::Ready("feature/shared".to_string());

        let active = active_workspace_paths_by_branch(&repo, &[open_first, open_second]);

        assert_eq!(
            active.get("feature/shared"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature-a"))
        );
    }

    #[test]
    fn active_workspace_paths_returns_empty_when_worktrees_loading() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Loading;

        let open_repo = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );

        let active = active_workspace_paths_by_branch(&repo, &[open_repo]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_returns_empty_when_worktrees_not_loaded() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::NotLoaded;

        let open_repo = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );

        let active = active_workspace_paths_by_branch(&repo, &[open_repo]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_returns_empty_when_worktrees_error() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Error("failed to load".into());

        let open_repo = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );

        let active = active_workspace_paths_by_branch(&repo, &[open_repo]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_returns_empty_when_worktrees_empty() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![]));

        let open_repo = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );

        let active = active_workspace_paths_by_branch(&repo, &[open_repo]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_matches_open_repo_by_workdir_path() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/listed".to_string()),
            detached: false,
        }]));

        let mut open_repo = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_repo.head_branch = Loadable::Ready("different-branch".to_string());

        let active = active_workspace_paths_by_branch(&repo, &[open_repo]);

        assert_eq!(
            active.get("different-branch"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
        assert!(!active.contains_key("feature/listed"));
    }

    #[test]
    fn branch_workspace_badge_path_prefers_listed_workspace_and_falls_back_to_active() {
        assert_eq!(
            branch_workspace_badge_path(
                Some(std::path::Path::new("/tmp/repo-feature-listed")),
                Some(std::path::Path::new("/tmp/repo-feature-open")),
            ),
            Some(std::path::PathBuf::from("/tmp/repo-feature-listed"))
        );
        assert_eq!(
            branch_workspace_badge_path(None, Some(std::path::Path::new("/tmp/repo-feature-open")),),
            Some(std::path::PathBuf::from("/tmp/repo-feature-open"))
        );
    }

    #[test]
    fn local_branch_double_click_checks_out_when_no_workspace_is_open() {
        assert_eq!(
            local_branch_double_click_action("feature/workspace", None),
            LocalBranchDoubleClickAction::CheckoutBranch {
                name: "feature/workspace".to_string(),
            }
        );
    }

    #[test]
    fn local_branch_double_click_opens_workspace_when_branch_has_active_workspace() {
        assert_eq!(
            local_branch_double_click_action(
                "feature/workspace",
                Some(std::path::Path::new("/tmp/repo-feature"))
            ),
            LocalBranchDoubleClickAction::OpenWorkspace {
                path: std::path::PathBuf::from("/tmp/repo-feature"),
            }
        );
    }

    #[test]
    fn branch_row_selection_requires_matching_clicked_branch_identity() {
        let target = commit_id("shared-tip");
        let selected_branch = SelectedBranch {
            repo_id: RepoId(1),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert!(branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            Some(&target),
            Some(&target)
        ));
        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Remote,
            "origin/main",
            Some(&target),
            Some(&target)
        ));
    }

    #[test]
    fn branch_row_selection_requires_matching_selected_commit() {
        let target = commit_id("main-tip");
        let other = commit_id("other-tip");
        let selected_branch = SelectedBranch {
            repo_id: RepoId(1),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            Some(&other),
            Some(&target)
        ));
        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            None,
            Some(&target)
        ));
    }

    #[test]
    fn branch_row_selection_requires_resolved_selected_branch_tip() {
        let target = commit_id("main-tip");
        let selected_branch = SelectedBranch {
            repo_id: RepoId(1),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            Some(&target),
            None
        ));
    }

    #[test]
    fn branch_click_history_reveal_target_switches_head_local_branch_to_full_reachable() {
        let target = commit_id("main-tip");
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.history_state.history_scope = LogScope::CurrentBranch;
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "main".to_string(),
            target: target.clone(),
            upstream: None,
            divergence: None,
        }]));

        assert_eq!(
            branch_click_history_reveal_target(&repo, BranchSection::Local, "main", true),
            Some(BranchHistoryRevealTarget {
                commit_id: target,
                fallback_scope: Some(LogScope::FullReachable),
            })
        );
    }

    #[test]
    fn branch_click_history_reveal_target_switches_non_head_local_branch_to_all_branches() {
        let target = commit_id("feature-tip");
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.history_state.history_scope = LogScope::CurrentBranch;
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "feature".to_string(),
            target: target.clone(),
            upstream: None,
            divergence: None,
        }]));

        assert_eq!(
            branch_click_history_reveal_target(&repo, BranchSection::Local, "feature", false),
            Some(BranchHistoryRevealTarget {
                commit_id: target,
                fallback_scope: Some(LogScope::AllBranches),
            })
        );
    }

    #[test]
    fn branch_click_history_reveal_target_switches_remote_branch_to_all_branches() {
        let target = commit_id("origin-feature-tip");
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.history_state.history_scope = LogScope::CurrentBranch;
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "feature/topic".to_string(),
            target: target.clone(),
        }]));

        assert_eq!(
            branch_click_history_reveal_target(
                &repo,
                BranchSection::Remote,
                "origin/feature/topic",
                false,
            ),
            Some(BranchHistoryRevealTarget {
                commit_id: target,
                fallback_scope: Some(LogScope::AllBranches),
            })
        );
    }

    #[gpui::test]
    fn branch_badges_are_static_and_worktree_badge_remains_interactive(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let store_for_assert = store.clone();
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        crate::view::test_support::redraw(cx);
        store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
        wait_until(cx, "opened repo placeholder", |_cx| {
            let snapshot = store_for_assert.snapshot();
            snapshot.active_repo == Some(repo_id)
                && snapshot.repos.iter().any(|repo| repo.id == repo_id)
        });
        sync_view_for_tests(cx, &view);

        store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok("main".to_string()),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![
                Branch {
                    name: "main".to_string(),
                    target: commit_id("main-tip"),
                    upstream: Some(Upstream {
                        remote: "origin".to_string(),
                        branch: "main".to_string(),
                    }),
                    divergence: None,
                },
                Branch {
                    name: "feature".to_string(),
                    target: commit_id("feature-tip"),
                    upstream: Some(Upstream {
                        remote: "origin".to_string(),
                        branch: "feature".to_string(),
                    }),
                    divergence: Some(UpstreamDivergence {
                        ahead: 3,
                        behind: 2,
                    }),
                },
            ]),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::RemoteBranchesLoaded {
            repo_id,
            result: Ok(vec![
                RemoteBranch {
                    remote: "origin".to_string(),
                    name: "main".to_string(),
                    target: commit_id("origin-main-tip"),
                },
                RemoteBranch {
                    remote: "origin".to_string(),
                    name: "feature".to_string(),
                    target: commit_id("origin-feature-tip"),
                },
            ]),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::WorktreesLoaded {
            repo_id,
            result: Ok(vec![
                Worktree {
                    path: PathBuf::from("/tmp/repo"),
                    head: None,
                    branch: Some("main".to_string()),
                    detached: false,
                },
                Worktree {
                    path: PathBuf::from("/tmp/repo-feature"),
                    head: None,
                    branch: Some("feature".to_string()),
                    detached: false,
                },
            ]),
        }));
        wait_until(cx, "sidebar badges loaded", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            matches!(repo.head_branch, Loadable::Ready(ref head) if head == "main")
                && matches!(repo.branches, Loadable::Ready(_))
                && matches!(repo.remote_branches, Loadable::Ready(_))
                && matches!(repo.worktrees, Loadable::Ready(_))
        });
        sync_view_for_tests(cx, &view);

        let feature_ix = branch_row_index_for_name(cx, &view, BranchSection::Local, "feature");
        let upstream_ix =
            branch_row_index_for_name(cx, &view, BranchSection::Remote, "origin/main");

        let feature_row_selector =
            leak_selector(format!("branch_row_{}_{}", repo_id.0, feature_ix));
        let feature_badge_selector = leak_selector(format!("branch_workspace_badge_{feature_ix}"));
        let feature_pull_badge_selector = leak_selector(format!("branch_pull_badge_{feature_ix}"));
        let feature_push_badge_selector = leak_selector(format!("branch_push_badge_{feature_ix}"));
        let feature_menu_selector = leak_selector(format!(
            "branch_menu_indicator_{}_{}",
            repo_id.0, feature_ix
        ));
        assert!(
            cx.debug_bounds(feature_menu_selector).is_none(),
            "expected branch hamburger menu indicator to be removed"
        );
        let feature_badge_before = cx
            .debug_bounds(feature_badge_selector)
            .expect("expected worktree badge before hover");
        let feature_pull_badge_before = cx
            .debug_bounds(feature_pull_badge_selector)
            .expect("expected pull count badge before hover");
        let feature_push_badge_before = cx
            .debug_bounds(feature_push_badge_selector)
            .expect("expected push count badge before hover");
        let feature_row_bounds = cx
            .debug_bounds(feature_row_selector)
            .expect("expected feature branch row");
        let feature_row_center = feature_row_bounds.center();
        let feature_dots_selector = leak_selector(format!("branch_dots_{feature_ix}"));
        assert!(
            cx.debug_bounds(feature_dots_selector).is_none(),
            "expected the trailing `⋮` slot to be gone from branch rows"
        );
        // The row keeps only the trailing padding to the right of its badges,
        // so the worktree badge lands on the row's right edge.
        assert!(
            (feature_row_bounds.right() - feature_badge_before.right() - px(4.0)).abs() <= px(1.0),
            "expected the worktree badge to sit one trailing pad off the row's right edge, \
             row right {:?} badge right {:?}",
            feature_row_bounds.right(),
            feature_badge_before.right()
        );
        cx.simulate_mouse_move(feature_row_center, None, gpui::Modifiers::default());
        crate::view::test_support::redraw(cx);
        let feature_badge_after = cx
            .debug_bounds(feature_badge_selector)
            .expect("expected worktree badge after hover");
        let feature_pull_badge_after = cx
            .debug_bounds(feature_pull_badge_selector)
            .expect("expected pull count badge after hover");
        let feature_push_badge_after = cx
            .debug_bounds(feature_push_badge_selector)
            .expect("expected push count badge after hover");
        // Hover reveals nothing in the trailing run any more, so the badges
        // keep the exact geometry they had at rest.
        assert!(
            cx.debug_bounds(feature_dots_selector).is_none(),
            "expected no `⋮` slot to appear on row hover"
        );
        assert_eq!(
            feature_badge_before.left(),
            feature_badge_after.left(),
            "expected the worktree badge to stay fixed on row hover"
        );
        assert_eq!(
            feature_badge_before.right(),
            feature_badge_after.right(),
            "expected the worktree badge to stay fixed on row hover"
        );
        assert_eq!(
            feature_pull_badge_before.left(),
            feature_pull_badge_after.left(),
            "expected the pull badge to stay fixed on row hover"
        );
        assert_eq!(
            feature_push_badge_before.left(),
            feature_push_badge_after.left(),
            "expected the push badge to stay fixed on row hover"
        );
        // Right-click over the label (near the row's leading edge) rather than
        // the center: the trailing area holds the worktree badge, which opens
        // its own menu.
        let feature_row_label_point =
            gpui::point(feature_row_bounds.left() + px(48.0), feature_row_center.y);
        cx.simulate_mouse_down(
            feature_row_label_point,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        crate::view::test_support::redraw(cx);
        let popover_kind = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert!(
            matches!(
                popover_kind,
                Some(PopoverKind::BranchMenu {
                    repo_id: opened_repo_id,
                    section: BranchSection::Local,
                    ref name,
                    ..
                }) if opened_repo_id == repo_id && name == "feature"
            ),
            "expected feature branch right-click to open the branch menu"
        );
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.close_popover(cx);
                });
            });
        });
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);

        let feature_badge_center = cx
            .debug_bounds(feature_badge_selector)
            .expect("expected worktree badge before badge click")
            .center();
        cx.simulate_mouse_move(feature_badge_center, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(
            feature_badge_center,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            feature_badge_center,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        crate::view::test_support::redraw(cx);
        let popover_kind = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert!(
            matches!(
                popover_kind,
                Some(PopoverKind::Repo {
                    repo_id: opened_repo_id,
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu {
                        ref path,
                        branch: Some(ref branch),
                    }),
                }) if opened_repo_id == repo_id
                    && path == &PathBuf::from("/tmp/repo-feature")
                    && branch == "feature"
            ),
            "expected worktree badge click to open the worktree menu"
        );
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.close_popover(cx);
                });
            });
        });
        cx.run_until_parked();

        cx.simulate_mouse_down(
            feature_badge_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        crate::view::test_support::redraw(cx);
        let popover_kind = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert!(
            matches!(
                popover_kind,
                Some(PopoverKind::Repo {
                    repo_id: opened_repo_id,
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu {
                        ref path,
                        branch: Some(ref branch),
                    }),
                }) if opened_repo_id == repo_id
                    && path == &PathBuf::from("/tmp/repo-feature")
                    && branch == "feature"
            ),
            "expected worktree badge right-click to open the worktree menu"
        );
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.close_popover(cx);
                });
            });
        });
        cx.run_until_parked();

        let upstream_row_selector =
            leak_selector(format!("branch_row_{}_{}", repo_id.0, upstream_ix));
        let upstream_badge_selector = leak_selector(format!("branch_upstream_badge_{upstream_ix}"));
        let upstream_menu_selector = leak_selector(format!(
            "branch_menu_indicator_{}_{}",
            repo_id.0, upstream_ix
        ));
        assert!(
            cx.debug_bounds(upstream_menu_selector).is_none(),
            "expected upstream branch hamburger menu indicator to be removed"
        );
        let upstream_badge_before = cx
            .debug_bounds(upstream_badge_selector)
            .expect("expected upstream badge before hover");
        let upstream_row_center = cx
            .debug_bounds(upstream_row_selector)
            .expect("expected upstream branch row")
            .center();
        cx.simulate_mouse_move(upstream_row_center, None, gpui::Modifiers::default());
        crate::view::test_support::redraw(cx);
        let upstream_badge_after = cx
            .debug_bounds(upstream_badge_selector)
            .expect("expected upstream badge after hover");
        assert_eq!(
            upstream_badge_before.left(),
            upstream_badge_after.left(),
            "expected the upstream badge to stay fixed on row hover"
        );
        assert_eq!(
            upstream_badge_before.right(),
            upstream_badge_after.right(),
            "expected the upstream badge to stay fixed on row hover"
        );
    }

    #[gpui::test]
    fn branch_reveal_marks_the_branch_chip_on_the_revealed_history_row(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let store_for_assert = store.clone();
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let feature_tip = commit_id("feature-tip");
        let initial_scope = LogScope::default();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
        wait_until(cx, "opened repo placeholder", |_cx| {
            let snapshot = store_for_assert.snapshot();
            snapshot.active_repo == Some(repo_id)
                && snapshot.repos.iter().any(|repo| repo.id == repo_id)
        });

        store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok("main".to_string()),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![
                Branch {
                    name: "main".to_string(),
                    target: commit_id("main-tip"),
                    upstream: None,
                    divergence: None,
                },
                Branch {
                    name: "feature".to_string(),
                    target: feature_tip.clone(),
                    upstream: None,
                    divergence: None,
                },
            ]),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::LogLoaded {
            repo_id,
            seq: active_log_seq(&store_for_assert, repo_id),
            scope: initial_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![commit("feature-tip"), commit("main-tip")],
                next_cursor: None,
            }),
        }));
        wait_until(cx, "sidebar repo data", |cx| {
            sync_view_for_tests(cx, &view);
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            matches!(repo.branches, Loadable::Ready(_)) && matches!(repo.log, Loadable::Ready(_))
        });

        let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
        cx.update(|window, app| {
            sidebar_pane.update(app, |pane, cx| {
                pane.reveal_branch_commit_in_history(
                    repo_id,
                    BranchSection::Local,
                    "feature",
                    feature_tip.clone(),
                    None,
                    cx,
                );
            });
            let _ = window.draw(app);
        });

        wait_until(cx, "revealed branch tip selected", |cx| {
            sync_view_for_tests(cx, &view);
            let snapshot = store_for_assert.snapshot();
            snapshot
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .is_some_and(|repo| {
                    repo.history_state.selected_commit.as_ref() == Some(&feature_tip)
                })
        });

        let marked = cx.update(|_window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view
                .read(app)
                .selected_branch_for_history_row(repo_id, true)
        });
        assert_eq!(
            marked,
            Some(SelectedHistoryBranch {
                section: BranchSection::Local,
                name: "feature".into(),
            }),
            "the revealed row should mark the clicked branch's chip as selected"
        );
    }

    #[gpui::test]
    fn branch_reveal_routes_through_main_pane_and_selects_commit(cx: &mut gpui::TestAppContext) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let store_for_assert = store.clone();
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let sync_view_from_store = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, app| {
                view.update(app, |this, cx| {
                    crate::view::test_support::sync_store_snapshot(this, cx)
                });
                window.refresh();
                let _ = window.draw(app);
            });
        };

        let repo_id = RepoId(1);
        let target = commit_id("main-tip");
        let initial_scope = LogScope::default();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
        wait_until(cx, "opened repo placeholder", |_cx| {
            let snapshot = store_for_assert.snapshot();
            snapshot.active_repo == Some(repo_id)
                && snapshot.repos.iter().any(|repo| repo.id == repo_id)
        });
        sync_view_from_store(cx);

        store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok("main".to_string()),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![Branch {
                name: "main".to_string(),
                target: target.clone(),
                upstream: None,
                divergence: None,
            }]),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::LogLoaded {
            repo_id,
            seq: active_log_seq(&store_for_assert, repo_id),
            scope: initial_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![commit("main-tip")],
                next_cursor: None,
            }),
        }));
        store_for_assert.dispatch(Msg::SelectDiff {
            repo_id,
            target: DiffTarget::Commit {
                commit_id: commit_id("previous"),
                path: None,
            },
        });
        wait_until(cx, "sidebar repo data", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            matches!(repo.head_branch, Loadable::Ready(ref head) if head == "main")
                && matches!(repo.branches, Loadable::Ready(_))
                && matches!(repo.log, Loadable::Ready(_))
                && repo.diff_state.diff_target.is_some()
        });
        sync_view_from_store(cx);

        wait_until(cx, "history view active repo", |cx| {
            sync_view_for_tests(cx, &view);
            cx.update(|_window, app| {
                let (sidebar_pane, main_pane) = {
                    let root = view.read(app);
                    (root.sidebar_pane.clone(), root.main_pane.clone())
                };
                let history_view = main_pane.read(app).history_view.clone();

                sidebar_pane.read(app).active_repo_id() == Some(repo_id)
                    && main_pane.read(app).active_repo_id() == Some(repo_id)
                    && history_view.read(app).active_repo_id() == Some(repo_id)
            })
        });

        sync_view_for_tests(cx, &view);
        let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
        cx.update(|window, app| {
            sidebar_pane.update(app, |pane, cx| {
                pane.reveal_branch_commit_in_history(
                    repo_id,
                    BranchSection::Local,
                    "main",
                    target.clone(),
                    None,
                    cx,
                );
            });
            let _ = window.draw(app);
        });

        wait_until(cx, "branch reveal store state", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            repo.diff_state.diff_target.is_none()
                && repo.history_state.history_scope == initial_scope
                && repo.history_state.selected_commit.as_ref() == Some(&target)
        });
    }

    #[gpui::test]
    fn branch_reveal_closes_open_history_refs_hover_without_reentrant_root_update(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let store_for_assert = store.clone();
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let sync_view_from_store = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, app| {
                view.update(app, |this, cx| {
                    crate::view::test_support::sync_store_snapshot(this, cx)
                });
                window.refresh();
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        };

        let repo_id = RepoId(1);
        let target = commit_id("main-tip");
        let initial_scope = LogScope::default();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
        wait_until(cx, "opened repo placeholder", |_cx| {
            let snapshot = store_for_assert.snapshot();
            snapshot.active_repo == Some(repo_id)
                && snapshot.repos.iter().any(|repo| repo.id == repo_id)
        });
        sync_view_from_store(cx);

        store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok("main".to_string()),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![Branch {
                name: "main".to_string(),
                target: target.clone(),
                upstream: None,
                divergence: None,
            }]),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::LogLoaded {
            repo_id,
            seq: active_log_seq(&store_for_assert, repo_id),
            scope: initial_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![commit("main-tip")],
                next_cursor: None,
            }),
        }));
        wait_until(cx, "sidebar repo data", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            matches!(repo.head_branch, Loadable::Ready(ref head) if head == "main")
                && matches!(repo.branches, Loadable::Ready(_))
                && matches!(repo.log, Loadable::Ready(_))
        });
        sync_view_from_store(cx);

        wait_until(cx, "history row rendered", |cx| {
            sync_view_for_tests(cx, &view);
            cx.debug_bounds("history_row_0").is_some()
        });

        let history_row_bounds = cx
            .debug_bounds("history_row_0")
            .expect("history row should be rendered");
        let hover_items: Arc<[HistoryRefListItem]> = vec![HistoryRefListItem {
            text: HistoryTextVm::new("main".into()),
            kind: HistoryRefListItemKind::LocalBranch {
                name: "main".to_string(),
            },
        }]
        .into();
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.show_history_refs_hover(
                    repo_id,
                    target.clone(),
                    history_row_bounds,
                    hover_items.clone(),
                    history_row_bounds.center(),
                    window,
                    cx,
                );
            });
            let _ = window.draw(app);
        });
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.update(|_window, app| {
            assert!(crate::view::test_support::history_refs_hover_is_open(
                view.read(app),
                app
            ));
        });

        let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
        cx.update(|window, app| {
            sidebar_pane.update(app, |pane, cx| {
                pane.reveal_branch_commit_in_history(
                    repo_id,
                    BranchSection::Local,
                    "main",
                    target.clone(),
                    None,
                    cx,
                );
            });
            let _ = window.draw(app);
        });

        wait_until(cx, "branch reveal store state and hover closure", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            repo.history_state.history_scope == initial_scope
                && repo.history_state.selected_commit.as_ref() == Some(&target)
                && _cx.update(|_window, app| {
                    !crate::view::test_support::history_refs_hover_is_open(view.read(app), app)
                })
        });
    }
}
