use super::helpers::*;
use super::*;

/// What a left-click on a row should leave selected.
///
/// Clicking a hunk or file header is a deliberate "act on this whole region"
/// gesture, so it still selects the region it spans. Clicking a single line is
/// not: it is how you put the caret somewhere to read, and washing the whole row
/// for it both drowns the matching-delimiter highlight and says a range is
/// selected when the user only pointed at something.
///
/// The anchor is still set either way, so shift-click still extends from the
/// last place clicked and keyboard focus still has a row. Right-click sets its
/// own selection when the clicked row is not already inside one
/// (`diff_text.rs`), so the stage/discard actions still show their target.
fn row_click_selection_range(
    kind: DiffClickKind,
    clicked_visible_ix: usize,
    end: usize,
) -> Option<(usize, usize)> {
    match kind {
        DiffClickKind::Line => None,
        DiffClickKind::HunkHeader | DiffClickKind::FileHeader => Some((clicked_visible_ix, end)),
    }
}

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
        self.diff_selection_range = row_click_selection_range(kind, clicked_visible_ix, end);
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
        self.diff_selection_range = row_click_selection_range(kind, clicked_visible_ix, end);
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
        // A file diff has no hunk or file headers to click, so every row here is
        // a line -- and a plain line click anchors without selecting a range, as
        // [`row_click_selection_range`]'s `Line` arm says. Written out rather
        // than called, because passing a constant kind to a helper reads as a
        // decision being made per click when there is nothing to decide.
        self.diff_selection_range = None;
    }

    /// Full mode change blocks, as source-visible rows.
    fn file_change_blocks(&self) -> Vec<std::ops::Range<usize>> {
        let provider_blocks = match self.diff_view {
            DiffViewMode::Inline => self
                .file_diff_inline_row_provider
                .as_ref()
                .map(|provider| provider.change_blocks()),
            DiffViewMode::Split => self
                .file_diff_row_provider
                .as_ref()
                .map(|provider| provider.change_blocks()),
        };
        let blocks = provider_blocks.unwrap_or_else(|| {
            let len = match self.diff_view {
                DiffViewMode::Inline => self.file_diff_inline_row_len(),
                DiffViewMode::Split => self.file_diff_split_row_len(),
            };
            diff_navigation::change_block_ranges(len, |row_ix| self.file_diff_row_is_change(row_ix))
        });
        blocks
            .into_iter()
            .filter_map(|rows| {
                let start = self.diff_source_visible_ix_for_mapped_ix(rows.start)?;
                let last = self.diff_source_visible_ix_for_mapped_ix(rows.end - 1)?;
                Some(start..last + 1)
            })
            .collect()
    }

    /// Change blocks of the text diff, as source-visible rows.
    fn diff_change_blocks(&self) -> Vec<std::ops::Range<usize>> {
        if self.is_file_diff_view_active() {
            self.file_change_blocks()
        } else if self.is_collapsed_diff_projection_active() {
            self.collapsed_change_blocks()
        } else {
            self.patch_change_blocks()
        }
    }

    fn is_rendered_markdown_diff_active(&self) -> bool {
        self.is_markdown_preview_active() && !self.is_file_preview_active()
    }

    /// Remembers the block navigation landed on at visual row `target`, for
    /// the accent bar that marks it.
    fn focus_change_block_at(&mut self, target: usize) {
        let rows = if self.is_rendered_markdown_diff_active() {
            None
        } else {
            self.diff_change_blocks()
                .into_iter()
                .find(|rows| self.diff_visual_ix_for_source_visible_ix(rows.start) == target)
        };
        self.diff_focused_change_block = rows.map(|rows| DiffFocusedChangeBlock {
            anchor: target,
            sides: self.diff_block_change_sides(rows.clone()),
            rows,
            layout: self.diff_visible_layout_key(),
        });
    }

    pub(super) fn diff_source_visible_ix_for_mapped_ix(&self, mapped_ix: usize) -> Option<usize> {
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

    /// Translate a source row index into the row the list actually scrolls to,
    /// which differs once word wrap has split earlier rows.
    fn markdown_preview_visual_ix(&self, list: MarkdownPreviewList, row_ix: usize) -> usize {
        self.markdown_preview_wrap_plan(list)
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

    /// Change-navigation stops as visual indices: the first row of each
    /// change block, whatever the content mode.
    pub(in crate::view) fn diff_nav_entries(&self) -> Vec<usize> {
        if self.is_rendered_markdown_diff_active() {
            return self.markdown_preview_change_visible_indices();
        }
        self.diff_change_blocks()
            .into_iter()
            .map(|rows| self.diff_visual_ix_for_source_visible_ix(rows.start))
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

    /// Shared by the keys and the toolbar buttons so both agree on reachability.
    pub(in crate::view) fn diff_nav_prev_target_ix(&self, entries: &[usize]) -> Option<usize> {
        let current = self.diff_focus_visible_range().map(|(start, _end)| start);
        diff_navigation::diff_nav_prev_target(entries, current)
    }

    pub(in crate::view) fn diff_nav_next_target_ix(&self, entries: &[usize]) -> Option<usize> {
        let current = self.diff_focus_visible_range().map(|(_start, end)| end);
        diff_navigation::diff_nav_next_target(entries, current)
    }

    fn clear_diff_navigation_selection(&mut self) {
        self.clear_diff_text_selection();
        self.diff_selection_range = None;
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
        self.diff_jump(entries, true);
    }

    pub(in crate::view) fn diff_jump_next(&mut self) {
        let entries = self.diff_nav_entries();
        self.diff_jump(entries, false);
    }

    fn diff_jump(&mut self, entries: Vec<usize>, previous: bool) {
        if entries.is_empty() {
            return;
        }
        let target = if previous {
            self.diff_nav_prev_target_ix(&entries)
        } else {
            self.diff_nav_next_target_ix(&entries)
        };

        let Some(target) = target else {
            // Past the last stop: collapse to the edge being left. With nothing
            // focused, leave it so the first stop stays reachable.
            if let Some((start, end)) = self.diff_focus_visible_range() {
                let current = if previous { start } else { end };
                self.clear_diff_navigation_selection();
                self.diff_selection_anchor = Some(current);
            }
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        // Anchor only, like a line click: the block's bar and outline mark
        // where navigation is, so its first row gets no selection wash.
        self.clear_diff_navigation_selection();
        self.diff_selection_anchor = Some(target);
        self.focus_change_block_at(target);
    }

    pub(in crate::view) fn maybe_autoscroll_diff_to_first_change(&mut self) {
        if !self.diff_autoscroll_pending {
            return;
        }
        if self.diff_search_has_query() {
            self.diff_autoscroll_pending = false;
            return;
        }
        let visible_len = if self.is_rendered_markdown_diff_active() {
            self.markdown_preview_row_count().unwrap_or(0)
        } else {
            self.diff_visible_len()
        };
        if visible_len == 0 {
            return;
        }

        let entries = self.diff_nav_entries();
        let target = entries.first().copied().unwrap_or(0);

        // Strict: the scroll offset survives a target change, and centring
        // clamps to the top when the first block is near it, keeping a
        // collapsed hunk's header in view.
        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = None;
        if entries.is_empty() {
            self.diff_focused_change_block = None;
        } else {
            self.focus_change_block_at(target);
        }
        self.diff_autoscroll_pending = false;
    }
}
