use super::helpers::*;
use super::*;

impl MainPaneView {
    pub(in crate::view) fn handle_patch_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        if self.is_file_diff_view_active() {
            self.handle_file_diff_row_click(clicked_visible_ix, shift);
            return;
        }
        match self.diff_view {
            DiffViewMode::Inline => self.handle_diff_row_click(clicked_visible_ix, kind, shift),
            DiffViewMode::Split => self.handle_split_row_click(clicked_visible_ix, kind, shift),
        }
    }

    pub(super) fn handle_split_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        let list_len = self.diff_visible_len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);

        if self.is_collapsed_diff_projection_active()
            && matches!(kind, DiffClickKind::HunkHeader | DiffClickKind::FileHeader)
        {
            return;
        }

        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        let end = match kind {
            DiffClickKind::Line => clicked_visible_ix,
            DiffClickKind::HunkHeader => self
                .split_next_boundary_visible_ix(clicked_visible_ix, |row| {
                    matches!(
                        row,
                        PatchSplitRow::Raw {
                            click_kind: DiffClickKind::HunkHeader | DiffClickKind::FileHeader,
                            ..
                        }
                    )
                })
                .unwrap_or(list_len - 1),
            DiffClickKind::FileHeader => self
                .split_next_boundary_visible_ix(clicked_visible_ix, |row| {
                    matches!(
                        row,
                        PatchSplitRow::Raw {
                            click_kind: DiffClickKind::FileHeader,
                            ..
                        }
                    )
                })
                .unwrap_or(list_len - 1),
        };

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, end));
    }

    pub(super) fn handle_diff_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        let list_len = self.diff_visible_len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);

        if self.is_collapsed_diff_projection_active()
            && matches!(kind, DiffClickKind::HunkHeader | DiffClickKind::FileHeader)
        {
            return;
        }

        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        let end = match kind {
            DiffClickKind::Line => clicked_visible_ix,
            DiffClickKind::HunkHeader => self
                .diff_next_boundary_visible_ix(clicked_visible_ix, |src_ix| {
                    self.patch_diff_row(src_ix).is_some_and(|line| {
                        matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk)
                            || (matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                                && line.text.starts_with("diff --git "))
                    })
                })
                .unwrap_or(list_len - 1),
            DiffClickKind::FileHeader => self
                .diff_next_boundary_visible_ix(clicked_visible_ix, |src_ix| {
                    self.patch_diff_row(src_ix).is_some_and(|line| {
                        matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                            && line.text.starts_with("diff --git ")
                    })
                })
                .unwrap_or(list_len - 1),
        };

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, end));
    }

    pub(super) fn handle_file_diff_row_click(&mut self, clicked_visible_ix: usize, shift: bool) {
        let list_len = self.diff_visible_len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);
        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, clicked_visible_ix));
    }

    pub(super) fn file_change_visible_indices(&self) -> Vec<usize> {
        if !self.is_file_diff_view_active() {
            return Vec::new();
        }
        match self.diff_view {
            DiffViewMode::Inline => {
                if let Some(provider) = self.file_diff_inline_row_provider.as_ref() {
                    return provider
                        .change_visible_indices()
                        .into_iter()
                        .filter_map(|inline_ix| self.diff_visual_ix_for_mapped_ix(inline_ix))
                        .collect();
                }
                (0..self.file_diff_inline_row_len())
                    .filter_map(|inline_ix| {
                        let is_change =
                            matches!(
                                self.file_diff_inline_visual_kind(inline_ix),
                                gitcomet_core::domain::DiffLineKind::Add
                                    | gitcomet_core::domain::DiffLineKind::Remove
                            ) && self.file_diff_inline_row(inline_ix).is_some_and(|l| {
                                matches!(
                                    l.kind,
                                    gitcomet_core::domain::DiffLineKind::Add
                                        | gitcomet_core::domain::DiffLineKind::Remove
                                )
                            });
                        if !is_change {
                            return None;
                        }
                        self.diff_visual_ix_for_mapped_ix(inline_ix)
                    })
                    .collect()
            }
            DiffViewMode::Split => {
                if let Some(provider) = self.file_diff_row_provider.as_ref() {
                    return provider
                        .change_visible_indices()
                        .into_iter()
                        .filter_map(|row_ix| self.diff_visual_ix_for_mapped_ix(row_ix))
                        .collect();
                }
                (0..self.file_diff_split_row_len())
                    .filter_map(|row_ix| {
                        let is_change = !matches!(
                            self.file_diff_split_visual_kind(row_ix),
                            gitcomet_core::file_diff::FileDiffRowKind::Context
                        );
                        is_change.then(|| self.diff_visual_ix_for_mapped_ix(row_ix))?
                    })
                    .collect()
            }
        }
    }

    fn diff_source_visible_ix_for_mapped_ix(&self, mapped_ix: usize) -> Option<usize> {
        if let Some(map) = self.diff_visible_inline_map.as_ref() {
            return map.visible_ix_for_src_ix(mapped_ix);
        }
        if self.diff_visible_indices.is_empty()
            || self
                .diff_visible_indices
                .get(mapped_ix)
                .is_some_and(|visible_mapped_ix| *visible_mapped_ix == mapped_ix)
        {
            return Some(mapped_ix);
        }
        let visible_ix = self
            .diff_visible_indices
            .partition_point(|visible_mapped_ix| *visible_mapped_ix < mapped_ix);
        self.diff_visible_indices
            .get(visible_ix)
            .is_some_and(|visible_mapped_ix| *visible_mapped_ix == mapped_ix)
            .then_some(visible_ix)
    }

    fn diff_visual_ix_for_mapped_ix(&self, mapped_ix: usize) -> Option<usize> {
        self.diff_source_visible_ix_for_mapped_ix(mapped_ix)
            .map(|source_visible_ix| self.diff_visual_ix_for_source_visible_ix(source_visible_ix))
    }

    /// Translate a source row index into the row the list actually scrolls to,
    /// which differs once word wrap has split earlier rows.
    fn markdown_preview_visual_ix(&self, list: MarkdownPreviewList, row_ix: usize) -> usize {
        self.markdown_preview_wrap
            .plan(list)
            .map(|plan| plan.visual_ix_for_row(row_ix))
            .unwrap_or(row_ix)
    }

    fn markdown_preview_change_visible_indices(&self) -> Vec<usize> {
        let Loadable::Ready(preview) = &self.file_markdown_preview else {
            return Vec::new();
        };

        match self.diff_view {
            DiffViewMode::Inline => {
                diff_navigation::change_block_entries(preview.inline.rows.len(), |visible_ix| {
                    preview.inline.rows.get(visible_ix).is_some_and(|row| {
                        row.change_hint != crate::view::markdown_preview::MarkdownChangeHint::None
                    })
                })
                .into_iter()
                .map(|row_ix| self.markdown_preview_visual_ix(MarkdownPreviewList::Inline, row_ix))
                .collect()
            }
            DiffViewMode::Split => {
                let visible_len = preview.old.rows.len().max(preview.new.rows.len());
                diff_navigation::change_block_entries(visible_len, |visible_ix| {
                    preview.old.rows.get(visible_ix).is_some_and(|row| {
                        row.change_hint != crate::view::markdown_preview::MarkdownChangeHint::None
                    }) || preview.new.rows.get(visible_ix).is_some_and(|row| {
                        row.change_hint != crate::view::markdown_preview::MarkdownChangeHint::None
                    })
                })
                .into_iter()
                .map(|row_ix| self.markdown_preview_visual_ix(MarkdownPreviewList::Old, row_ix))
                .collect()
            }
        }
    }

    pub(in crate::view) fn patch_hunk_entries(&self) -> Vec<(usize, usize)> {
        if self.is_collapsed_diff_projection_active() {
            debug_assert_eq!(
                self.collapsed_diff_hunk_visible_indices.len(),
                self.collapsed_diff_hunks.len()
            );
            return self
                .collapsed_diff_hunk_visible_indices
                .iter()
                .enumerate()
                .filter_map(|(hunk_ix, &visible_ix)| {
                    self.collapsed_diff_hunks.get(hunk_ix).and_then(|hunk| {
                        (hunk.has_additions || hunk.has_removals).then_some((
                            self.diff_visual_ix_for_source_visible_ix(visible_ix),
                            hunk.src_ix,
                        ))
                    })
                })
                .collect();
        }

        let mut out = Vec::new();
        for visible_ix in 0..self.diff_visible_len() {
            let Some(ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                continue;
            };
            match self.diff_view {
                DiffViewMode::Inline => {
                    let Some(line) = self.patch_diff_row(ix) else {
                        continue;
                    };
                    if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) && {
                        let (has_additions, has_removals) = self.collapsed_hunk_change_summary(ix);
                        has_additions || has_removals
                    } {
                        out.push((visible_ix, ix));
                    }
                }
                DiffViewMode::Split => {
                    let Some(row) = self.patch_diff_split_row(ix) else {
                        continue;
                    };
                    if let PatchSplitRow::Raw {
                        src_ix,
                        click_kind: DiffClickKind::HunkHeader,
                    } = row
                    {
                        let (has_additions, has_removals) =
                            self.collapsed_hunk_change_summary(src_ix);
                        if !has_additions && !has_removals {
                            continue;
                        }
                        out.push((visible_ix, src_ix));
                    }
                }
            }
        }
        out
    }

    pub(in crate::view) fn diff_nav_entries(&self) -> Vec<usize> {
        if self.is_markdown_preview_active() && !self.is_file_preview_active() {
            return self.markdown_preview_change_visible_indices();
        }
        if self.is_file_diff_view_active() {
            return self.file_change_visible_indices();
        }
        if self.is_collapsed_diff_projection_active() {
            return self
                .collapsed_diff_hunk_visible_indices
                .iter()
                .enumerate()
                .filter_map(|(hunk_ix, visible_ix)| {
                    self.collapsed_diff_hunks.get(hunk_ix).and_then(|hunk| {
                        (hunk.has_additions || hunk.has_removals)
                            .then(|| self.diff_visual_ix_for_source_visible_ix(*visible_ix))
                    })
                })
                .collect();
        }
        self.patch_hunk_entries()
            .into_iter()
            .map(|(visible_ix, _)| visible_ix)
            .collect()
    }

    fn diff_row_focus_visible_range(&self) -> Option<(usize, usize)> {
        self.diff_selection_range
            .map(|(a, b)| (a.min(b), a.max(b)))
            .or_else(|| self.diff_selection_anchor.map(|ix| (ix, ix)))
    }

    pub(in crate::view) fn diff_focus_visible_range(&self) -> Option<(usize, usize)> {
        self.diff_text_selection_visible_range()
            .or_else(|| self.diff_row_focus_visible_range())
    }

    pub(in crate::view) fn diff_nav_prev_current_ix(&self) -> usize {
        self.diff_focus_visible_range()
            .map(|(start, _end)| start)
            .unwrap_or(0)
    }

    pub(in crate::view) fn diff_nav_next_current_ix(&self) -> usize {
        self.diff_focus_visible_range()
            .map(|(_start, end)| end)
            .unwrap_or(0)
    }

    fn clear_diff_navigation_selection(&mut self) {
        self.clear_diff_text_selection();
        self.diff_selection_range = None;
    }

    pub(in crate::view) fn scroll_diff_to_item(
        &mut self,
        target: usize,
        strategy: gpui::ScrollStrategy,
    ) {
        self.diff_scroll.scroll_to_item(target, strategy);
        if self.diff_view == DiffViewMode::Split {
            self.diff_split_right_scroll
                .scroll_to_item(target, strategy);
        }
    }

    pub(in crate::view) fn scroll_diff_to_item_strict(
        &mut self,
        target: usize,
        strategy: gpui::ScrollStrategy,
    ) {
        self.diff_scroll.scroll_to_item_strict(target, strategy);
        if self.diff_view == DiffViewMode::Split {
            self.diff_split_right_scroll
                .scroll_to_item_strict(target, strategy);
        }
    }

    fn has_active_diff_target(&self) -> bool {
        self.active_repo()
            .and_then(|repo| repo.diff_state.diff_target.as_ref())
            .is_some()
    }

    fn navigate_diff_change(&mut self, previous: bool, cx: &mut gpui::Context<Self>) -> bool {
        if !self.has_active_diff_target() {
            return false;
        }

        if self.is_conflict_resolver_active() {
            if self.is_conflict_rendered_preview_active() {
                return false;
            }
            if previous {
                self.conflict_jump_prev(cx);
            } else {
                self.conflict_jump_next(cx);
            }
            return true;
        }

        if self.is_file_preview_active() {
            return false;
        }

        if previous {
            self.diff_jump_prev();
        } else {
            self.diff_jump_next();
        }
        true
    }

    pub(in crate::view) fn navigate_prev_diff_change(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.navigate_diff_change(true, cx)
    }

    pub(in crate::view) fn navigate_next_diff_change(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.navigate_diff_change(false, cx)
    }

    pub(in crate::view) fn navigate_prev_search_match_or_diff_change(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.diff_search_active {
            self.diff_search_prev_match();
            return true;
        }
        self.navigate_prev_diff_change(cx)
    }

    pub(in crate::view) fn navigate_next_search_match_or_diff_change(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.diff_search_active {
            self.diff_search_next_match();
            return true;
        }
        self.navigate_next_diff_change(cx)
    }

    pub(in crate::view) fn diff_jump_prev(&mut self) {
        let entries = self.diff_nav_entries();
        let focus_range = self.diff_focus_visible_range();
        let current = focus_range.map(|(start, _end)| start).unwrap_or(0);
        if entries.is_empty() {
            return;
        }

        let Some(target) = diff_navigation::diff_nav_prev_target(&entries, current) else {
            if focus_range.is_some() {
                self.clear_diff_navigation_selection();
                self.diff_selection_range = Some((current, current));
            }
            self.diff_selection_anchor = Some(current);
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.clear_diff_navigation_selection();
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in crate::view) fn diff_jump_next(&mut self) {
        let entries = self.diff_nav_entries();
        let focus_range = self.diff_focus_visible_range();
        let current = focus_range.map(|(_start, end)| end).unwrap_or(0);
        if entries.is_empty() {
            return;
        }

        let Some(target) = diff_navigation::diff_nav_next_target(&entries, current) else {
            if focus_range.is_some() {
                self.clear_diff_navigation_selection();
                self.diff_selection_range = Some((current, current));
            }
            self.diff_selection_anchor = Some(current);
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.clear_diff_navigation_selection();
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in crate::view) fn maybe_autoscroll_diff_to_first_change(&mut self) {
        if !self.diff_autoscroll_pending {
            return;
        }
        if self.diff_search_has_query() {
            self.diff_autoscroll_pending = false;
            return;
        }
        let visible_len = if self.is_markdown_preview_active() && !self.is_file_preview_active() {
            self.markdown_preview_row_count().unwrap_or(0)
        } else {
            self.diff_visible_len()
        };
        if visible_len == 0 {
            return;
        }

        let entries = self.diff_nav_entries();
        let target = entries.first().copied().unwrap_or(0);

        self.scroll_diff_to_item(target, gpui::ScrollStrategy::Top);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
        self.diff_autoscroll_pending = false;
    }
}
