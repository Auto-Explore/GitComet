//! Virtualized submodule summary document. Every changed file is a list item;
//! headers are separate items, so a large section never builds its whole body.
#[cfg(test)]
mod tests;
use super::*;
use gitcomet_core::domain::{
    SubmoduleDiffRangeKind, SubmoduleDiffSummary, SubmoduleDiffSummaryMode, SubmoduleInnerChange,
    SubmoduleStatus,
};
use gitcomet_state::model::{
    SubmoduleChangeSection as ChangeSection, submodule_inline_diff_entries,
    submodule_inline_diff_target,
};

#[derive(Clone, Copy)]
enum SummaryRow {
    Header,
    RangeHeader(usize),
    RangeFooter(usize),
    EmptyRange(usize),
    LiveHeader(ChangeSection),
    Change {
        section: ChangeSection,
        index: usize,
        inline_index: Option<usize>,
    },
}

struct SummaryRows {
    summary: Arc<SubmoduleDiffSummary>,
    rows: Vec<SummaryRow>,
}

/// Carry a scroll position across a rebuild. `ListState::scroll_to` keeps an
/// in-range `offset_in_item` verbatim, so only `Change` rows may carry one:
/// they are the single fixed-height kind, while headers grow and shrink with
/// their status and hashes.
fn restored_scroll_top(
    mut top: gpui::ListOffset,
    previous: &SummaryRows,
    next: &SummaryRows,
) -> gpui::ListOffset {
    let ix = top.item_ix.min(next.rows.len().saturating_sub(1));
    let fixed_height = previous
        .rows
        .get(top.item_ix)
        .zip(next.rows.get(ix))
        .is_some_and(|(before, after)| {
            matches!(before, SummaryRow::Change { .. })
                && matches!(after, SummaryRow::Change { .. })
        });
    top.item_ix = ix;
    if !fixed_height {
        top.offset_in_item = px(0.0);
    }
    top
}

/// Height of one changed-file row. The list windows on it, so it has to be the
/// number the row itself lays out at -- density and UI font size move both, and
/// layout snaps the result to whole pixels.
fn summary_change_row_height(scale: crate::ui_scale::UiScale) -> gpui::Pixels {
    px(f32::from(scale.row_height(28.0, 32.0)).round())
}

impl SummaryRows {
    fn new(summary: Arc<SubmoduleDiffSummary>) -> Self {
        let mut rows = vec![SummaryRow::Header];
        let mut inline_index = 0;
        for (slot, range) in summary.ranges.iter().enumerate() {
            rows.push(SummaryRow::RangeHeader(slot));
            let navigable = range.from.is_some() && range.to.is_some();
            if range.changes.is_empty() {
                rows.push(SummaryRow::EmptyRange(slot));
            }
            for index in 0..range.changes.len() {
                rows.push(SummaryRow::Change {
                    section: ChangeSection::Range(slot),
                    index,
                    inline_index: navigable.then_some(inline_index),
                });
                if navigable {
                    inline_index += 1;
                }
            }
            rows.push(SummaryRow::RangeFooter(slot));
        }
        if summary.mode == SubmoduleDiffSummaryMode::Worktree {
            for (section, changes) in [
                (ChangeSection::LiveStaged, &summary.live_staged),
                (ChangeSection::LiveUnstaged, &summary.live_unstaged),
            ] {
                if changes.is_empty() {
                    continue;
                }
                rows.push(SummaryRow::LiveHeader(section));
                for index in 0..changes.len() {
                    rows.push(SummaryRow::Change {
                        section,
                        index,
                        inline_index: Some(inline_index),
                    });
                    inline_index += 1;
                }
            }
        }
        Self { summary, rows }
    }
}

pub(in crate::view) struct SubmoduleSummaryCache {
    repo_id: RepoId,
    target: DiffTarget,
    revision: u64,
    submodules_rev: u64,
    status: Option<SubmoduleStatus>,
    presentation: Arc<SummaryRows>,
    submodule_repo_path: Arc<std::path::PathBuf>,
    pub(in crate::view) scroll: gpui::ListState,
    row_height: gpui::Pixels,
    #[cfg(test)]
    pub(in crate::view) rendered_rows: usize,
    /// Counts trips through the rebuild path, so a test can prove a redraw
    /// reuses the cache rather than re-deriving it.
    #[cfg(test)]
    pub(in crate::view) rebuilds: usize,
}

