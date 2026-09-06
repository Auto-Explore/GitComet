use super::diff_canvas;
use super::diff_text::*;
use super::history_canvas;
use super::*;
use crate::view::caches::HistoryListRow;
use palette::IntoColor;

use crate::view::markdown_preview::{
    MarkdownAlertKind, MarkdownChangeHint, MarkdownInlineImage, MarkdownInlineStyle,
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind, MarkdownPreviewVisualRow,
    MarkdownPreviewWrapPlan,
};
use crate::view::panes::main::diff_search::DiffSearchMatcher;
use crate::view::perf::{self, ViewPerfRenderLane, ViewPerfSpan};
use gitcomet_state::msg::CommitSelectMode;
use rustc_hash::FxHasher;

impl HistoryView {
    pub(in super::super) fn render_history_table_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let (_, worktree_counts) = this.ensure_history_worktree_summary_cache();
        let plan = this.ensure_history_list_plan();
        let stash_ids = this.ensure_history_stash_ids_cache();
        // One lane keeps full colour; the rest wash out. Resolved once here rather
        // than per row -- it is a scan of the page behind a memo.
        let selected_lane = this.history_selected_lane(plan.show_working_tree_summary_row());

        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let primary_selection =
            super::history_primary_selection(repo, plan.show_working_tree_summary_row());
        let show_graph_color_marker =
            history_scope_shows_graph_color_marker(repo.history_state.history_scope);

        let theme = this.theme;
        let col_branch = this.history_col_branch;
        let col_graph = this.history_col_graph;
        let col_author = this.history_col_author;
        let col_date = this.history_col_date;
        let col_sha = this.history_col_sha;
        let ui_scale = this.ui_scale();
        let (show_graph, show_author, show_date, show_sha) = this.history_visible_columns();
        let display_key = HistoryDisplayKey::new(
            this.date_time_format,
            this.timezone,
            this.show_timezone,
            this.history_relative_dates,
        );

