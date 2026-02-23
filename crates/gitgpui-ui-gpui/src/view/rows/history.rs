use super::diff_canvas;
use super::diff_text::*;
use super::history_canvas;
use super::*;

impl MainPaneView {
    pub(in super::super) fn render_worktree_preview_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let query = if this.diff_search_active {
            this.diff_search_query.as_ref()
        } else {
            ""
        };

        let theme = this.theme;
        let Some(path) = this.worktree_preview_path.as_ref() else {
            return Vec::new();
        };
        let Loadable::Ready(lines) = &this.worktree_preview else {
            return Vec::new();
        };

        let should_clear_cache = match this.worktree_preview_segments_cache_path.as_ref() {
            Some(p) => p != path,
            None => true,
        };
        if should_clear_cache {
            this.worktree_preview_segments_cache_path = Some(path.clone());
            this.worktree_preview_syntax_language =
                diff_syntax_language_for_path(path.to_string_lossy().as_ref());
            this.worktree_preview_segments_cache.clear();
        }

        let syntax_mode = if lines.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
            DiffSyntaxMode::Auto
        } else {
            DiffSyntaxMode::HeuristicOnly
        };
        let language = this.worktree_preview_syntax_language;

        let highlight_deleted_file = this.deleted_file_preview_abs_path().is_some();
        let highlight_new_file = this.untracked_worktree_preview_path().is_some()
            || this.added_file_preview_abs_path().is_some()
            || this.diff_preview_is_new_file;
        let bar_color = if highlight_deleted_file {
            Some(theme.colors.danger)
        } else if highlight_new_file {
            Some(theme.colors.success)
        } else {
            None
        };

        range
            .map(|ix| {
                let line = lines.get(ix).map(String::as_str).unwrap_or("");

                let styled = this
                    .worktree_preview_segments_cache
                    .entry(ix)
                    .or_insert_with(|| {
                        build_cached_diff_styled_text(
                            theme,
                            line,
                            &[],
                            query,
                            language,
                            syntax_mode,
                            None,
                        )
                    });

                let line_no = line_number_string(u32::try_from(ix + 1).ok());
                diff_canvas::worktree_preview_row_canvas(
                    theme,
                    cx.entity(),
                    ix,
                    px(0.0),
                    bar_color,
                    line_no,
                    styled,
                )
            })
            .collect()
    }

    pub(in super::super) fn render_history_table_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let (show_working_tree_summary_row, worktree_counts) =
            this.ensure_history_worktree_summary_cache();
        let stash_ids = this.ensure_history_stash_ids_cache();

        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };

        let theme = this.theme;
        let col_branch = this.history_col_branch;
        let col_graph = this.history_col_graph;
        let col_author = this.history_col_author;
        let col_date = this.history_col_date;
        let col_sha = this.history_col_sha;
        let (show_author, show_date, show_sha) = this.history_visible_columns();

        let page = match &repo.log {
            Loadable::Ready(page) => Some(page),
            _ => None,
        };
        let cache = this
            .history_cache
            .as_ref()
            .filter(|c| c.request.repo_id == repo.id);
        let worktree_node_color = cache
            .and_then(|c| c.graph_rows.first())
            .and_then(|row| row.lanes_now.get(row.node_col).map(|l| l.color))
            .unwrap_or(theme.colors.accent);

        range
            .filter_map(|list_ix| {
                if show_working_tree_summary_row && list_ix == 0 {
                    let selected = repo.selected_commit.is_none();
                    return Some(working_tree_summary_history_row(
                        theme,
                        col_branch,
                        col_graph,
                        col_author,
                        col_date,
                        col_sha,
                        show_author,
                        show_date,
                        show_sha,
                        worktree_node_color,
                        repo.id,
                        selected,
                        worktree_counts,
                        cx,
                    ));
                }

                let offset = usize::from(show_working_tree_summary_row);
                let visible_ix = list_ix.checked_sub(offset)?;

                let page = page?;
                let cache = cache?;

                let commit_ix = cache.visible_indices.get(visible_ix).copied()?;
                let commit = page.commits.get(commit_ix)?;
                let graph_row = cache.graph_rows.get(visible_ix)?;
                let row_vm = cache.commit_row_vms.get(visible_ix)?;
                let connect_incoming_node = show_working_tree_summary_row && visible_ix == 0;
                let selected = repo.selected_commit.as_ref() == Some(&commit.id);
                let show_graph_color_marker =
                    repo.history_scope == gitgpui_core::domain::LogScope::AllBranches;
                let is_stash_node = stash_ids
                    .as_ref()
                    .is_some_and(|ids| ids.contains(&commit.id));

                Some(history_table_row(
                    theme,
                    col_branch,
                    col_graph,
                    col_author,
                    col_date,
                    col_sha,
                    show_author,
                    show_date,
                    show_sha,
                    show_graph_color_marker,
                    list_ix,
                    repo.id,
                    commit,
                    Arc::clone(graph_row),
                    connect_incoming_node,
                    Arc::clone(&row_vm.tag_names),
                    row_vm.branches_text.clone(),
                    row_vm.author.clone(),
                    row_vm.summary.clone(),
                    row_vm.when.clone(),
                    row_vm.short_sha.clone(),
                    selected,
                    row_vm.is_head,
                    is_stash_node,
                    this.active_context_menu_invoker.as_ref(),
                    cx,
                ))
            })
            .collect()
    }
}