impl MainPaneView {
    /// The one owner of `submodule_summary_cache`, run once per frame before
    /// anything renders: it holds a row per changed file, so it is released as
    /// soon as the pane leaves the repository and target it was built for.
    ///
    /// A `Loading`/`Error` refresh of that target keeps it, and so does an
    /// inline diff opened from one of its rows -- both come back to these rows,
    /// at the position they left.
    pub(in crate::view) fn release_stale_submodule_summary_cache(&mut self) {
        let Some(cache) = self.submodule_summary_cache.as_ref() else {
            return;
        };
        let still_shown = self.active_repo().is_some_and(|repo| {
            repo.id == cache.repo_id
                && !matches!(repo.diff_state.submodule_summary, Loadable::NotLoaded)
                && repo.diff_state.diff_target.as_ref() == Some(&cache.target)
        });
        if !still_shown {
            self.submodule_summary_cache = None;
        }
    }

    pub(in crate::view) fn render_submodule_summary(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let Some(repo) = self.active_repo() else {
            return components::empty_state(theme, "Submodule", "No repository.")
                .into_any_element();
        };
        let Some(target) = repo.diff_state.diff_target.clone() else {
            return components::empty_state(theme, "Submodule", "No submodule selected.")
                .into_any_element();
        };
        // Only a file-shaped target names a submodule; a commit range has none
        // to summarize. The reducer leaves those at `NotLoaded` -- this makes it
        // the pane's invariant too. No cache is ever built for one, so none can
        // need releasing here.
        if !matches!(
            &target,
            DiffTarget::WorkingTree { .. } | DiffTarget::Commit { path: Some(_), .. }
        ) {
            return components::empty_state(theme, "Submodule", "No submodule selected.")
                .into_any_element();
        }
        let summary = match &repo.diff_state.submodule_summary {
            Loadable::Ready(summary) => Arc::clone(summary),
            state => {
                let message = match state {
                    Loadable::Error(error) => error.clone(),
                    _ => "Loading submodule summary…".to_string(),
                };
                // Any cache surviving here is this target's own; see
                // `release_stale_submodule_summary_cache`.
                return components::empty_state(theme, "Submodule", message).into_any_element();
            }
        };
        let repo_id = repo.id;
        let revision = repo.diff_state.submodule_summary_rev;
        let submodules_rev = repo.submodules_rev;
        let status = self
            .submodule_summary_cache
            .as_ref()
            .filter(|cache| {
                cache.repo_id == repo_id
                    && cache.target == target
                    && cache.revision == revision
                    && cache.submodules_rev == submodules_rev
                    && Arc::ptr_eq(&cache.presentation.summary, &summary)
            })
            .map(|cache| cache.status)
            .unwrap_or_else(|| {
                summary.status.or_else(|| match &repo.submodules {
                    Loadable::Ready(submodules) => submodules
                        .iter()
                        .find(|entry| entry.path == summary.path)
                        .map(|entry| entry.status),
                    _ => None,
                })
            });
        let scale = crate::ui_scale::UiScale::current(cx);
        let row_height = summary_change_row_height(scale);
        let same_target = self
            .submodule_summary_cache
            .as_ref()
            .is_some_and(|cache| cache.repo_id == repo_id && cache.target == target);
        let reusable = same_target
            && self.submodule_summary_cache.as_ref().is_some_and(|cache| {
                cache.revision == revision && Arc::ptr_eq(&cache.presentation.summary, &summary)
            });
        // Joined only when the cache is rebuilt: on the reuse path every frame
        // would otherwise allocate a `PathBuf` and throw it away.
        let rebuilt_path = (!reusable).then(|| Arc::new(repo.spec.workdir.join(&summary.path)));
        #[cfg(test)]
        let rebuilds = self
            .submodule_summary_cache
            .as_ref()
            .map_or(0, |cache| cache.rebuilds);
        if let Some(submodule_repo_path) = rebuilt_path {
            let presentation = Arc::new(SummaryRows::new(summary));
            let previous = same_target.then(|| {
                let cache = self.submodule_summary_cache.as_ref().unwrap();
                (
                    cache.scroll.logical_scroll_top(),
                    Arc::clone(&cache.presentation),
                )
            });
            let scroll =
                gpui::ListState::new(presentation.rows.len(), gpui::ListAlignment::Top, px(280.0))
                    .with_uniform_item_height(row_height);
            if let Some((top, previous_rows)) = previous {
                scroll.scroll_to(restored_scroll_top(top, &previous_rows, &presentation));
            }
            self.submodule_summary_cache = Some(SubmoduleSummaryCache {
                repo_id,
                target,
                revision,
                submodules_rev,
                status,
                presentation,
                submodule_repo_path,
                scroll,
                row_height,
                #[cfg(test)]
                rendered_rows: 0,
                #[cfg(test)]
                rebuilds: rebuilds + 1,
            });
        }
        let cache = self
            .submodule_summary_cache
            .as_mut()
            .expect("summary cached");
        cache.submodules_rev = submodules_rev;
        cache.status = status;
        if cache.row_height != row_height {
            let top = cache.scroll.logical_scroll_top();
            cache
                .scroll
                .reset_with_uniform_height(cache.presentation.rows.len(), row_height);
            cache.scroll.scroll_to(top);
            cache.row_height = row_height;
        }
        #[cfg(test)]
        {
            cache.rendered_rows = 0;
        }
        let scroll = cache.scroll.clone();
        let list = gpui::list(
            scroll.clone(),
            cx.processor(|this, ix, _window, cx| this.render_submodule_summary_row(ix, cx)),
        )
        .size_full();
        div()
            .id("submodule_summary_scroll")
            .relative()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            .bg(theme.colors.surface.canvas)
            .child(list)
            .child(
                components::Scrollbar::new("submodule_summary_scrollbar", scroll)
                    .auto_hide()
                    .render(theme),
            )
            .into_any_element()
    }

