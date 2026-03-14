use super::diff_canvas;
use super::diff_text::*;
use super::history_canvas;
use super::*;

use crate::view::markdown_preview::{
    MarkdownChangeHint, MarkdownInlineStyle, MarkdownPreviewDocument, MarkdownPreviewRow,
    MarkdownPreviewRowKind,
};
use crate::view::perf::{self, ViewPerfRenderLane, ViewPerfSpan};

impl MainPaneView {
    pub(in super::super) fn render_worktree_preview_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let min_width = this.diff_horizontal_min_width;
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
            this.worktree_preview_syntax_language = diff_syntax_language_for_path(path);
            this.worktree_preview_segments_cache.clear();
        }

        let configured_syntax_mode = if lines.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
            DiffSyntaxMode::Auto
        } else {
            DiffSyntaxMode::HeuristicOnly
        };
        let language = this.worktree_preview_syntax_language;
        let syntax_document = this.worktree_preview_prepared_syntax_document();
        let syntax_mode = if syntax_document.is_some() {
            configured_syntax_mode
        } else {
            DiffSyntaxMode::HeuristicOnly
        };

        let bar_color = worktree_preview_bar_color(this, theme);

        range
            .map(|ix| {
                let line = lines.get(ix).map(String::as_str).unwrap_or("");

                let styled = this
                    .worktree_preview_segments_cache
                    .entry(ix)
                    .or_insert_with(|| {
                        build_cached_diff_styled_text_for_prepared_document_line(
                            theme,
                            line,
                            &[],
                            query,
                            DiffSyntaxConfig {
                                language,
                                mode: syntax_mode,
                            },
                            None,
                            PreparedDiffSyntaxLine {
                                document: syntax_document,
                                line_ix: ix,
                            },
                        )
                    });

                let line_no = line_number_string(u32::try_from(ix + 1).ok());
                diff_canvas::worktree_preview_row_canvas(
                    theme,
                    cx.entity(),
                    ix,
                    min_width,
                    bar_color,
                    line_no,
                    styled,
                )
            })
            .collect()
    }

    pub(in super::super) fn render_markdown_preview_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let Loadable::Ready(document) = &this.worktree_markdown_preview else {
            return Vec::new();
        };
        render_markdown_preview_document_rows(
            theme,
            document,
            range,
            worktree_preview_bar_color(this, theme),
        )
    }

    pub(in super::super) fn render_markdown_diff_left_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let Loadable::Ready(preview) = &this.file_markdown_preview else {
            return Vec::new();
        };
        render_markdown_preview_document_rows(theme, &preview.old, range, None)
    }

    pub(in super::super) fn render_markdown_diff_right_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let Loadable::Ready(preview) = &this.file_markdown_preview else {
            return Vec::new();
        };
        render_markdown_preview_document_rows(theme, &preview.new, range, None)
    }
}

const MARKDOWN_PREVIEW_ROW_HEIGHT_PX: f32 = 24.0;

pub(super) fn render_markdown_preview_document_rows(
    theme: AppTheme,
    document: &MarkdownPreviewDocument,
    range: Range<usize>,
    bar_color: Option<gpui::Rgba>,
) -> Vec<AnyElement> {
    let requested_rows = range.len();
    let rows = range
        .filter_map(|ix| {
            let row = document.rows.get(ix)?;
            Some(markdown_preview_row_element(theme, row, bar_color))
        })
        .collect::<Vec<_>>();
    perf::record_row_batch(
        ViewPerfRenderLane::MarkdownPreview,
        requested_rows,
        rows.len(),
    );
    rows
}

