use super::*;

impl MainPaneView {
    pub(super) fn current_file_diff_line_to_row_maps(
        &self,
    ) -> (&[Option<usize>], &[Option<usize>], usize) {
        match self.diff_view {
            DiffViewMode::Inline => (
                self.file_diff_old_line_to_inline_row.as_ref(),
                self.file_diff_new_line_to_inline_row.as_ref(),
                self.file_diff_inline_row_len(),
            ),
            DiffViewMode::Split => (
                self.file_diff_old_line_to_row.as_ref(),
                self.file_diff_new_line_to_row.as_ref(),
                self.file_diff_split_row_len(),
            ),
        }
    }

    pub(super) fn collapsed_hunk_row_range_for_parsed(
        &self,
        parsed: &crate::view::diff_utils::ParsedHunkHeader,
    ) -> Option<(usize, usize)> {
        let (old_line_to_row, new_line_to_row, _row_count) =
            self.current_file_diff_line_to_row_maps();

        let map_range_start = |line_to_row: &[Option<usize>], start_line: u32, line_count: u32| {
            (line_count > 0)
                .then_some(start_line)
                .filter(|line| *line > 0)
                .and_then(|line| usize::try_from(line.saturating_sub(1)).ok())
                .and_then(|line_ix| line_to_row.get(line_ix).copied().flatten())
        };
        let map_range_end = |line_to_row: &[Option<usize>], start_line: u32, line_count: u32| {
            (line_count > 0)
                .then_some(start_line.saturating_add(line_count).saturating_sub(1))
                .filter(|line| *line > 0)
                .and_then(|line| usize::try_from(line.saturating_sub(1)).ok())
                .and_then(|line_ix| line_to_row.get(line_ix).copied().flatten())
                .map(|row_ix| row_ix.saturating_add(1))
        };

        let start = [
            map_range_start(
                old_line_to_row,
                parsed.old_start_line,
                parsed.old_line_count,
            ),
            map_range_start(
                new_line_to_row,
                parsed.new_start_line,
                parsed.new_line_count,
            ),
        ]
        .into_iter()
        .flatten()
        .min()?;

        let end = [
            map_range_end(
                old_line_to_row,
                parsed.old_start_line,
                parsed.old_line_count,
            ),
            map_range_end(
                new_line_to_row,
                parsed.new_start_line,
                parsed.new_line_count,
            ),
        ]
        .into_iter()
        .flatten()
        .max()?;

        (start < end).then_some((start, end))
    }