        let page = Self::display_log_page_for_repo(repo);
        let cache = this
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo.id);
        let worktree_node_color_ix =
            history_worktree_node_color_ix(cache.map(|cache| cache.base.graph_rows.as_ref()));

        let worktree_dirty = match &repo.worktree_dirty {
            Loadable::Ready(dirty) => Some(Arc::clone(dirty)),
            _ => None,
        };
        range
            .filter_map(|list_ix| {
                let row = plan.row_at(list_ix)?;
                if let HistoryListRow::WorktreeUncommitted {
                    visible_ix,
                    worktree_ix,
                } = row
                {
                    let cache = cache?;
                    let summary = worktree_dirty.as_ref()?.get(worktree_ix)?;
                    // The row shows the lanes of the commit it sits on top of,
                    // so it needs that row's paint data.
                    let graph_row = cache.base.graph_rows.get(visible_ix)?;
                    // Whatever sits directly above draws a connector down into
                    // this row; carry it through so the lane is not broken.
                    let connect_from_top_col =
                        super::history_graph_paint::worktree_band_connect_from_top_col(
                            &plan,
                            cache.base.graph_rows.as_ref(),
                            worktree_dirty
                                .as_ref()
                                .map_or(&[][..], |dirty| dirty.as_slice()),
                            list_ix,
                        );
                    return Some(worktree_uncommitted_history_row(
                        theme,
                        ui_scale,
                        col_branch,
                        col_graph,
                        col_author,
                        col_date,
                        col_sha,
                        show_graph,
                        show_author,
                        show_date,
                        show_sha,
                        graph_row,
                        visible_ix,
                        connect_from_top_col,
                        selected_lane,
                        show_graph_color_marker,
                        repo.id,
                        list_ix,
                        matches!(
                            &primary_selection,
                            Some(super::HistoryPrimarySelection::Worktree(path))
                                if path == &summary.path
                        ),
                        (summary.added, summary.modified, summary.deleted),
                        summary,
                        cx,
                    ));
                }

                if matches!(row, HistoryListRow::WorkingTreeSummary) {
                    let selected = matches!(
                        &primary_selection,
                        Some(super::HistoryPrimarySelection::WorkingTree)
                    );
                    return Some(working_tree_summary_history_row(
                        theme,
                        ui_scale,
                        col_branch,
                        col_graph,
                        col_author,
                        col_date,
                        col_sha,
                        show_graph,
                        show_author,
                        show_date,
                        show_sha,
                        worktree_node_color_ix,
                        selected_lane,
                        show_graph_color_marker,
                        repo.id,
                        selected,
                        worktree_counts,
                        cx,
                    ));
                }

                let HistoryListRow::Commit { visible_ix } = row else {
                    return None;
                };

                let page = page.as_deref()?;
                let cache = cache?;

                let commit_ix = cache.base.visible_indices.get(visible_ix)?;
                let commit = page.commits.get(commit_ix)?;
                cache.base.graph_rows.get(visible_ix)?;
                let base_row_vm = cache.base.row_vms.get(visible_ix)?;
                let decoration_row_vm = cache.decorations.row_vms.get(visible_ix)?;
                // A synthetic row above connects down into this commit, so this
                // row draws the matching stub upwards even when its lane is born
                // here. Same resolution the bands use, so the two never disagree
                // about where the stub lands.
                let connect_from_top_col =
                    super::history_graph_paint::worktree_band_connect_from_top_col(
                        &plan,
                        cache.base.graph_rows.as_ref(),
                        worktree_dirty
                            .as_ref()
                            .map_or(&[][..], |dirty| dirty.as_slice()),
                        list_ix,
                    );
                let selected = matches!(
                    &primary_selection,
                    Some(super::HistoryPrimarySelection::Commit(commit_id))
                        if commit_id == &commit.id
                ) || repo.history_state.multi_selection.is_multi()
                    && repo.history_state.multi_selection.contains(&commit.id);
                let selected_branch = this.selected_branch_for_history_row(repo.id, selected);
                let is_stash_node = base_row_vm.is_stash
                    || stash_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&commit.id));
                let when = base_row_vm.when.resolve(display_key);
                let short_sha = base_row_vm.short_sha.resolve();

                let lane_branch_name = decoration_row_vm
                    .lane_branch
                    .and_then(|ix| cache.decorations.branch_names.get(usize::from(ix)))
                    .cloned();

                Some(history_table_row(
                    theme,
                    ui_scale,
                    col_branch,
                    col_graph,
                    col_author,
                    col_date,
                    col_sha,
                    show_graph,
                    show_author,
                    show_date,
                    show_sha,
                    show_graph_color_marker,
                    list_ix,
                    repo.id,
                    commit,
                    Arc::clone(&cache.base.graph_rows),
                    visible_ix,
                    connect_from_top_col,
                    Arc::clone(&decoration_row_vm.tag_names),
                    Arc::clone(&decoration_row_vm.branch_chips),
                    Arc::clone(&decoration_row_vm.ref_items),
                    selected_branch,
                    selected_lane,
                    lane_branch_name,
                    base_row_vm.author.clone(),
                    base_row_vm.summary.clone(),
                    when,
                    short_sha,
                    selected,
                    base_row_vm.is_head,
                    is_stash_node,
                    this.active_context_menu_invoker.as_ref(),
                    cx,
                ))
            })
            .collect()
    }
}

const HISTORY_ROW_HEIGHT_PX: f32 = 28.0;
/// Widest a worktree row's badge may grow before its branch label truncates.
/// Matches the sidebar's branch-row worktree pill.
const HISTORY_WORKTREE_BADGE_MAX_W_PX: f32 = 200.0;
/// Matches the history table's ref chips so the badge sits on the same rhythm.
const HISTORY_WORKTREE_BADGE_HEIGHT_PX: f32 = 18.0;

fn history_worktree_node_color_ix(
    graph_rows: Option<&[history_graph::GraphRow]>,
) -> history_graph::LaneColorIx {
    graph_rows
        .and_then(|rows| rows.first())
        .and_then(|row| {
            // Column 0 can be a hole, whose `color_ix` is a real palette index
            // rather than a lane's colour.
            row.lanes_now
                .first()
                .filter(|lane| lane.is_active())
                .map(|lane| lane.color_ix)
        })
        .unwrap_or(0)
}