const HISTORY_ROW_HEIGHT_PX: f32 = 24.0;

#[allow(clippy::too_many_arguments)]
fn history_table_row(
    theme: AppTheme,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    show_graph_color_marker: bool,
    ix: usize,
    repo_id: RepoId,
    commit: &Commit,
    graph_row: Arc<history_graph::GraphRow>,
    connect_incoming_node: bool,
    tag_names: Arc<[SharedString]>,
    branches_text: SharedString,
    author: SharedString,
    summary: SharedString,
    when: SharedString,
    short_sha: SharedString,
    selected: bool,
    is_head: bool,
    is_stash_node: bool,
    active_context_menu_invoker: Option<&SharedString>,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let context_menu_invoker: SharedString = format!(
        "history_commit_menu_{}_{}",
        repo_id.0,
        commit.id.0.as_str()
    )
    .into();
    let context_menu_active = active_context_menu_invoker == Some(&context_menu_invoker);
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
        show_author,
        show_date,
        show_sha,
        show_graph_color_marker,
        is_stash_node,
        connect_incoming_node,
        graph_row,
        tag_names,
        branches_text,
        author,
        summary,
        when,
        short_sha,
    );

    let commit_id = commit.id.clone();
    let mut row = div()
        .id(ix)
        .relative()
        .h(px(HISTORY_ROW_HEIGHT_PX))
        .w_full()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| {
            if context_menu_active {
                s.bg(theme.colors.active)
            } else {
                s.bg(theme.colors.hover)
            }
        })
        .active(move |s| s.bg(theme.colors.active))
        .child(commit_row)
        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.store.dispatch(Msg::SelectCommit {
                repo_id,
                commit_id: commit_id.clone(),
            });
            cx.notify();
        }));

    if selected {
        row = row.bg(with_alpha(theme.colors.accent, 0.15));
    }
    if context_menu_active {
        row = row.bg(theme.colors.active);
    }

    if is_head {
        let thickness = px(1.0);
        let color = with_alpha(theme.colors.accent, 0.90);
        row = row
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(thickness)
                    .bg(color),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(thickness)
                    .bg(color),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(thickness)
                    .bg(color),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(thickness)
                    .bg(color),
            );
    }

    row.into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn working_tree_summary_history_row(
    theme: AppTheme,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    node_color: gpui::Rgba,
    repo_id: RepoId,
    selected: bool,
    counts: (usize, usize, usize),
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let cell_pad_x = px(HISTORY_COL_HANDLE_PX / 2.0);
    let icon_count = |icon: &'static str, color: gpui::Rgba, count: usize| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .text_color(color)
                    .child(icon),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child(count.to_string()),
            )
            .into_any_element()
    };

    let (added, modified, deleted) = counts;
    let mut parts: Vec<AnyElement> = Vec::with_capacity(3);
    if modified > 0 {
        parts.push(icon_count("✎", theme.colors.warning, modified));
    }
    if added > 0 {
        parts.push(icon_count("+", theme.colors.success, added));
    }
    if deleted > 0 {
        parts.push(icon_count("−", theme.colors.danger, deleted));
    }

    let black = gpui::rgba(0x000000ff);
    let circle = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _cx| {
            use gpui::{PathBuilder, fill, point, px, size};
            let r = px(3.0);
            let border = px(1.0);
            let outer = r + border;
            let margin_x = px(HISTORY_GRAPH_MARGIN_X_PX);
            let col_gap = px(HISTORY_GRAPH_COL_GAP_PX);
            let node_x = margin_x + col_gap * 0.0;
            let center = point(
                bounds.left() + node_x,
                bounds.top() + bounds.size.height / 2.0,
            );

            // Connect the working tree node into the history graph below.
            let stroke_width = px(1.6);
            let mut path = PathBuilder::stroke(stroke_width);
            path.move_to(point(center.x, center.y));
            path.line_to(point(center.x, bounds.bottom()));
            if let Ok(p) = path.build() {
                window.paint_path(p, node_color);
            }

            window.paint_quad(
                fill(
                    gpui::Bounds::new(
                        point(center.x - outer, center.y - outer),
                        size(outer * 2.0, outer * 2.0),
                    ),
                    node_color,
                )
                .corner_radii(outer),
            );
            window.paint_quad(
                fill(
                    gpui::Bounds::new(point(center.x - r, center.y - r), size(r * 2.0, r * 2.0)),
                    black,
                )
                .corner_radii(r),
            );
        },
    )
    .w_full()
    .h_full()
    .cursor(CursorStyle::PointingHand);

    let mut row = div()
        .id(("history_worktree_summary", repo_id.0))
        .h(px(HISTORY_ROW_HEIGHT_PX))
        .flex()
        .w_full()
        .items_center()
        .px_2()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(theme.colors.hover))
        .active(move |s| s.bg(theme.colors.active))
        .child(
            div()
                .w(col_branch)
                .text_xs()
                .text_color(theme.colors.text_muted)
                .line_clamp(1)
                .whitespace_nowrap()
                .child(div()),
        )
        .child(
            div()
                .w(col_graph)
                .h_full()
                .flex()
                .justify_center()
                .overflow_hidden()
                .child(circle),
        )
        .child({
            let mut summary = div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                .px(cell_pad_x);
            summary = summary.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child("Uncommitted changes"),
            );
            if !parts.is_empty() {
                summary = summary.child(div().flex().items_center().gap_2().children(parts));
            }
            summary
        })
        .when(show_author, |row| row.child(div().w(col_author)))
        .when(show_date, |row| {
            row.child(
                div()
                    .w(col_date)
                    .flex()
                    .justify_end()
                    .px(cell_pad_x)
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .whitespace_nowrap()
                    .child("Click to review"),
            )
        })
        .when(show_sha, |row| row.child(div().w(col_sha)))
        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.store.dispatch(Msg::ClearCommitSelection { repo_id });
            this.store.dispatch(Msg::ClearDiffSelection { repo_id });
            cx.notify();
        }));

    if selected {
        row = row.bg(with_alpha(theme.colors.accent, 0.15));
    }

    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use crate::view::{DateTimeFormat, format_datetime_utc};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn commit_date_formats_as_yyyy_mm_dd_utc() {
        assert_eq!(
            format_datetime_utc(UNIX_EPOCH, DateTimeFormat::YmdHm),
            "1970-01-01 00:00"
        );
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH + Duration::from_secs(86_400),
                DateTimeFormat::YmdHm
            ),
            "1970-01-02 00:00"
        );
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH - Duration::from_secs(86_400),
                DateTimeFormat::YmdHm
            ),
            "1969-12-31 00:00"
        );

        // 2000-02-29 12:34:56 UTC
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH + Duration::from_secs(951_782_400 + 12 * 3600 + 34 * 60 + 56),
                DateTimeFormat::YmdHms
            ),
            "2000-02-29 12:34:56"
        );
    }
}