    pub(in crate::view) fn collapsed_hunk_change_summary(&self, src_ix: usize) -> (bool, bool) {
        let mut has_additions = false;
        let mut has_removals = false;

        for candidate_ix in src_ix.saturating_add(1)..self.patch_diff_row_len() {
            let click_kind = self
                .diff_click_kinds
                .get(candidate_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if click_kind != DiffClickKind::Line {
                break;
            }

            match self.patch_visual_line_kind(candidate_ix) {
                gitcomet_core::domain::DiffLineKind::Add => has_additions = true,
                gitcomet_core::domain::DiffLineKind::Remove => has_removals = true,
                gitcomet_core::domain::DiffLineKind::Context
                | gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => {}
            }

            if has_additions && has_removals {
                break;
            }
        }

        (has_additions, has_removals)
    }

    pub(super) fn reindex_collapsed_diff_hunks(&mut self) {
        self.collapsed_diff_hunk_ix_by_src_ix.clear();
        for (hunk_ix, hunk) in self.collapsed_diff_hunks.iter().enumerate() {
            let previous = self
                .collapsed_diff_hunk_ix_by_src_ix
                .insert(hunk.src_ix, hunk_ix);
            debug_assert!(previous.is_none());
        }
    }

    pub(super) fn ensure_collapsed_diff_hunk_index(&mut self) {
        if self.collapsed_diff_hunk_ix_by_src_ix.len() != self.collapsed_diff_hunks.len() {
            self.reindex_collapsed_diff_hunks();
        }
    }

    pub(super) fn ensure_collapsed_diff_hunks_initialized(&mut self) {
        if !self.collapsed_diff_hunks.is_empty() {
            self.ensure_collapsed_diff_hunk_index();
            return;
        }

        for src_ix in 0..self.patch_diff_row_len() {
            let click_kind = self
                .diff_click_kinds
                .get(src_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if click_kind != DiffClickKind::HunkHeader {
                continue;
            }

            let Some(line) = self.patch_diff_row(src_ix) else {
                continue;
            };
            let Some(parsed) =
                crate::view::diff_utils::parse_unified_hunk_header_for_display(line.text.as_ref())
            else {
                continue;
            };
            let Some((base_row_start, base_row_end_exclusive)) =
                self.collapsed_hunk_row_range_for_parsed(&parsed)
            else {
                continue;
            };
            let (has_additions, has_removals) = self.collapsed_hunk_change_summary(src_ix);
            let reveal = self
                .collapsed_diff_reveals
                .get(&src_ix)
                .copied()
                .unwrap_or_default();
            self.collapsed_diff_hunks.push(CollapsedDiffHunk {
                src_ix,
                base_row_start,
                base_row_end_exclusive,
                has_additions,
                has_removals,
                reveal_up_lines: reveal.up_lines,
                reveal_down_lines: reveal.down_lines,
            });
        }
        self.reindex_collapsed_diff_hunks();
    }

    pub(super) fn persist_collapsed_diff_hunk_reveal(&mut self, hunk_ix: usize) {
        let Some(hunk) = self.collapsed_diff_hunks.get(hunk_ix).copied() else {
            return;
        };
        let reveal = CollapsedDiffReveal {
            up_lines: hunk.reveal_up_lines,
            down_lines: hunk.reveal_down_lines,
        };
        if reveal == CollapsedDiffReveal::default() {
            self.collapsed_diff_reveals.remove(&hunk.src_ix);
        } else {
            self.collapsed_diff_reveals.insert(hunk.src_ix, reveal);
        }
    }

    pub(super) fn collapsed_diff_expansion_kind(
        &self,
        hunk_ix: usize,
    ) -> crate::view::panes::main::CollapsedDiffExpansionKind {
        use crate::view::panes::main::CollapsedDiffExpansionKind;

        let Some(hunk) = self.collapsed_diff_hunks.get(hunk_ix).copied() else {
            return CollapsedDiffExpansionKind::None;
        };

        let hidden_up = self.collapsed_diff_hidden_up_rows(hunk.src_ix);
        if hunk_ix == 0 {
            if hidden_up > 0 {
                CollapsedDiffExpansionKind::Up
            } else {
                CollapsedDiffExpansionKind::None
            }
        } else if hidden_up == 0 {
            CollapsedDiffExpansionKind::None
        } else if hidden_up <= COLLAPSED_DIFF_REVEAL_STEP {
            CollapsedDiffExpansionKind::Short
        } else {
            CollapsedDiffExpansionKind::Both
        }
    }

    pub(super) fn collapsed_diff_hidden_rows_for_expansion_kind(
        &self,
        src_ix: usize,
        expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind,
    ) -> usize {
        match expansion_kind {
            crate::view::panes::main::CollapsedDiffExpansionKind::Down => {
                self.collapsed_diff_hidden_down_rows(src_ix)
            }
            crate::view::panes::main::CollapsedDiffExpansionKind::Up
            | crate::view::panes::main::CollapsedDiffExpansionKind::Both
            | crate::view::panes::main::CollapsedDiffExpansionKind::Short => {
                self.collapsed_diff_hidden_up_rows(src_ix)
            }
            crate::view::panes::main::CollapsedDiffExpansionKind::None => 0,
        }
    }

    pub(super) fn merge_collapsed_diff_hunks_up(&mut self, hunk_ix: usize) {
        if hunk_ix == 0 || hunk_ix >= self.collapsed_diff_hunks.len() {
            return;
        }

        let previous = self.collapsed_diff_hunks[hunk_ix - 1];
        let current = self.collapsed_diff_hunks[hunk_ix];
        self.collapsed_diff_hunks[hunk_ix - 1] = CollapsedDiffHunk {
            src_ix: previous.src_ix,
            base_row_start: previous.base_row_start,
            base_row_end_exclusive: current.base_row_end_exclusive,
            has_additions: previous.has_additions || current.has_additions,
            has_removals: previous.has_removals || current.has_removals,
            reveal_up_lines: previous.reveal_up_lines,
            reveal_down_lines: current.reveal_down_lines,
        };
        self.collapsed_diff_hunks.remove(hunk_ix);
        self.reindex_collapsed_diff_hunks();
        self.collapsed_diff_header_display_cache.clear();
    }

    pub(super) fn merge_collapsed_diff_hunks_down(&mut self, hunk_ix: usize) {
        if hunk_ix + 1 >= self.collapsed_diff_hunks.len() {
            return;
        }

        let current = self.collapsed_diff_hunks[hunk_ix];
        let next = self.collapsed_diff_hunks[hunk_ix + 1];
        self.collapsed_diff_hunks[hunk_ix] = CollapsedDiffHunk {
            src_ix: current.src_ix,
            base_row_start: current.base_row_start,
            base_row_end_exclusive: next.base_row_end_exclusive,
            has_additions: current.has_additions || next.has_additions,
            has_removals: current.has_removals || next.has_removals,
            reveal_up_lines: current.reveal_up_lines,
            reveal_down_lines: next.reveal_down_lines,
        };
        self.collapsed_diff_hunks.remove(hunk_ix + 1);
        self.reindex_collapsed_diff_hunks();
        self.collapsed_diff_header_display_cache.clear();
    }

    pub(super) fn collapsed_diff_gap_fully_revealed_after_rebuild(&self, hunk_ix: usize) -> bool {
        let Some(current) = self.collapsed_diff_hunks.get(hunk_ix).copied() else {
            return false;
        };
        let Some(next) = self.collapsed_diff_hunks.get(hunk_ix + 1).copied() else {
            return false;
        };

        let gap_len = next
            .base_row_start
            .saturating_sub(current.base_row_end_exclusive);
        if gap_len == 0 {
            return false;
        }

        current
            .reveal_down_lines
            .min(gap_len)
            .saturating_add(next.reveal_up_lines.min(gap_len))
            >= gap_len
    }

    pub(super) fn normalize_collapsed_diff_hunks_after_rebuild(&mut self) {
        let mut hunk_ix = 0;
        while hunk_ix + 1 < self.collapsed_diff_hunks.len() {
            if self.collapsed_diff_gap_fully_revealed_after_rebuild(hunk_ix) {
                self.merge_collapsed_diff_hunks_down(hunk_ix);
            } else {
                hunk_ix += 1;
            }
        }
    }

    pub(super) fn rebuild_collapsed_diff_header_display_cache(&mut self) {
        self.collapsed_diff_header_display_cache.clear();
        let src_ixs = self
            .collapsed_diff_hunks
            .iter()
            .map(|hunk| hunk.src_ix)
            .collect::<Vec<_>>();
        for src_ix in src_ixs {
            if let Some(display) = self.collapsed_diff_dynamic_hunk_range_display(src_ix) {
                self.collapsed_diff_header_display_cache
                    .insert(src_ix, display);
            }
        }
    }

    pub(super) fn rebuild_collapsed_diff_projection(&mut self) {
        self.collapsed_diff_visible_rows.clear();
        self.collapsed_diff_hunk_visible_indices.clear();
        self.collapsed_diff_header_display_cache.clear();

        if !self.is_collapsed_diff_projection_active() {
            return;
        }

        let next_identity = self.current_collapsed_diff_projection_identity();
        if self.collapsed_diff_projection_identity != next_identity {
            self.collapsed_diff_hunks.clear();
            self.collapsed_diff_hunk_ix_by_src_ix.clear();
            self.collapsed_diff_reveals.clear();
        }
        self.collapsed_diff_projection_identity = next_identity;
        if self.collapsed_diff_projection_identity.is_none() {
            return;
        }

        let (_, _, total_rows) = self.current_file_diff_line_to_row_maps();
        if total_rows == 0 {
            return;
        }

        self.ensure_collapsed_diff_hunks_initialized();
        self.normalize_collapsed_diff_hunks_after_rebuild();
        self.reindex_collapsed_diff_hunks();

        if self.collapsed_diff_hunks.is_empty() {
            return;
        }

        for hunk_ix in 0..self.collapsed_diff_hunks.len() {
            let hunk = self.collapsed_diff_hunks[hunk_ix];
            let expansion_kind = self.collapsed_diff_expansion_kind(hunk_ix);
            let has_expansion_header =
                expansion_kind != crate::view::panes::main::CollapsedDiffExpansionKind::None;

            let up_revealed_rows = if hunk_ix == 0 {
                let leading_start = hunk
                    .base_row_start
                    .saturating_sub(hunk.reveal_up_lines.min(hunk.base_row_start));
                leading_start..hunk.base_row_start
            } else {
                let previous = self.collapsed_diff_hunks[hunk_ix - 1];
                let gap_start = previous.base_row_end_exclusive;
                let gap_end = hunk.base_row_start.max(gap_start);
                let gap_len = gap_end.saturating_sub(gap_start);
                let top_end = gap_start.saturating_add(previous.reveal_down_lines.min(gap_len));
                let bottom_start = gap_end.saturating_sub(hunk.reveal_up_lines.min(gap_len));

                for row_ix in gap_start..top_end {
                    self.collapsed_diff_visible_rows
                        .push(CollapsedDiffVisibleRow::FileRow { row_ix });
                }
                bottom_start.max(top_end)..gap_end
            };

            if !has_expansion_header {
                for row_ix in up_revealed_rows.clone() {
                    self.collapsed_diff_visible_rows
                        .push(CollapsedDiffVisibleRow::FileRow { row_ix });
                }
            }

            self.collapsed_diff_hunk_visible_indices
                .push(self.collapsed_diff_visible_rows.len());
            if has_expansion_header {
                let hidden_rows =
                    self.collapsed_diff_hidden_rows_for_expansion_kind(hunk.src_ix, expansion_kind);
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::HunkHeader {
                        src_ix: hunk.src_ix,
                        expansion_kind,
                        display_src_ix: Some(hunk.src_ix),
                        hidden_rows,
                    });
                for row_ix in up_revealed_rows {
                    self.collapsed_diff_visible_rows
                        .push(CollapsedDiffVisibleRow::FileRow { row_ix });
                }
            }
            for row_ix in hunk.base_row_start..hunk.base_row_end_exclusive {
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::FileRow { row_ix });
            }
        }

        if let Some(last_hunk) = self.collapsed_diff_hunks.last().copied() {
            let trailing_end = last_hunk
                .base_row_end_exclusive
                .saturating_add(
                    last_hunk
                        .reveal_down_lines
                        .min(total_rows.saturating_sub(last_hunk.base_row_end_exclusive)),
                )
                .min(total_rows);
            for row_ix in last_hunk.base_row_end_exclusive..trailing_end {
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::FileRow { row_ix });
            }

            let hidden_rows = self.collapsed_diff_hidden_down_rows(last_hunk.src_ix);
            if hidden_rows > 0 {
                self.collapsed_diff_visible_rows
                    .push(CollapsedDiffVisibleRow::HunkHeader {
                        src_ix: last_hunk.src_ix,
                        expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                        display_src_ix: None,
                        hidden_rows,
                    });
            }
        }
        self.rebuild_collapsed_diff_header_display_cache();
    }

    pub(super) fn collapsed_diff_hunk_index_for_src_ix(&self, src_ix: usize) -> Option<usize> {
        self.collapsed_diff_hunk_ix_by_src_ix.get(&src_ix).copied()
    }

    pub(in crate::view) fn collapsed_diff_hunk_for_src_ix(
        &self,
        src_ix: usize,
    ) -> Option<CollapsedDiffHunk> {
        self.collapsed_diff_hunk_index_for_src_ix(src_ix)
            .and_then(|hunk_ix| self.collapsed_diff_hunks.get(hunk_ix).copied())
    }

    pub(in crate::view) fn collapsed_diff_hidden_up_rows(&self, src_ix: usize) -> usize {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return 0;
        };
        let hunk = self.collapsed_diff_hunks[hunk_ix];
        if hunk_ix == 0 {
            return hunk
                .base_row_start
                .saturating_sub(hunk.reveal_up_lines.min(hunk.base_row_start));
        }

        let prev = self.collapsed_diff_hunks[hunk_ix - 1];
        let gap_len = hunk
            .base_row_start
            .saturating_sub(prev.base_row_end_exclusive);
        let visible = prev
            .reveal_down_lines
            .min(gap_len)
            .saturating_add(hunk.reveal_up_lines.min(gap_len));
        gap_len.saturating_sub(visible.min(gap_len))
    }

    pub(in crate::view) fn collapsed_diff_hidden_down_rows(&self, src_ix: usize) -> usize {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return 0;
        };
        let hunk = self.collapsed_diff_hunks[hunk_ix];
        let (_, _, total_rows) = self.current_file_diff_line_to_row_maps();
        if hunk_ix + 1 >= self.collapsed_diff_hunks.len() {
            return total_rows
                .saturating_sub(hunk.base_row_end_exclusive)
                .saturating_sub(
                    hunk.reveal_down_lines
                        .min(total_rows.saturating_sub(hunk.base_row_end_exclusive)),
                );
        }

        let next = self.collapsed_diff_hunks[hunk_ix + 1];
        let gap_len = next
            .base_row_start
            .saturating_sub(hunk.base_row_end_exclusive);
        let visible = hunk
            .reveal_down_lines
            .min(gap_len)
            .saturating_add(next.reveal_up_lines.min(gap_len));
        gap_len.saturating_sub(visible.min(gap_len))
    }

    pub(super) fn collapsed_diff_file_row_line_numbers(
        &self,
        row_ix: usize,
    ) -> Option<(Option<u32>, Option<u32>)> {
        match self.diff_view {
            DiffViewMode::Inline => self
                .file_diff_inline_render_data(row_ix)
                .map(|row| (row.old_line, row.new_line)),
            DiffViewMode::Split => self
                .file_diff_split_row(row_ix)
                .map(|row| (row.old_line, row.new_line)),
        }
    }

    pub(super) fn collapsed_diff_dynamic_hunk_range_display(
        &self,
        src_ix: usize,
    ) -> Option<SharedString> {
        fn update_bounds(min: &mut Option<u32>, max: &mut Option<u32>, line: Option<u32>) {
            let Some(line) = line else {
                return;
            };
            *min = Some(min.map_or(line, |current| current.min(line)));
            *max = Some(max.map_or(line, |current| current.max(line)));
        }

        fn format_range(
            prefix: char,
            fallback_start: u32,
            min: Option<u32>,
            max: Option<u32>,
        ) -> String {
            let (start, count) = match (min, max) {
                (Some(min), Some(max)) if max >= min => (min, max.saturating_sub(min) + 1),
                _ => (fallback_start, 0),
            };
            if count == 1 {
                format!("{prefix}{start}")
            } else {
                format!("{prefix}{start},{count}")
            }
        }

        let (_, _, total_rows) = self.current_file_diff_line_to_row_maps();
        let hunk_ix = self.collapsed_diff_hunk_index_for_src_ix(src_ix)?;
        let hunk = self.collapsed_diff_hunks[hunk_ix];
        let has_revealed_above = if hunk_ix == 0 {
            hunk.reveal_up_lines.min(hunk.base_row_start) > 0
        } else {
            let previous = self.collapsed_diff_hunks[hunk_ix - 1];
            let gap_len = hunk
                .base_row_start
                .saturating_sub(previous.base_row_end_exclusive);
            previous.reveal_down_lines.min(gap_len) > 0 || hunk.reveal_up_lines.min(gap_len) > 0
        };
        let has_revealed_below = if hunk_ix + 1 < self.collapsed_diff_hunks.len() {
            let next = self.collapsed_diff_hunks[hunk_ix + 1];
            let gap_len = next
                .base_row_start
                .saturating_sub(hunk.base_row_end_exclusive);
            hunk.reveal_down_lines.min(gap_len) > 0
        } else {
            hunk.reveal_down_lines
                .min(total_rows.saturating_sub(hunk.base_row_end_exclusive))
                > 0
        };
        if !has_revealed_above && !has_revealed_below {
            return None;
        }

        let parsed = self.patch_diff_row(src_ix).and_then(|line| {
            crate::view::diff_utils::parse_unified_hunk_header_for_display(line.text.as_ref())
        })?;
        let mut old_min = None;
        let mut old_max = None;
        let mut new_min = None;
        let mut new_max = None;
        let mut has_revealed_context = false;

        let mut visit_rows = |range: std::ops::Range<usize>,
                              revealed_context: bool,
                              this: &Self| {
            if range.is_empty() {
                return;
            }
            has_revealed_context |= revealed_context;
            for row_ix in range {
                let Some((old_line, new_line)) = this.collapsed_diff_file_row_line_numbers(row_ix)
                else {
                    continue;
                };
                update_bounds(&mut old_min, &mut old_max, old_line);
                update_bounds(&mut new_min, &mut new_max, new_line);
            }
        };

        if hunk_ix == 0 {
            let leading_start = hunk
                .base_row_start
                .saturating_sub(hunk.reveal_up_lines.min(hunk.base_row_start));
            visit_rows(leading_start..hunk.base_row_start, true, self);
        } else {
            let previous = self.collapsed_diff_hunks[hunk_ix - 1];
            let gap_start = previous.base_row_end_exclusive;
            let gap_end = hunk.base_row_start.max(gap_start);
            let gap_len = gap_end.saturating_sub(gap_start);
            let top_end = gap_start.saturating_add(previous.reveal_down_lines.min(gap_len));
            let bottom_start = gap_end.saturating_sub(hunk.reveal_up_lines.min(gap_len));

            visit_rows(gap_start..top_end, true, self);
            visit_rows(bottom_start.max(top_end)..gap_end, true, self);
        }

        visit_rows(
            hunk.base_row_start..hunk.base_row_end_exclusive,
            false,
            self,
        );

        let trailing_end = if hunk_ix + 1 < self.collapsed_diff_hunks.len() {
            let next = self.collapsed_diff_hunks[hunk_ix + 1];
            let gap_len = next
                .base_row_start
                .saturating_sub(hunk.base_row_end_exclusive);
            hunk.base_row_end_exclusive
                .saturating_add(hunk.reveal_down_lines.min(gap_len))
        } else {
            hunk.base_row_end_exclusive
                .saturating_add(
                    hunk.reveal_down_lines
                        .min(total_rows.saturating_sub(hunk.base_row_end_exclusive)),
                )
                .min(total_rows)
        };
        visit_rows(hunk.base_row_end_exclusive..trailing_end, true, self);

        has_revealed_context.then(|| {
            format!(
                "{} {}",
                format_range('-', parsed.old_start_line, old_min, old_max),
                format_range('+', parsed.new_start_line, new_min, new_max)
            )
            .into()
        })
    }

    pub(in crate::view) fn collapsed_diff_hunk_header_display(
        &self,
        src_ix: usize,
    ) -> Option<SharedString> {
        self.collapsed_diff_header_display_cache
            .get(&src_ix)
            .cloned()
            .or_else(|| self.diff_header_display_cache.get(&src_ix).cloned())
            .or_else(|| {
                self.patch_diff_row(src_ix)
                    .map(|line| SharedString::from(line.text.as_ref().to_owned()))
            })
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_up(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        let delta = self
            .collapsed_diff_hidden_up_rows(src_ix)
            .min(COLLAPSED_DIFF_REVEAL_STEP);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[hunk_ix].reveal_up_lines = self.collapsed_diff_hunks[hunk_ix]
            .reveal_up_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(hunk_ix);
        if self.collapsed_diff_hidden_up_rows(src_ix) == 0 && hunk_ix > 0 {
            self.merge_collapsed_diff_hunks_up(hunk_ix);
        }
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_down(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        let delta = self
            .collapsed_diff_hidden_down_rows(src_ix)
            .min(COLLAPSED_DIFF_REVEAL_STEP);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[hunk_ix].reveal_down_lines = self.collapsed_diff_hunks[hunk_ix]
            .reveal_down_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(hunk_ix);
        if hunk_ix + 1 < self.collapsed_diff_hunks.len()
            && self.collapsed_diff_hidden_down_rows(src_ix) == 0
        {
            self.merge_collapsed_diff_hunks_down(hunk_ix);
        }
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_down_before(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        if hunk_ix == 0 {
            return;
        }
        let previous_hunk_ix = hunk_ix - 1;
        let previous_src_ix = self.collapsed_diff_hunks[previous_hunk_ix].src_ix;
        let delta = self
            .collapsed_diff_hidden_down_rows(previous_src_ix)
            .min(COLLAPSED_DIFF_REVEAL_STEP);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[previous_hunk_ix].reveal_down_lines = self.collapsed_diff_hunks
            [previous_hunk_ix]
            .reveal_down_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(previous_hunk_ix);
        if previous_hunk_ix + 1 < self.collapsed_diff_hunks.len()
            && self.collapsed_diff_hidden_down_rows(previous_src_ix) == 0
        {
            self.merge_collapsed_diff_hunks_down(previous_hunk_ix);
        }
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }

    pub(in crate::view) fn collapsed_diff_reveal_hunk_short(
        &mut self,
        src_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(hunk_ix) = self.collapsed_diff_hunk_index_for_src_ix(src_ix) else {
            return;
        };
        if hunk_ix == 0 {
            return;
        }
        let delta = self.collapsed_diff_hidden_up_rows(src_ix);
        if delta == 0 {
            return;
        }
        self.collapsed_diff_hunks[hunk_ix].reveal_up_lines = self.collapsed_diff_hunks[hunk_ix]
            .reveal_up_lines
            .saturating_add(delta);
        self.persist_collapsed_diff_hunk_reveal(hunk_ix);
        self.merge_collapsed_diff_hunks_up(hunk_ix);
        self.invalidate_collapsed_diff_visible_projection();
        self.ensure_diff_visible_indices();
        cx.notify();
    }
}