    fn render_submodule_summary_row(
        &mut self,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let Some(cache) = self.submodule_summary_cache.as_mut() else {
            return div().into_any_element();
        };
        #[cfg(test)]
        {
            cache.rendered_rows += 1;
        }
        let presentation = Arc::clone(&cache.presentation);
        let Some(row) = presentation.rows.get(ix).copied() else {
            return div().into_any_element();
        };
        let repo_id = cache.repo_id;
        let submodule_repo_path = Arc::clone(&cache.submodule_repo_path);
        let status = cache.status;
        let selected_area = match cache.target {
            DiffTarget::WorkingTree { area, .. } => Some(area),
            _ => None,
        };
        let summary = &presentation.summary;
        let theme = self.theme;
        match row {
            SummaryRow::Header => self.render_submodule_summary_header(
                repo_id,
                summary,
                &submodule_repo_path,
                status,
                theme,
                cx,
            ),
            SummaryRow::RangeHeader(slot) => {
                self.render_submodule_range_header(slot, summary, selected_area, theme, cx)
            }
            SummaryRow::RangeFooter(slot) => {
                summary_range_surface(theme, summary.ranges[slot].kind, selected_area)
                    .h(px(8.0))
                    .border_b_1()
                    .rounded_b(px(theme.radii.row))
                    .mb_2()
                    .into_any_element()
            }
            SummaryRow::EmptyRange(slot) => {
                summary_range_surface(theme, summary.ranges[slot].kind, selected_area)
                    .px_2()
                    .py_1()
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child("No inner changes.")
                    .into_any_element()
            }
            SummaryRow::LiveHeader(section) => div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(match section {
                    ChangeSection::LiveStaged => "Uncommitted inner staged",
                    ChangeSection::LiveUnstaged => "Uncommitted inner unstaged",
                    // `SummaryRows` emits `LiveHeader` for the live halves only.
                    ChangeSection::Range(_) => "",
                })
                .into_any_element(),
            SummaryRow::Change {
                section,
                index,
                inline_index,
            } => {
                let changes = match section {
                    ChangeSection::Range(slot) => &summary.ranges[slot].changes,
                    ChangeSection::LiveStaged => &summary.live_staged,
                    ChangeSection::LiveUnstaged => &summary.live_unstaged,
                };
                let row = self.render_submodule_change(
                    repo_id,
                    ix,
                    &changes[index],
                    summary,
                    &submodule_repo_path,
                    section,
                    index,
                    inline_index,
                    theme,
                    cx,
                );
                match section {
                    ChangeSection::Range(slot) => {
                        summary_range_surface(theme, summary.ranges[slot].kind, selected_area)
                            .px_2()
                            .child(row)
                            .into_any_element()
                    }
                    _ => row,
                }
            }
        }
    }
    fn prepare_submodule_hash_input(
        &mut self,
        slot: usize,
        value: String,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let Some(input) = self
            .submodule_hash_inputs
            .get(slot % self.submodule_hash_inputs.len().max(1))
            .cloned()
        else {
            return self.diff_raw_input.clone();
        };
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(value, cx);
            input.set_read_only(true, cx);
        });
        input
    }

    fn render_submodule_summary_header(
        &mut self,
        repo_id: RepoId,
        summary: &SubmoduleDiffSummary,
        submodule_repo_path: &Arc<std::path::PathBuf>,
        summary_status: Option<SubmoduleStatus>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let can_open = summary.checkout_available
            && !matches!(
                summary_status,
                Some(
                    SubmoduleStatus::NotInitialized
                        | SubmoduleStatus::MergeConflict
                        | SubmoduleStatus::MissingMapping
                )
            );
        let can_change_pointer = summary.mode == SubmoduleDiffSummaryMode::Worktree && can_open;
        let show_load = summary.mode == SubmoduleDiffSummaryMode::Worktree
            && (summary_status == Some(SubmoduleStatus::NotInitialized)
                || (summary_status.is_none() && !summary.checkout_available));
        let open_path = Arc::clone(submodule_repo_path);
        let summary_path = summary.path.clone();
        let scale = crate::ui_scale::UiScale::current(cx);
        let status_badge = |status: SubmoduleStatus| {
            let (label, color) = match status {
                SubmoduleStatus::UpToDate => ("Loaded", theme.colors.status.success.foreground),
                SubmoduleStatus::NotInitialized => (
                    "Not loaded",
                    with_alpha(
                        theme.colors.foreground.secondary,
                        if theme.is_dark { 0.86 } else { 0.94 },
                    ),
                ),
                SubmoduleStatus::HeadMismatch => {
                    ("Head mismatch", theme.colors.status.warning.foreground)
                }
                SubmoduleStatus::MergeConflict => {
                    ("Conflict", theme.colors.status.danger.foreground)
                }
                SubmoduleStatus::MissingMapping => {
                    ("Missing mapping", theme.colors.status.danger.foreground)
                }
                SubmoduleStatus::Unknown(_) => ("Unknown", theme.colors.foreground.secondary),
            };

            div()
                .px_1p5()
                .h(scale.px(20.0))
                .rounded(px(theme.radii.row))
                .border_1()
                .border_color(with_alpha(color, if theme.is_dark { 0.45 } else { 0.32 }))
                .bg(with_alpha(color, if theme.is_dark { 0.14 } else { 0.10 }))
                .text_size(theme.ui_text(12.0))
                .text_color(color)
                .child(label)
        };

        div()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(crate::view::icons::svg_icon(
                                "icons/box.svg",
                                match summary_status.unwrap_or(SubmoduleStatus::UpToDate) {
                                    SubmoduleStatus::NotInitialized => with_alpha(
                                        theme.colors.foreground.secondary,
                                        if theme.is_dark { 0.82 } else { 0.94 },
                                    ),
                                    SubmoduleStatus::HeadMismatch => {
                                        theme.colors.status.warning.foreground
                                    }
                                    SubmoduleStatus::MergeConflict
                                    | SubmoduleStatus::MissingMapping => {
                                        theme.colors.status.danger.foreground
                                    }
                                    SubmoduleStatus::UpToDate | SubmoduleStatus::Unknown(_) => {
                                        theme.colors.accent.foreground
                                    }
                                },
                                px(14.0),
                            ))
                            .child(
                                div()
                                    .text_size(theme.ui_text(14.0))
                                    .font_weight(FontWeight::BOLD)
                                    .child(summary.path.display().to_string()),
                            )
                            .when_some(summary_status, |this, status| {
                                this.child(status_badge(status))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                components::Button::new("submodule_summary_open", "Open submodule")
                                    .style(components::ButtonStyle::Outlined)
                                    .disabled(!can_open)
                                    .on_click(theme, cx, move |this, _e, _w, cx| {
                                        if can_open {
                                            this.store.dispatch(Msg::OpenRepo(
                                                open_path.as_ref().clone(),
                                            ));
                                            cx.notify();
                                        }
                                    }),
                            )
                            .when(show_load, |row| {
                                let load_path = summary.path.clone();
                                row.child(
                                    components::Button::new(
                                        "submodule_summary_load",
                                        "Load submodule",
                                    )
                                    .style(components::ButtonStyle::Outlined)
                                    .on_click(
                                        theme,
                                        cx,
                                        move |this, _e, _w, cx| {
                                            this.store.dispatch(Msg::LoadSubmodule {
                                                repo_id,
                                                path: load_path.clone(),
                                            });
                                            cx.notify();
                                        },
                                    ),
                                )
                            })
                            .child(
                                components::Button::new(
                                    "submodule_summary_change_pointer",
                                    "Change pointer…",
                                )
                                .style(components::ButtonStyle::Outlined)
                                .disabled(!can_change_pointer)
                                .on_click(
                                    theme,
                                    cx,
                                    move |this, e, window, cx| {
                                        if !can_change_pointer {
                                            return;
                                        }
                                        this.open_popover_at(
                                            PopoverKind::submodule(
                                                repo_id,
                                                SubmodulePopoverKind::ChangePointerPrompt {
                                                    path: summary_path.clone(),
                                                },
                                            ),
                                            e.position(),
                                            window,
                                            cx,
                                        );
                                        cx.notify();
                                    },
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_submodule_range_header(
        &mut self,
        slot: usize,
        summary: &SubmoduleDiffSummary,
        selected_area: Option<DiffArea>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let range = &summary.ranges[slot];
        let emphasized = summary_range_emphasized(range.kind, selected_area);
        let changed = range.from != range.to;
        let range_hash_input = self.prepare_submodule_hash_input(
            slot,
            format!(
                "{} -> {}",
                full_submodule_hash_opt(range.from.as_ref()),
                full_submodule_hash_opt(range.to.as_ref())
            ),
            theme,
            cx,
        );
        let mut section = div()
            .id(format!("submodule_range_{:?}", range.kind))
            .px_2()
            .py_2()
            .rounded_t(px(theme.radii.row))
            .border_t_1()
            .border_x_1()
            .border_color(if emphasized {
                theme.colors.interaction.pressed_background
            } else {
                theme.colors.stroke.default
            })
            .bg(if emphasized {
                with_alpha(
                    theme.colors.interaction.hover_background,
                    if theme.is_dark { 0.28 } else { 0.48 },
                )
            } else {
                gpui::rgba(0x00000000)
            })
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(submodule_range_label(range.kind)),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .text_color(if changed {
                                theme.colors.foreground.primary
                            } else {
                                theme.colors.foreground.secondary
                            })
                            .child(format!(
                                "{} -> {}",
                                short_submodule_hash_opt(range.from.as_ref()),
                                short_submodule_hash_opt(range.to.as_ref())
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child("Hashes"),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .child(range_hash_input),
                    ),
            );
        if let Some(reason) = range.unavailable_reason.as_ref() {
            section = section.child(
                div()
                    .px_2()
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(reason.clone()),
            );
        }
        section
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child("Changes between hashes"),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_submodule_change(
        &mut self,
        repo_id: RepoId,
        row_ix: usize,
        change: &SubmoduleInnerChange,
        summary: &Arc<SubmoduleDiffSummary>,
        submodule_repo_path: &Arc<std::path::PathBuf>,
        section: ChangeSection,
        change_ix: usize,
        inline_selected_ix: Option<usize>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let submodule_repo_path = Arc::clone(submodule_repo_path);
        let change_row_icon = |kind: FileStatusKind| match kind {
            FileStatusKind::Untracked | FileStatusKind::Added => {
                ("icons/plus.svg", theme.colors.status.success.foreground)
            }
            FileStatusKind::Modified => {
                ("icons/pencil.svg", theme.colors.status.warning.foreground)
            }
            FileStatusKind::Deleted => ("icons/minus.svg", theme.colors.status.danger.foreground),
            FileStatusKind::Renamed => ("icons/swap.svg", theme.colors.accent.foreground),
            FileStatusKind::Conflicted => {
                ("icons/warning.svg", theme.colors.status.danger.foreground)
            }
        };

        let (icon, icon_color) = change_row_icon(change.kind);
        let additions = change
            .additions
            .map(|value| format!("+{value}"))
            .unwrap_or_else(|| "—".to_string());
        let deletions = change
            .deletions
            .map(|value| format!("-{value}"))
            .unwrap_or_else(|| "—".to_string());
        let change_path = change.path.clone();
        // `None` exactly when this row has nothing to open: a pointer range
        // missing an endpoint has no two sides to diff.
        let target = submodule_inline_diff_target(summary, section, change_ix);
        let repo_path_for_click = submodule_repo_path.clone();
        let repo_path_for_menu = submodule_repo_path.clone();
        let summary_path_for_inline = summary.path.clone();
        // Built by the click, not by the frame: one entry per changed file.
        let summary_for_click = Arc::clone(summary);
        let context_menu_path = change_path.clone();

        let mut row = div()
            .id(("submodule_change", row_ix))
            .debug_selector(move || format!("submodule_change_{row_ix}"))
            .h(summary_change_row_height(crate::ui_scale::UiScale::current(cx)))
            .px_2()
            .py_1()
            .rounded(px(theme.radii.row))
            .flex()
            .items_center()
            .gap_2()
            .child(crate::view::icons::svg_icon(icon, icon_color, px(12.0)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .child(change_path.display().to_string()),
            )
            .child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.status.success.foreground)
                    .child(additions),
            )
            .child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.status.danger.foreground)
                    .child(deletions),
            );

        if let Some(target) = target {
            row = row
                .cursor(CursorStyle::PointingHand)
                .hover(move |row| row.bg(theme.colors.interaction.hover_background))
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    let selected_ix = inline_selected_ix.unwrap_or(0);
                    this.store.dispatch(Msg::OpenInlineSubmoduleDiff {
                        repo_id,
                        origin: gitcomet_state::model::ForeignDiffOrigin::Submodule,
                        submodule_repo_path: repo_path_for_click.as_ref().clone(),
                        parent_submodule_path: summary_path_for_inline.clone(),
                        entries: submodule_inline_diff_entries(&summary_for_click).into(),
                        selected_ix,
                    });
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.activate_context_menu_invoker(
                            format!(
                                "submodule_inner_diff_menu_{}_{}",
                                repo_id.0,
                                context_menu_path.display()
                            )
                            .into(),
                            cx,
                        );
                        this.open_popover_at(
                            PopoverKind::SubmoduleInnerDiffMenu {
                                repo_id,
                                submodule_repo_path: repo_path_for_menu.as_ref().clone(),
                                target: target.clone(),
                            },
                            e.position,
                            window,
                            cx,
                        );
                    }),
                );
        }

        row.into_any_element()
    }
}

