use super::*;

impl MainPaneView {
    pub(super) fn ensure_diff_split_cache(&mut self) {
        if self.diff_split_row_provider.is_some() {
            return;
        }
        if self.diff_split_cache_len == self.diff_cache.len() && !self.diff_split_cache.is_empty() {
            return;
        }
        self.diff_split_cache_len = self.diff_cache.len();
        self.diff_split_cache = build_patch_split_rows(&self.diff_cache);
    }

    pub(super) fn diff_scrollbar_markers_patch(&self) -> Vec<components::ScrollbarMarker> {
        match self.diff_view {
            DiffViewMode::Inline => {
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(src_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    match self.patch_visual_line_kind(src_ix) {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                if self.diff_split_row_provider.is_some() && !self.diff_word_wrap {
                    let meta = self.patch_split_visible_meta_from_source();
                    debug_assert_eq!(meta.visible_indices.as_slice(), self.diff_visible_indices);
                    return scrollbar_markers_from_visible_flags(meta.visible_flags.as_slice());
                }
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(row_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    let Some(row) = self.patch_diff_split_row(row_ix) else {
                        return 0;
                    };
                    match &row {
                        PatchSplitRow::Aligned { .. } => {
                            match self.patch_split_visual_row_kind(&row) {
                                gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                                gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                                gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                                gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                            }
                        }
                        PatchSplitRow::Raw { .. } => 0,
                    }
                })
            }
        }
    }

    pub(super) fn collapsed_diff_hunk_marker_flag(hunk: CollapsedDiffHunk) -> u8 {
        match (hunk.has_additions, hunk.has_removals) {
            (true, true) => 3,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 0,
        }
    }

    pub(super) fn collapsed_diff_hunk_visible_file_bounds(
        &self,
        hunk_ix: usize,
        hunk: CollapsedDiffHunk,
    ) -> Option<(usize, usize)> {
        let mut visible_ix = *self.collapsed_diff_hunk_visible_indices.get(hunk_ix)?;
        while let Some(row) = self.collapsed_diff_visible_rows.get(visible_ix).copied() {
            match row {
                CollapsedDiffVisibleRow::HunkHeader { .. } => visible_ix += 1,
                CollapsedDiffVisibleRow::FileRow { row_ix } if row_ix < hunk.base_row_start => {
                    visible_ix += 1;
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } if row_ix == hunk.base_row_start => {
                    let end_ix = visible_ix
                        .saturating_add(
                            hunk.base_row_end_exclusive
                                .saturating_sub(hunk.base_row_start),
                        )
                        .min(self.collapsed_diff_visible_rows.len());
                    return (visible_ix < end_ix).then_some((visible_ix, end_ix));
                }
                CollapsedDiffVisibleRow::FileRow { .. } => return None,
            }
        }
        None
    }

    pub(super) fn diff_scrollbar_markers_collapsed(&self) -> Vec<components::ScrollbarMarker> {
        let ranges = self
            .collapsed_diff_hunks
            .iter()
            .enumerate()
            .filter_map(|(hunk_ix, hunk)| {
                let flag = Self::collapsed_diff_hunk_marker_flag(*hunk);
                let (start, end) = self.collapsed_diff_hunk_visible_file_bounds(hunk_ix, *hunk)?;
                Some((start, end, flag))
            })
            .collect::<Vec<_>>();
        if self.diff_word_wrap {
            return scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                let source_visible_ix = self
                    .diff_source_visible_ix_for_visible_ix(visible_ix)
                    .unwrap_or(visible_ix);
                ranges
                    .iter()
                    .find_map(|(start, end, flag)| {
                        (source_visible_ix >= *start && source_visible_ix < *end).then_some(*flag)
                    })
                    .unwrap_or(0)
            });
        }
        scrollbar_markers_from_visible_ranges(self.diff_visible_len(), ranges)
    }

    pub(in crate::view) fn compute_diff_scrollbar_markers(
        &self,
    ) -> Vec<components::ScrollbarMarker> {
        if self.is_collapsed_diff_projection_active() {
            return self.diff_scrollbar_markers_collapsed();
        }

        if !self.is_file_diff_view_active() {
            return self.diff_scrollbar_markers_patch();
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                if let Some(provider) = self.file_diff_inline_row_provider.as_ref()
                    && !self.diff_word_wrap
                {
                    return provider.scrollbar_markers();
                }
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(inline_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    match self.file_diff_inline_visual_kind(inline_ix) {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                if let Some(provider) = self.file_diff_row_provider.as_ref()
                    && !self.diff_word_wrap
                {
                    return provider.scrollbar_markers();
                }
                scrollbar_markers_from_flags(self.diff_visible_len(), |visible_ix| {
                    let Some(row_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return 0;
                    };
                    match self.file_diff_split_visual_kind(row_ix) {
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                    }
                })
            }
        }
    }

    pub(in crate::view) fn ensure_diff_visible_indices(&mut self) {
        let is_file_view = self.is_file_diff_view_active();
        let collapsed_projection_active = self.is_collapsed_diff_projection_active();
        let projection_rev = if collapsed_projection_active {
            self.diff_visible_projection_rev
        } else {
            0
        };
        let needs_collapsed_rebuild = collapsed_projection_active
            && (self.diff_visible_cache_projection_rev != projection_rev
                || self.diff_visible_view != self.diff_view
                || self.diff_visible_is_file_view != is_file_view);
        if needs_collapsed_rebuild {
            self.rebuild_collapsed_diff_projection();
        }

        let current_len = if collapsed_projection_active {
            self.collapsed_diff_visible_rows.len()
        } else if is_file_view {
            match self.diff_view {
                DiffViewMode::Inline => self.file_diff_inline_row_len(),
                DiffViewMode::Split => self.file_diff_split_row_len(),
            }
        } else {
            match self.diff_view {
                DiffViewMode::Inline => self.patch_diff_row_len(),
                DiffViewMode::Split => self.patch_diff_split_row_len(),
            }
        };

        if self.diff_visible_cache_len == current_len
            && self.diff_visible_view == self.diff_view
            && self.diff_visible_is_file_view == is_file_view
            && self.diff_visible_cache_projection_rev == projection_rev
        {
            return;
        }

        let preserve_horizontal_width = collapsed_projection_active
            && self.diff_visible_cache_projection_rev != u64::MAX
            && self.diff_visible_view == self.diff_view
            && self.diff_visible_is_file_view == is_file_view;

        self.diff_visible_cache_len = current_len;
        self.diff_visible_view = self.diff_view;
        self.diff_visible_is_file_view = is_file_view;
        self.diff_visible_cache_projection_rev = projection_rev;
        self.diff_wrap_visible_rows.clear();
        self.diff_wrap_visible_cache_key = None;
        if !preserve_horizontal_width {
            self.reset_diff_horizontal_scroll_state();
        }
        self.diff_visible_inline_map = None;
        self.diff_search_inline_patch_trigram_index = None;

        if collapsed_projection_active {
            self.diff_visible_indices.clear();
            self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
            if self.diff_search_has_query() {
                self.diff_search_recompute_matches_for_current_view_preserving_current();
            }
            return;
        }

        if is_file_view {
            self.diff_visible_indices = (0..current_len).collect();
            self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
            if self.diff_search_has_query() {
                self.diff_search_recompute_matches_for_current_view_preserving_current();
            }
            return;
        }

        let mut split_visible_flags: Option<Vec<u8>> = None;
        match self.diff_view {
            DiffViewMode::Inline => {
                if self.diff_hide_unified_header_for_src_ix.len() == current_len {
                    self.diff_visible_inline_map = Some(PatchInlineVisibleMap::from_hidden_flags(
                        self.diff_hide_unified_header_for_src_ix.as_slice(),
                    ));
                    self.diff_visible_indices = Vec::new();
                } else {
                    self.diff_visible_indices = self
                        .patch_diff_rows_slice(0, current_len)
                        .into_iter()
                        .enumerate()
                        .filter_map(|(ix, line)| {
                            (!should_hide_unified_diff_header_line(&line)).then_some(ix)
                        })
                        .collect();
                }
            }
            DiffViewMode::Split => {
                if self.diff_split_row_provider.is_some() {
                    let meta = self.patch_split_visible_meta_from_source();
                    debug_assert_eq!(meta.total_rows, current_len);
                    self.diff_visible_indices = meta.visible_indices;
                    split_visible_flags = Some(meta.visible_flags);
                } else {
                    self.ensure_diff_split_cache();

                    self.diff_visible_indices = self
                        .diff_split_cache
                        .iter()
                        .enumerate()
                        .filter_map(|(ix, row)| match row {
                            PatchSplitRow::Raw { src_ix, .. } => self
                                .diff_cache
                                .get(*src_ix)
                                .is_some_and(|line| !should_hide_unified_diff_header_line(line))
                                .then_some(ix),
                            PatchSplitRow::Aligned { .. } => Some(ix),
                        })
                        .collect();
                }
            }
        }

        self.diff_scrollbar_markers_cache = split_visible_flags
            .map(|flags| scrollbar_markers_from_visible_flags(flags.as_slice()))
            .unwrap_or_else(|| self.compute_diff_scrollbar_markers());

        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_for_current_view_preserving_current();
        }
    }
}