/// The lane-coloured border down the left edge of a message cell, matching the
/// one the commit rows paint on their canvas.
///
/// Absolutely positioned so the label keeps the same left offset it has on a
/// commit row — a flow child would push the text over by the border's width.
fn history_message_border(ui_scale: ui_scale::UiScale, color: gpui::Rgba) -> impl IntoElement {
    let border_w = ui_scale.px(HISTORY_MESSAGE_BORDER_W_PX);
    let inset_y = ui_scale.px(HISTORY_MESSAGE_BORDER_INSET_Y_PX);
    div()
        .absolute()
        .left_0()
        .top(inset_y)
        .bottom(inset_y)
        .w(border_w)
        .rounded(border_w * 0.5)
        .bg(color)
}

fn history_row_height(ui_scale: ui_scale::UiScale) -> Pixels {
    ui_scale.px(HISTORY_ROW_HEIGHT_PX)
}

fn history_scope_shows_graph_color_marker(scope: gitcomet_core::domain::LogScope) -> bool {
    !matches!(scope, gitcomet_core::domain::LogScope::FirstParent)
}

#[allow(clippy::too_many_arguments)]
fn history_table_row(
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_graph: bool,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    show_graph_color_marker: bool,
    ix: usize,
    repo_id: RepoId,
    commit: &Commit,
    graph_rows: Arc<[history_graph::GraphRow]>,
    graph_row_ix: usize,
    connect_from_top_col: Option<usize>,
    tag_names: Arc<[HistoryTextVm]>,
    branch_chips: Arc<[HistoryBranchChipVm]>,
    ref_items: Arc<[HistoryRefListItem]>,
    selected_branch: Option<SelectedHistoryBranch>,
    // Colour index of the lane the selection sits on; every other lane washes
    // out. A property of the lane, not of this row.
    selected_lane: Option<super::history_graph_paint::SelectedLane>,
    // Branch this commit belongs to, shown as a faded badge while the row is
    // hovered. Inherited down the lane, so unlabelled commits have one too.
    lane_branch_name: Option<SharedString>,
    author: HistoryTextVm,
    summary: HistoryTextVm,
    when: HistoryTextVm,
    short_sha: HistoryTextVm,
    selected: bool,
    is_head: bool,
    is_stash_node: bool,
    active_context_menu_invoker: Option<&SharedString>,
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let context_menu_invoker: SharedString =
        format!("history_commit_menu_{}_{}", repo_id.0, commit.id.as_ref()).into();
    let context_menu_active = active_context_menu_invoker == Some(&context_menu_invoker);
    // The row's background as one value rather than three `.bg()` calls that
    // overwrite each other, because the graph canvas needs to know it: its icon
    // nodes knock their glyphs out in the colour the row is actually painted,
    // and a knockout in the untinted surface leaves a visible patch inside a
    // tinted row. The hover tint is the canvas's business -- it owns the hitbox
    // -- so it is not folded in here.
    let row_bg_overlay = if context_menu_active {
        Some(theme.colors.interaction.pressed_background)
    } else if selected {
        Some(theme.colors.accent.subtle_background)
    } else if is_head {
        // A quiet tint keeps HEAD findable without competing with selection.
        Some(with_alpha(theme.colors.accent.foreground, 0.06))
    } else {
        None
    };
    let commit_row = history_canvas::history_commit_row_canvas(
        theme,
        cx.entity(),
        ix,
        repo_id,
        commit.id.clone(),
        col_branch,
        col_graph,
        col_author,
        col_date,
        col_sha,
        show_graph,
        show_author,
        show_date,
        show_sha,
        show_graph_color_marker,
        is_stash_node,
        connect_from_top_col,
        graph_rows,
        graph_row_ix,
        tag_names,
        branch_chips,
        ref_items,
        selected_branch,
        selected_lane,
        lane_branch_name,
        author,
        summary,
        when,
        short_sha,
        active_context_menu_invoker.cloned(),
        row_bg_overlay,
        if context_menu_active {
            theme.colors.interaction.pressed_background
        } else {
            theme.colors.interaction.hover_background
        },
    );

    let commit_id = commit.id.clone();
    let row_height = history_row_height(ui_scale);
    let mut row = div()
        .id(ix)
        .debug_selector(move || format!("history_row_{ix}"))
        .relative()
        .h(row_height)
        .w_full()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| {
            if context_menu_active {
                s.bg(theme.colors.interaction.pressed_background)
            } else {
                s.bg(theme.colors.interaction.hover_background)
            }
        })
        .active(move |s| s.bg(theme.colors.interaction.pressed_background))
        .child(commit_row)
        // Selecting on press, like the sidebar rows: the row the gesture
        // *starts* on owns it, so a release that merely drifted here — the end
        // of a text-selection drag in the details pane, say — selects nothing.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                let modifiers = e.modifiers;
                let mode = if modifiers.shift {
                    CommitSelectMode::Range
                } else if modifiers.secondary() || modifiers.control || modifiers.platform {
                    CommitSelectMode::Toggle
                } else {
                    CommitSelectMode::Single
                };
                let visible_order = (mode == CommitSelectMode::Range)
                    .then(|| this.visible_commit_ids_for_repo(repo_id))
                    .flatten();
                this.store.dispatch(Msg::SelectCommitMulti {
                    repo_id,
                    commit_id: commit_id.clone(),
                    mode,
                    clicked_index: Some(graph_row_ix),
                    visible_order,
                });
                cx.notify();
            }),
        );

    if let Some(overlay) = row_bg_overlay {
        row = row.bg(overlay);
    }

    // On light themes the selection tint lands within a few percent of the list
    // surface, so a selected row is a smudge rather than a marked row. Ring it
    // the way selected sidebar rows already are.
    if selected && let Some(outline) = components::light_theme_selection_outline(theme) {
        row = row.shadow(vec![outline]);
    }

    if is_head {
        row = row.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(ui_scale.px(3.0))
                .bg(with_alpha(theme.colors.accent.foreground, 0.90)),
        );
    }

    row.into_any_element()
}