fn summary_range_emphasized(kind: SubmoduleDiffRangeKind, selected_area: Option<DiffArea>) -> bool {
    match kind {
        SubmoduleDiffRangeKind::StagedPointer => selected_area == Some(DiffArea::Staged),
        SubmoduleDiffRangeKind::UnstagedPointer => selected_area == Some(DiffArea::Unstaged),
        SubmoduleDiffRangeKind::CommitHistory => true,
    }
}

fn summary_range_surface(
    theme: AppTheme,
    kind: SubmoduleDiffRangeKind,
    selected_area: Option<DiffArea>,
) -> gpui::Div {
    let emphasized = summary_range_emphasized(kind, selected_area);
    div()
        .border_x_1()
        .border_color(if emphasized {
            theme.colors.interaction.pressed_background
        } else {
            theme.colors.stroke.default
        })
        .bg(if emphasized {
            with_alpha(
                theme.colors.interaction.hover_background,
                if theme.is_dark { 0.28 } else { 0.48 },
            )
        } else {
            gpui::rgba(0x00000000)
        })
}
fn short_submodule_hash(commit_id: &CommitId) -> String {
    let raw = commit_id.as_ref();
    raw.chars().take(12).collect()
}

fn short_submodule_hash_opt(commit_id: Option<&CommitId>) -> String {
    commit_id
        .map(short_submodule_hash)
        .unwrap_or_else(|| "missing".to_string())
}

fn full_submodule_hash_opt(commit_id: Option<&CommitId>) -> String {
    commit_id
        .map(|commit_id| commit_id.as_ref().to_string())
        .unwrap_or_else(|| "missing".to_string())
}

fn submodule_range_label(kind: SubmoduleDiffRangeKind) -> &'static str {
    match kind {
        SubmoduleDiffRangeKind::StagedPointer => "Committed -> Index",
        SubmoduleDiffRangeKind::UnstagedPointer => "Index -> Checked out",
        SubmoduleDiffRangeKind::CommitHistory => "Parent -> Commit",
    }
}