fn markdown_preview_row_element(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
    bar_color: Option<gpui::Rgba>,
) -> AnyElement {
    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewStyledRowBuild);
    let (display, highlights) = markdown_preview_display_and_highlights(theme, row);
    let text_color = markdown_preview_row_text_color(theme, row);
    let indent_steps = f32::from(row.indent_level.saturating_sub(1));
    let indent = px(8.0 + indent_steps * 16.0);

    let mut content = div()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex()
        .items_center()
        .pl(indent)
        .pr(px(12.0))
        .whitespace_nowrap()
        .text_xs()
        .text_color(text_color);

    content = match row.kind {
        MarkdownPreviewRowKind::Heading { level: 1 } => {
            content.text_sm().font_weight(FontWeight::BOLD)
        }
        MarkdownPreviewRowKind::Heading { .. } => content.font_weight(FontWeight::BOLD),
        MarkdownPreviewRowKind::CodeLine { .. }
        | MarkdownPreviewRowKind::TableRow
        | MarkdownPreviewRowKind::PlainFallback => content.font_family(UI_MONOSPACE_FONT_FAMILY),
        _ => content,
    };

    let text = if highlights.is_empty() {
        content.child(display).into_any_element()
    } else {
        content
            .child(gpui::StyledText::new(display).with_highlights(highlights))
            .into_any_element()
    };

    div()
        .h(px(MARKDOWN_PREVIEW_ROW_HEIGHT_PX))
        .min_h(px(MARKDOWN_PREVIEW_ROW_HEIGHT_PX))
        .flex()
        .items_center()
        .when_some(markdown_preview_row_background(theme, row), |div, bg| {
            div.bg(bg)
        })
        .when_some(bar_color, |container, color| {
            container.child(div().h_full().w(px(3.0)).bg(color))
        })
        .child(text)
        .into_any_element()
}

fn worktree_preview_bar_color(this: &MainPaneView, theme: AppTheme) -> Option<gpui::Rgba> {
    let highlight_deleted_file = this.deleted_file_preview_abs_path().is_some();
    let highlight_new_file = this.untracked_worktree_preview_path().is_some()
        || this.added_file_preview_abs_path().is_some()
        || this.diff_preview_is_new_file;
    if highlight_deleted_file {
        Some(theme.colors.danger)
    } else if highlight_new_file {
        Some(theme.colors.success)
    } else {
        None
    }
}

fn markdown_preview_display_and_highlights(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    let prefix = markdown_preview_row_prefix(row);
    let prefix_len = prefix.len();

    let display: SharedString = if prefix_len == 0 {
        row.text.clone()
    } else {
        let mut buf = String::with_capacity(prefix_len + row.text.len());
        buf.push_str(&prefix);
        buf.push_str(row.text.as_ref());
        buf.into()
    };

    let highlights = row
        .inline_spans
        .iter()
        .filter_map(|span| {
            let style = markdown_preview_inline_highlight(theme, span.style);
            (style != gpui::HighlightStyle::default()).then_some((
                (prefix_len + span.byte_range.start)..(prefix_len + span.byte_range.end),
                style,
            ))
        })
        .collect();

    (display, highlights)
}

fn markdown_preview_row_prefix(row: &MarkdownPreviewRow) -> std::borrow::Cow<'static, str> {
    let suffix = match row.kind {
        MarkdownPreviewRowKind::ListItem { number: Some(n) } => {
            return if row.blockquote_level == 0 {
                format!("{n}. ").into()
            } else {
                markdown_preview_row_prefix_with_blockquote(row.blockquote_level, &format!("{n}. "))
            };
        }
        MarkdownPreviewRowKind::ListItem { number: None } => "• ",
        _ => "",
    };
    markdown_preview_row_prefix_with_blockquote(row.blockquote_level, suffix)
}

fn markdown_preview_row_prefix_with_blockquote(
    blockquote_level: u8,
    suffix: &str,
) -> std::borrow::Cow<'static, str> {
    match (blockquote_level, suffix) {
        (0, "") => "".into(),
        (0, "• ") => "• ".into(),
        (1, "") => "│ ".into(),
        _ => {
            let mut prefix =
                String::with_capacity(usize::from(blockquote_level) * 4 + suffix.len());
            for _ in 0..blockquote_level {
                prefix.push_str("│ ");
            }
            prefix.push_str(suffix);
            prefix.into()
        }
    }
}