/// One linked worktree's uncommitted changes, rendered directly above the commit
/// that worktree has checked out.
///
/// Unlike the working-tree summary row this one is not pinned to the top of the
/// list, so it paints the full lane band of the commit below it rather than a
/// single connector stub: the lanes have to run through it uninterrupted.
#[allow(clippy::too_many_arguments)]
fn worktree_uncommitted_history_row(
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_graph: bool,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    graph_row: &history_graph::GraphRow,
    // Index of the commit row the band sits on top of, whose lanes it draws.
    visible_ix: usize,
    connect_from_top_col: Option<usize>,
    selected_lane: Option<super::history_graph_paint::SelectedLane>,
    show_graph_color_marker: bool,
    repo_id: RepoId,
    list_ix: usize,
    selected: bool,
    counts: (usize, usize, usize),
    summary: &gitcomet_core::domain::WorktreeDirtySummary,
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let scaled_px = |value| ui_scale.px(value);
    let cell_pad_x = scaled_px(HISTORY_COL_HANDLE_PX / 2.0);
    let band_node = super::history_graph_paint::band_node_for(
        graph_row,
        summary.branch.is_some() && !summary.detached,
    );
    // The node washes with its lane, like every other node in the graph -- the
    // text beside it still follows the row's relation to the selection.
    let node_color = super::history_graph_paint::lane_wash_color(
        theme,
        band_node.color_ix,
        visible_ix,
        selected_lane,
    );
    // Everything on the row washes with the lane it sits on, text included.
    let on_selected_lane =
        selected_lane.map(|selected| selected.covers(theme, visible_ix, band_node.color_ix));
    let label_color = history_canvas::selection_related_summary_color(theme, on_selected_lane);

    // A pass-through band: whatever entered the commit below from above runs
    // straight through this row, so inserting it leaves the graph unbroken.
    // `None` when the node sits on a lane of its own already (a branch head's
    // fork), which needs only a straight connector down.
    let node_exit_col = (band_node.exit_col != band_node.col).then_some(band_node.exit_col);
    // Only the commit's lanes cross into the band; everything else about its row
    // (`lanes_next`, `joins_in`, `edges_out`) belongs to the commit and is never
    // painted here, so the band carries the lanes alone rather than a row-shaped
    // copy of them.
    let band_lanes = graph_row.lanes_now.clone();
    // The node's middle is opaque, so it has to be filled in what the row is
    // painted over rather than in the list's bare surface.
    let row_background = if selected {
        crate::theme::composite_over(
            theme.colors.surface.canvas,
            theme.colors.accent.subtle_background,
        )
    } else {
        theme.colors.surface.canvas
    };
    let graph = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            super::history_graph_paint::paint_history_graph_band(
                theme,
                &band_lanes,
                visible_ix,
                connect_from_top_col,
                selected_lane,
                super::history_graph_paint::BandNodePaint {
                    col: band_node.col,
                    color: node_color,
                    exit_col: node_exit_col,
                },
                show_graph_color_marker,
                row_background,
                bounds,
                window,
                cx,
            );
        },
    )
    .w_full()
    .h_full();

    let icon_count = |icon_path: &'static str, color: gpui::Rgba, count: usize| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(svg_icon(icon_path, color, scaled_px(12.0)))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.foreground.secondary)
                    .child(count.to_string()),
            )
            .into_any_element()
    };
    let (added, modified, deleted) = counts;
    let mut parts: Vec<AnyElement> = Vec::with_capacity(3);
    if modified > 0 {
        parts.push(icon_count(
            "icons/pencil.svg",
            theme.colors.status.warning.foreground,
            modified,
        ));
    }
    if added > 0 {
        parts.push(icon_count(
            "icons/plus.svg",
            theme.colors.status.success.foreground,
            added,
        ));
    }
    if deleted > 0 {
        parts.push(icon_count(
            "icons/minus.svg",
            theme.colors.status.danger.foreground,
            deleted,
        ));
    }

    let palette = super::sidebar::worktree_badge_palette(theme);
    let badge_label = super::sidebar::worktree_origin_label(
        summary.branch.as_deref(),
        summary.detached,
        &summary.path,
    );
    let open_path = summary.path.clone();
    let badge_tooltip: SharedString =
        format!("Open this worktree\n{}", summary.path.display()).into();

    let badge = super::sidebar::worktree_origin_chip(
        theme,
        badge_label,
        scaled_px(9.0),
        scaled_px(HISTORY_WORKTREE_BADGE_HEIGHT_PX),
        scaled_px(HISTORY_WORKTREE_BADGE_MAX_W_PX),
        scaled_px(6.0),
    )
    .id(("history_worktree_badge", list_ix))
    .cursor(CursorStyle::PointingHand)
    .hover(move |s| {
        s.border_color(palette.hover_border)
            .text_color(palette.hover_text)
    })
    .gitcomet_tooltip(theme, badge_tooltip)
    // The badge is a control of its own: a right or middle click must not open
    // the repo, and a left click on it must not also select the row underneath
    // -- the row belongs to the repo we are navigating away from.
    .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if !e.standard_click() {
            return;
        }
        cx.stop_propagation();
        this.store.dispatch(Msg::OpenRepo(open_path.clone()));
        cx.notify();
    }));

    let select_path = summary.path.clone();
    // These cells share the canvas graph's column offsets. Keep every fixed
    // column non-shrinking so a long worktree label can only clip its summary,
    // never move this row's graph lane away from the commit rows below it.
    let mut row = div()
        .id(("history_worktree_uncommitted", list_ix))
        .h(history_row_height(ui_scale))
        .flex()
        .w_full()
        .items_center()
        .px_2()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(theme.colors.interaction.hover_background))
        .active(move |s| s.bg(theme.colors.interaction.pressed_background))
        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
            if !e.standard_click() {
                return;
            }
            this.store.dispatch(Msg::SelectWorktreeUncommitted {
                repo_id,
                path: select_path.clone(),
            });
            cx.notify();
        }))
        .child(
            div()
                .w(col_branch)
                .flex_none()
                .text_xs()
                .line_clamp(1)
                .whitespace_nowrap()
                .child(div()),
        )
        .when(show_graph, |row| {
            row.child(
                div()
                    .w(col_graph)
                    .flex_none()
                    .h_full()
                    .overflow_hidden()
                    .child(graph),
            )
        })
        .child({
            let mut summary = div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap_2()
                // Same offset the commit rows put their text at, so the message
                // column reads as one column down the whole list.
                .pl(ui_scale.px(history_message_text_left_px(show_graph_color_marker)))
                .pr(cell_pad_x)
                .when(show_graph_color_marker, |cell| {
                    cell.child(history_message_border(ui_scale, node_color))
                });
            summary = summary.child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(label_color)
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child("Worktree changes"),
            );
            if !parts.is_empty() {
                summary = summary.child(div().flex().items_center().gap_2().children(parts));
            }
            summary.child(div().flex_1().min_w(px(0.0))).child(
                div()
                    .min_w(px(0.0))
                    .max_w(scaled_px(HISTORY_WORKTREE_BADGE_MAX_W_PX))
                    .overflow_hidden()
                    .child(badge),
            )
        })
        .when(show_author, |row| {
            row.child(div().w(col_author).flex_none())
        })
        .when(show_date, |row| row.child(div().w(col_date).flex_none()))
        .when(show_sha, |row| row.child(div().w(col_sha).flex_none()));

    if selected {
        row = row.bg(theme.colors.accent.subtle_background);
        // Same light-theme selection ring the commit rows wear.
        if let Some(outline) = components::light_theme_selection_outline(theme) {
            row = row.shadow(vec![outline]);
        }
    }

    row.into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn working_tree_summary_history_row(
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_graph: bool,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    node_color_ix: history_graph::LaneColorIx,
    selected_lane: Option<super::history_graph_paint::SelectedLane>,
    show_graph_color_marker: bool,
    repo_id: RepoId,
    selected: bool,
    counts: (usize, usize, usize),
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let scaled_px = |value| ui_scale.px(value);
    let cell_pad_x = scaled_px(HISTORY_COL_HANDLE_PX / 2.0);
    // The connector washes with its lane, like every other node in the graph;
    // the label still follows the row's relation to the selection.
    // The pinned row sits above the newest commit, so it shares row 0's lanes.
    let node_color =
        super::history_graph_paint::lane_wash_color(theme, node_color_ix, 0, selected_lane);
    let on_selected_lane = selected_lane.map(|selected| selected.covers(theme, 0, node_color_ix));
    let label_color = history_canvas::selection_related_summary_color(theme, on_selected_lane);
    let icon_count = |icon_path: &'static str, color: gpui::Rgba, count: usize| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(svg_icon(icon_path, color, scaled_px(12.0)))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.foreground.secondary)
                    .child(count.to_string()),
            )
            .into_any_element()
    };

    let (added, modified, deleted) = counts;
    let mut parts: Vec<AnyElement> = Vec::with_capacity(3);
    if modified > 0 {
        parts.push(icon_count(
            "icons/pencil.svg",
            theme.colors.status.warning.foreground,
            modified,
        ));
    }
    if added > 0 {
        parts.push(icon_count(
            "icons/plus.svg",
            theme.colors.status.success.foreground,
            added,
        ));
    }
    if deleted > 0 {
        parts.push(icon_count(
            "icons/minus.svg",
            theme.colors.status.danger.foreground,
            deleted,
        ));
    }

    // What the row is *actually* painted over, so the node's opaque middle hides
    // the lane running through its column without leaving an untinted disc
    // punched into a selected row. Same compositing the linked-worktree band row
    // does; the hover tint stays out of it, being the div's business here.
    let node_background = if selected {
        crate::theme::composite_over(
            theme.colors.surface.canvas,
            theme.colors.accent.subtle_background,
        )
    } else {
        theme.colors.surface.canvas
    };
    let circle = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            use gpui::{PathBuilder, point};
            let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
            let scaled_px = |value| px(value * design_scale_factor);
            let margin_x = scaled_px(HISTORY_GRAPH_MARGIN_X_PX);
            let col_gap = scaled_px(HISTORY_GRAPH_COL_GAP_PX);
            let node_x = margin_x + col_gap * 0.0;
            let center = point(
                bounds.left() + node_x,
                bounds.top() + bounds.size.height / 2.0,
            );

            // Connect the working tree node into the history graph below.
            let stroke_width = scaled_px(1.6);
            let mut path = PathBuilder::stroke(stroke_width);
            path.move_to(point(center.x, center.y));
            path.line_to(point(center.x, bounds.bottom()));
            if let Ok(p) = path.build() {
                window.paint_path(p, node_color);
            }

            if show_graph_color_marker {
                super::history_graph_paint::paint_graph_fade(
                    node_color,
                    bounds,
                    scaled_px(HISTORY_GRAPH_FADE_WIDTH_PX),
                    window,
                );
            }

            super::history_graph_paint::paint_ring_icon_node(
                center.x,
                center.y,
                icons::UNCOMMITTED_NODE_ICON_PATH,
                node_color,
                node_background,
                window,
                cx,
            );
        },
    )
    .w_full()
    .h_full()
    .cursor(CursorStyle::PointingHand);

    // Match the same fixed column geometry used by commit and worktree rows;
    // the flexible summary is the only cell allowed to absorb width pressure.
    let mut row = div()
        .id(("history_worktree_summary", repo_id.0))
        .h(history_row_height(ui_scale))
        .flex()
        .w_full()
        .items_center()
        .px_2()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(theme.colors.interaction.hover_background))
        .active(move |s| s.bg(theme.colors.interaction.pressed_background))
        .child(
            div()
                .w(col_branch)
                .flex_none()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .line_clamp(1)
                .whitespace_nowrap()
                .child(div()),
        )
        .when(show_graph, |row| {
            row.child(
                div()
                    .w(col_graph)
                    .flex_none()
                    .h_full()
                    .flex()
                    .justify_center()
                    .overflow_hidden()
                    .child(circle),
            )
        })
        .child({
            let mut summary = div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap_2()
                // Same offset the commit rows put their text at, so the message
                // column reads as one column down the whole list.
                .pl(ui_scale.px(history_message_text_left_px(show_graph_color_marker)))
                .pr(cell_pad_x)
                .when(show_graph_color_marker, |cell| {
                    cell.child(history_message_border(ui_scale, node_color))
                });
            summary = summary.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .text_color(label_color)
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child("Uncommitted changes"),
            );
            if !parts.is_empty() {
                summary = summary.child(div().flex().items_center().gap_2().children(parts));
            }
            summary
        })
        .when(show_author, |row| {
            row.child(div().w(col_author).flex_none())
        })
        .when(show_date, |row| {
            row.child(
                div()
                    .w(col_date)
                    .flex_none()
                    .flex()
                    .justify_end()
                    .px(cell_pad_x)
                    .text_xs()
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.foreground.secondary)
                    .whitespace_nowrap()
                    .child("Click to review"),
            )
        })
        .when(show_sha, |row| row.child(div().w(col_sha).flex_none()))
        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.select_working_tree_summary_row(repo_id, cx);
        }));

    if selected {
        row = row.bg(theme.colors.accent.subtle_background);
        // Same light-theme selection ring the commit rows wear.
        if let Some(outline) = components::light_theme_selection_outline(theme) {
            row = row.shadow(vec![outline]);
        }
    }

    row.into_any_element()
}

mod markdown_preview_rows;
mod worktree_preview;

pub(in crate::view) use markdown_preview_rows::*;
#[cfg(test)]
use worktree_preview::*;

#[cfg(test)]
mod markdown_preview_search_tests;
#[cfg(test)]
mod tests;
