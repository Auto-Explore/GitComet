//! The details view for another linked worktree's uncommitted changes.

use super::*;

impl DetailsPaneView {
    /// The changed files of a linked worktree that is *not* this tab, shown when
    /// its history row is selected.
    ///
    /// The worktree chip is the header rather than decoration: everything below
    /// belongs to another checkout, and nothing else on screen says so.
    pub(super) fn worktree_uncommitted_view(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();

        // Only the counts and the chip's three fields are needed here. Cloning the
        // summary would copy both `FileStatus` vectors -- every changed *and*
        // untracked file of the worktree -- on every repaint of this pane.
        let Some((file_count, loaded_file_count, chip_label, worktree_path)) =
            self.selected_worktree_summary().map(|summary| {
                (
                    // Counts, not `staged.len() + unstaged.len()`: the file lists
                    // arrive with the scan the selection asked for, and the header
                    // has to be right before then. Each changed file lands in
                    // exactly one bucket, so the two agree once loaded.
                    summary.added + summary.modified + summary.deleted,
                    summary.staged.len() + summary.unstaged.len(),
                    crate::view::rows::sidebar::worktree_origin_label(
                        summary.branch.as_deref(),
                        summary.detached,
                        &summary.path,
                    ),
                    summary.path.clone(),
                )
            })
        else {
            return div().into_any_element();
        };

        let header = components::content_header_bar(theme, ui_scale)
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    // Not "Uncommitted changes": that is the current repo's
                    // own row, and these are somebody else's.
                    .child(SharedString::from("Worktree changes")),
            )
            .child(div().flex_1().min_w(px(0.0)))
            .child({
                let open_path = worktree_path.clone();

                crate::view::rows::sidebar::worktree_origin_chip(
                    "worktree_uncommitted_origin",
                    theme,
                    chip_label,
                    ui_scale.px(10.0),
                    crate::view::rows::sidebar::worktree_badge_height(ui_scale),
                    ui_scale.px(220.0),
                    ui_scale.px(6.0),
                )
                .debug_selector(|| "worktree_uncommitted_open".to_string())
                .gitcomet_tooltip(
                    theme,
                    format!("Open this worktree in a tab\n{}", worktree_path.display()).into(),
                )
                // A chip is a control of its own: a right or middle click must not
                // open a repo tab, and a left click must not reach the row behind it.
                .on_activate(
                    false,
                    controls::ControlActivation::Nested,
                    cx.listener(move |_this, e: &ClickEvent, window, cx| {
                        if !e.standard_click() {
                            return;
                        }
                        cx.stop_propagation();
                        crate::app::open_repository_from_view(
                            cx,
                            window.window_handle().window_id(),
                            open_path.clone(),
                        );
                        cx.notify();
                    }),
                )
            })
            .child(
                components::Button::new("worktree_uncommitted_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.store.dispatch(Msg::ClearCommitSelection {
                            request_id: None,
                            repo_id,
                        });
                        cx.notify();
                    })
                    .gitcomet_tooltip(theme, "Close".into()),
            );

        // No "no files" state: the scan only reports a worktree once
        // `WorktreeDirtySummary::is_dirty` holds, so `file_count` -- the sum of
        // those same three counts -- is always positive here. A change that
        // started reporting clean worktrees would need a branch of its own; it
        // would otherwise sit on "Loading files…" forever.
        let worktree_inputs = self.selected_worktree_summary().map(|summary| {
            let rev = self
                .active_repo()
                .map(|repo| repo.worktree_dirty_rev)
                .unwrap_or_default();
            (summary.path.clone(), rev)
        });
        let (worktree_row_count, worktree_counts) = worktree_inputs
            .as_ref()
            .and_then(|(path, rev)| {
                let summary = self.selected_worktree_summary()?;
                let inputs = self.cached_worktree_file_inputs(repo_id, *rev, summary);
                let projection =
                    self.cached_worktree_file_projection(repo_id, *rev, path, &inputs.files);
                let plan = self.cached_worktree_file_plan(repo_id, *rev, path, &inputs.files);
                Some((plan.row_len(), projection.counts))
            })
            .unwrap_or((loaded_file_count, Default::default()));
        let worktree_controls = self.file_list_controls(
            crate::view::rows::FileListId::WorktreeFiles,
            repo_id,
            "worktree_file",
            loaded_file_count == 0,
            cx,
        );
        let worktree_filters_width = self
            .worktree_filter_bounds_ref
            .borrow()
            .as_ref()
            .map(|b| b.size.width)
            .unwrap_or(Pixels::MAX);
        let worktree_filters = self.commit_file_filter_tabs(
            crate::view::rows::FileListId::WorktreeFiles,
            "worktree_file",
            worktree_filters_width,
            worktree_counts,
            cx,
        );

        let files_body: AnyElement = if loaded_file_count == 0 {
            // Counts without files means the scan carrying them is still running.
            // Saying so beats an empty list that reads as "nothing changed" while
            // the header above it counts the changes.
            div()
                .debug_selector(|| "worktree_files_loading".to_string())
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child("Loading files…")
                .into_any_element()
        } else {
            Self::vertical_scroll_frame_content(
                theme,
                ("worktree_files_container", repo_id.0),
                ("worktree_files_scrollbar", repo_id.0),
                &self.worktree_files_scroll,
                self.changed_file_list(
                    repo_id,
                    crate::view::rows::FileListId::WorktreeFiles,
                    worktree_row_count,
                    cx,
                ),
            )
            .into_any_element()
        };

        div()
            .id("worktree_uncommitted_container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .child(header)
            .child(
                div()
                    .id("worktree_uncommitted_body")
                    .debug_selector(|| "worktree_uncommitted_body".to_string())
                    .relative()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .p_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .w_full()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_size(theme.ui_text(14.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .line_clamp(1)
                                    .child(SharedString::from(format!("{file_count} changed"))),
                            )
                            .child(worktree_controls),
                    )
                    .child({
                        let bounds = std::rc::Rc::clone(&self.worktree_filter_bounds_ref);
                        let pane = cx.weak_entity();
                        div()
                            .relative()
                            .w_full()
                            .min_w(px(0.0))
                            .on_children_prepainted(move |children, _window, app| {
                                let next = children.first().copied();
                                let mut measured = bounds.borrow_mut();
                                if *measured != next {
                                    *measured = next;
                                    // Cached panes must be notified after prepaint.
                                    let pane = pane.clone();
                                    app.defer(move |app| {
                                        let _ = pane.update(app, |_pane, cx| cx.notify());
                                    });
                                }
                            })
                            .child(visible_bounds_probe())
                            .child(worktree_filters)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .h_full()
                            .min_h(ui_scale.px(RANGE_FILES_SECTION_MIN_HEIGHT_PX))
                            .border_t_1()
                            .border_color(theme.colors.stroke.subtle)
                            .pt_2()
                            .child(files_body),
                    ),
            )
            .into_any_element()
    }
}