fn markdown_preview_inline_highlight(
    theme: AppTheme,
    style: MarkdownInlineStyle,
) -> gpui::HighlightStyle {
    match style {
        MarkdownInlineStyle::Normal => gpui::HighlightStyle::default(),
        MarkdownInlineStyle::Bold => gpui::HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Italic => gpui::HighlightStyle {
            font_style: Some(gpui::FontStyle::Italic),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::BoldItalic => gpui::HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            font_style: Some(gpui::FontStyle::Italic),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Code => gpui::HighlightStyle {
            background_color: Some(
                with_alpha(
                    theme.colors.active_section,
                    if theme.is_dark { 0.75 } else { 0.55 },
                )
                .into(),
            ),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Strikethrough => gpui::HighlightStyle {
            color: Some(theme.colors.text_muted.into()),
            strikethrough: Some(gpui::StrikethroughStyle {
                thickness: px(1.0),
                color: Some(theme.colors.text_muted.into()),
            }),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Link => gpui::HighlightStyle {
            color: Some(theme.colors.accent.into()),
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.accent.into()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
    }
}

fn markdown_preview_row_text_color(theme: AppTheme, row: &MarkdownPreviewRow) -> gpui::Rgba {
    match row.kind {
        MarkdownPreviewRowKind::Heading { level: 1 | 2 } => theme.colors.accent,
        MarkdownPreviewRowKind::Heading { .. }
        | MarkdownPreviewRowKind::BlockquoteLine
        | MarkdownPreviewRowKind::ThematicBreak => theme.colors.text_muted,
        MarkdownPreviewRowKind::PlainFallback => theme.colors.warning,
        _ => theme.colors.text,
    }
}

fn markdown_preview_row_background(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> Option<gpui::Rgba> {
    use MarkdownChangeHint as Hint;
    use MarkdownPreviewRowKind as Kind;

    match row.change_hint {
        Hint::Added => Some(with_alpha(
            theme.colors.success,
            if theme.is_dark { 0.18 } else { 0.12 },
        )),
        Hint::Removed => Some(with_alpha(
            theme.colors.danger,
            if theme.is_dark { 0.16 } else { 0.10 },
        )),
        Hint::Modified => Some(with_alpha(
            theme.colors.accent,
            if theme.is_dark { 0.18 } else { 0.10 },
        )),
        Hint::None => match row.kind {
            Kind::CodeLine { .. } => Some(with_alpha(
                theme.colors.active_section,
                if theme.is_dark { 0.58 } else { 0.34 },
            )),
            Kind::ThematicBreak => Some(with_alpha(
                theme.colors.border,
                if theme.is_dark { 0.60 } else { 0.35 },
            )),
            Kind::PlainFallback => Some(with_alpha(
                theme.colors.warning,
                if theme.is_dark { 0.08 } else { 0.06 },
            )),
            _ => None,
        },
    }
}

impl HistoryView {
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
                    let selected = repo.history_state.selected_commit.is_none();
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
                let selected = repo.history_state.selected_commit.as_ref() == Some(&commit.id);
                let show_graph_color_marker = repo.history_state.history_scope
                    == gitcomet_core::domain::LogScope::AllBranches;
                let is_stash_node = row_vm.is_stash
                    || stash_ids
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
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let context_menu_invoker: SharedString =
        format!("history_commit_menu_{}_{}", repo_id.0, commit.id.0.as_str()).into();
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
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let cell_pad_x = px(HISTORY_COL_HANDLE_PX / 2.0);
    let icon_count = |icon_path: &'static str, color: gpui::Rgba, count: usize| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(svg_icon(icon_path, color, px(12.0)))
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
        parts.push(icon_count(
            "icons/pencil.svg",
            theme.colors.warning,
            modified,
        ));
    }
    if added > 0 {
        parts.push(icon_count("icons/plus.svg", theme.colors.success, added));
    }
    if deleted > 0 {
        parts.push(icon_count("icons/minus.svg", theme.colors.danger, deleted));
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
                .corner_radii(outer.min(px(2.0))),
            );
            window.paint_quad(
                fill(
                    gpui::Bounds::new(point(center.x - r, center.y - r), size(r * 2.0, r * 2.0)),
                    black,
                )
                .corner_radii(r.min(px(2.0))),
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
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
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
    use super::{
        MarkdownChangeHint, MarkdownPreviewRow, MarkdownPreviewRowKind, markdown_preview_row_prefix,
    };
    use crate::view::{DateTimeFormat, Timezone, format_datetime, format_datetime_utc};
    use gpui::SharedString;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn commit_date_formats_as_yyyy_mm_dd_utc() {
        assert_eq!(
            format_datetime_utc(UNIX_EPOCH, DateTimeFormat::YmdHm),
            "1970-01-01 00:00 UTC"
        );
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH + Duration::from_secs(86_400),
                DateTimeFormat::YmdHm
            ),
            "1970-01-02 00:00 UTC"
        );
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH - Duration::from_secs(86_400),
                DateTimeFormat::YmdHm
            ),
            "1969-12-31 00:00 UTC"
        );

        // 2000-02-29 12:34:56 UTC
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH + Duration::from_secs(951_782_400 + 12 * 3600 + 34 * 60 + 56),
                DateTimeFormat::YmdHms
            ),
            "2000-02-29 12:34:56 UTC"
        );
    }

    #[test]
    fn format_datetime_with_timezone_offset() {
        // UTC+5:30 (19800 seconds)
        let tz = Timezone::Fixed(19800);
        assert_eq!(
            format_datetime(UNIX_EPOCH, DateTimeFormat::YmdHm, tz, true),
            "1970-01-01 05:30 UTC+5:30"
        );

        // UTC-5
        let tz_neg = Timezone::Fixed(-18000);
        assert_eq!(
            format_datetime(
                UNIX_EPOCH + Duration::from_secs(86_400),
                DateTimeFormat::YmdHm,
                tz_neg,
                true,
            ),
            "1970-01-01 19:00 UTC\u{2212}5"
        );
    }

    #[test]
    fn format_datetime_can_hide_timezone_label() {
        let tz = Timezone::Fixed(7200);
        assert_eq!(
            format_datetime(UNIX_EPOCH, DateTimeFormat::YmdHm, tz, false),
            "1970-01-01 02:00"
        );
    }

    #[test]
    fn timezone_key_round_trips() {
        for tz in Timezone::all() {
            let key = tz.key();
            let parsed = Timezone::from_key(&key);
            assert_eq!(parsed, Some(*tz), "round-trip failed for {key}");
        }
    }

    #[test]
    fn worktree_preview_renderer_avoids_full_document_prepare_calls() {
        let source = include_str!("history.rs");
        let render_start = source
            .find("fn render_worktree_preview_rows")
            .expect("render_worktree_preview_rows should exist");
        let render_end = source[render_start..]
            .find("impl HistoryView")
            .map(|offset| render_start + offset)
            .expect("HistoryView impl should follow worktree preview renderer");
        let render_source = &source[render_start..render_end];

        assert!(
            !render_source.contains("prepare_diff_syntax_document("),
            "row renderer should not build prepared syntax documents"
        );
        assert!(
            !render_source.contains("prepare_diff_syntax_document_with_budget("),
            "row renderer should not run full-document parse prep"
        );
    }

    #[test]
    fn markdown_preview_row_prefix_preserves_ordered_item_number() {
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::ListItem { number: Some(7) },
            text: SharedString::from("item"),
            inline_spans: Arc::new(Vec::new()),
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 0,
        };

        assert_eq!(markdown_preview_row_prefix(&row), "7. ");
    }

    #[test]
    fn markdown_preview_row_prefix_includes_blockquote_depth() {
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::BlockquoteLine,
            text: SharedString::from("quote"),
            inline_spans: Arc::new(Vec::new()),
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 2,
        };

        assert_eq!(markdown_preview_row_prefix(&row), "│ │ ");
    }

    #[test]
    fn markdown_preview_row_prefix_combines_blockquote_and_list_markers() {
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::ListItem { number: None },
            text: SharedString::from("item"),
            inline_spans: Arc::new(Vec::new()),
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 1,
        };

        assert_eq!(markdown_preview_row_prefix(&row), "│ • ");
    }
}
