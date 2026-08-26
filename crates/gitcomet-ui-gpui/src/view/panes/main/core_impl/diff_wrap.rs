use super::*;

pub(super) fn line_ranges_intersect(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

pub(super) fn diff_wrap_columns_for_width(width: Pixels, char_width: Pixels) -> usize {
    let char_width = f32::from(char_width.max(px(1.0)));
    ((f32::from(width.max(px(0.0))) / char_width).floor() as usize).max(1)
}

pub(super) fn diff_wrap_byte_ranges_for_source_text(
    text: &str,
    columns: usize,
) -> Vec<rows::DiffWrapByteRange> {
    let mut ranges = rows::diff_wrap_ranges_for_text(text, columns)
        .into_iter()
        .map(rows::DiffWrapByteRange::from_range)
        .collect::<Vec<_>>();
    if ranges.is_empty() {
        ranges.push(rows::DiffWrapByteRange::default());
    }
    ranges
}

pub(super) fn diff_wrap_byte_ranges_for_revealed_text(
    source_text: &str,
    raw_text: Option<&str>,
    columns: usize,
) -> Vec<rows::DiffWrapByteRange> {
    let marker_text = raw_text
        .filter(|raw| crate::view::diff_utils::diff_text_display_len(raw) == source_text.len())
        .unwrap_or(source_text);
    let offset_map = rows::whitespace_visible_diff_offset_map(marker_text, true);
    let mut ranges = rows::diff_wrap_ranges_for_text(
        rows::whitespace_visible_line_text(marker_text).as_ref(),
        columns,
    )
    .into_iter()
    .map(|display_range| {
        let start = offset_map.source_offset_for_display(display_range.start);
        let end = if display_range.end >= offset_map.display_len() {
            offset_map.source_len()
        } else {
            offset_map.source_offset_for_display(display_range.end)
        };
        rows::DiffWrapByteRange { start, end }
    })
    .collect::<Vec<_>>();
    if ranges.is_empty() {
        ranges.push(rows::DiffWrapByteRange::default());
    }
    ranges
}

pub(super) fn diff_wrap_byte_ranges_for_text(
    source_text: &str,
    raw_text: Option<&str>,
    columns: usize,
    reveal_whitespace_chars: bool,
) -> Vec<rows::DiffWrapByteRange> {
    if reveal_whitespace_chars {
        diff_wrap_byte_ranges_for_revealed_text(source_text, raw_text, columns)
    } else {
        diff_wrap_byte_ranges_for_source_text(source_text, columns)
    }
}

pub(super) fn diff_wrap_empty_byte_ranges() -> Vec<rows::DiffWrapByteRange> {
    vec![rows::DiffWrapByteRange::default()]
}

pub(super) fn diff_wrap_byte_ranges_for_file_diff_text(
    text: &gitcomet_core::file_diff::FileDiffLineText,
    columns: usize,
    reveal_whitespace_chars: bool,
) -> Vec<rows::DiffWrapByteRange> {
    let display = crate::view::file_diff_display::file_diff_display_text(text);
    diff_wrap_byte_ranges_for_text(
        display.as_ref(),
        Some(text.as_ref()),
        columns,
        reveal_whitespace_chars,
    )
}

pub(super) fn diff_wrap_byte_ranges_for_optional_file_diff_text(
    text: Option<&gitcomet_core::file_diff::FileDiffLineText>,
    columns: usize,
    reveal_whitespace_chars: bool,
) -> Vec<rows::DiffWrapByteRange> {
    text.map(|text| {
        diff_wrap_byte_ranges_for_file_diff_text(text, columns, reveal_whitespace_chars)
    })
    .unwrap_or_else(diff_wrap_empty_byte_ranges)
}

pub(super) fn diff_wrap_byte_range_at(
    ranges: &[rows::DiffWrapByteRange],
    wrap_ix: usize,
) -> rows::DiffWrapByteRange {
    ranges.get(wrap_ix).copied().unwrap_or_default()
}

pub(super) fn shift_resolved_output_marker(
    marker: ResolvedOutputConflictMarker,
    line_delta: isize,
) -> ResolvedOutputConflictMarker {
    ResolvedOutputConflictMarker {
        conflict_ix: marker.conflict_ix,
        range_start: shifted_line_index(marker.range_start, line_delta),
        range_end: shifted_line_index(marker.range_end, line_delta),
        is_start: marker.is_start,
        is_end: marker.is_end,
        unresolved: marker.unresolved,
    }
}

impl MainPaneView {
    pub(super) fn diff_source_visible_len(&self) -> usize {
        // A file preview has no diff rows: its source rows are the file's
        // lines, and they wrap through the same projection.
        if self.is_file_preview_active() {
            return self.worktree_preview_line_count().unwrap_or(0);
        }
        if self.is_collapsed_diff_projection_active() {
            return self.collapsed_diff_visible_rows.len();
        }
        self.diff_visible_inline_map
            .as_ref()
            .map(|map| map.visible_len())
            .unwrap_or_else(|| self.diff_visible_indices.len())
    }

    /// True when the *text diff's* wrap projection maps list positions to
    /// source rows.
    ///
    /// The rendered markdown preview keeps its own visual-row mapping and
    /// never refreshes these rows, so a preview opened after a wrapped text
    /// diff would otherwise be remapped through that diff's stale rows.
    pub(super) fn diff_wrap_projection_active(&self) -> bool {
        self.diff_word_wrap
            && self.diff_wrap_visible_cache_key.is_some()
            && !self.is_markdown_preview_active()
    }

    pub(in crate::view) fn diff_visible_len(&self) -> usize {
        if self.diff_wrap_projection_active() {
            return self.diff_wrap_visible_rows.len();
        }
        self.diff_source_visible_len()
    }

    pub(in crate::view) fn diff_source_visible_ix_for_visible_ix(
        &self,
        visible_ix: usize,
    ) -> Option<usize> {
        if self.diff_wrap_projection_active() {
            return self
                .diff_wrap_visible_rows
                .get(visible_ix)
                .map(|row| row.source_visible_ix);
        }
        Some(visible_ix)
    }

    pub(in crate::view) fn diff_visual_ix_for_source_visible_ix(
        &self,
        source_visible_ix: usize,
    ) -> usize {
        if !self.diff_wrap_projection_active() {
            return source_visible_ix;
        }

        let visual_ix = self
            .diff_wrap_visible_rows
            .partition_point(|row| row.source_visible_ix < source_visible_ix);
        if self
            .diff_wrap_visible_rows
            .get(visual_ix)
            .is_some_and(|row| row.source_visible_ix == source_visible_ix)
        {
            visual_ix
        } else {
            source_visible_ix
        }
    }

    pub(in crate::view) fn diff_source_mapped_ix_for_visible_ix(
        &self,
        visible_ix: usize,
    ) -> Option<usize> {
        if self.is_collapsed_diff_projection_active() {
            return self
                .collapsed_visible_row(visible_ix)
                .and_then(CollapsedDiffVisibleRow::row_ix);
        }
        if let Some(map) = self.diff_visible_inline_map.as_ref() {
            return map.src_ix_for_visible_ix(visible_ix);
        }
        self.diff_visible_indices.get(visible_ix).copied()
    }

    pub(in crate::view) fn diff_mapped_ix_for_visible_ix(
        &self,
        visible_ix: usize,
    ) -> Option<usize> {
        if self.diff_word_wrap
            && let Some(row) = self.diff_wrap_visible_rows.get(visible_ix)
        {
            return self.diff_source_mapped_ix_for_visible_ix(row.source_visible_ix);
        }
        self.diff_source_mapped_ix_for_visible_ix(visible_ix)
    }

    pub(in crate::view) fn diff_text_wrap_for_visible_ix(
        &self,
        visible_ix: usize,
    ) -> Option<rows::DiffTextWrapSlice> {
        if !self.diff_wrap_projection_active() {
            return None;
        }
        let row = self.diff_wrap_visible_rows.get(visible_ix)?;
        let is_split_source = row.wrap_ix > 0
            || self
                .diff_wrap_visible_rows
                .get(visible_ix.saturating_add(1))
                .is_some_and(|next| next.source_visible_ix == row.source_visible_ix);
        if !is_split_source {
            return None;
        }
        let key = self.diff_wrap_visible_cache_key?;
        let columns = if self.is_file_preview_active() {
            // The preview is one column whatever the diff view is set to.
            key.preview_columns
        } else {
            match self.diff_view {
                DiffViewMode::Inline => key.inline_columns,
                DiffViewMode::Split => key.split_columns,
            }
        };
        Some(rows::DiffTextWrapSlice {
            wrap_ix: row.wrap_ix,
            wrap_columns: columns,
            primary_range: row.primary_range,
            secondary_range: row.secondary_range,
        })
    }

    pub(in crate::view) fn ensure_diff_wrap_visible_rows(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.diff_word_wrap {
            if self.diff_wrap_visible_cache_key.take().is_some()
                || !self.diff_wrap_visible_rows.is_empty()
            {
                self.diff_wrap_visible_rows.clear();
                self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
                if self.diff_search_has_query() {
                    self.diff_search_recompute_matches_for_current_view_preserving_current();
                }
            }
            return;
        }

        let source_len = self.diff_source_visible_len();
        let (inline_columns, split_columns) = self.diff_wrap_columns(window, cx);
        let preview_columns = self.worktree_preview_wrap_columns(window, cx);
        let key = DiffWrapVisibleCacheKey {
            source_len,
            diff_view: self.diff_view,
            is_file_view: self.is_file_diff_view_active(),
            preview_columns,
            preview_content_rev: if self.is_file_preview_active() {
                self.worktree_preview_content_rev
            } else {
                0
            },
            collapsed_projection_active: self.is_collapsed_diff_projection_active(),
            projection_rev: if self.is_collapsed_diff_projection_active() {
                self.diff_visible_projection_rev
            } else {
                0
            },
            diff_cache_rev: self.diff_cache_rev,
            file_diff_cache_seq: self.file_diff_cache_seq,
            inline_columns,
            split_columns,
            reveal_whitespace_chars: self.reveal_whitespace_chars,
        };
        if self.diff_wrap_visible_cache_key == Some(key) {
            return;
        }

        self.diff_wrap_visible_rows.clear();
        self.diff_wrap_visible_rows.reserve(source_len);
        for source_visible_ix in 0..source_len {
            let (primary_ranges, secondary_ranges) = self.diff_wrap_ranges_for_source_visible_ix(
                source_visible_ix,
                inline_columns,
                split_columns,
                preview_columns,
            );
            let row_count = primary_ranges.len().max(secondary_ranges.len()).max(1);
            for wrap_ix in 0..row_count {
                self.diff_wrap_visible_rows.push(DiffWrapVisualRow {
                    source_visible_ix,
                    wrap_ix,
                    primary_range: diff_wrap_byte_range_at(&primary_ranges, wrap_ix),
                    secondary_range: diff_wrap_byte_range_at(&secondary_ranges, wrap_ix),
                });
            }
        }
        self.diff_wrap_visible_cache_key = Some(key);
        self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_for_current_view_preserving_current();
        }
    }

    /// Font the wrapped diff rows are painted in — the same family the rows
    /// container applies via `.font_family(editor_font_family)`. Wrap widths
    /// must be measured in it, never in the ambient text style.
    pub(in crate::view) fn diff_wrap_measure_font_family(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::SharedString {
        crate::font_preferences::current_editor_font_family(cx).into()
    }

    pub(in crate::view) fn diff_wrap_columns(
        &self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> (usize, usize) {
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let vertical_gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
        let content_width = (self.main_pane_content_width(cx) - vertical_gutter).max(px(0.0));
        // Measured in the editor font the rows are painted in, not in the
        // ambient UI font that is still current while this element tree is
        // being built. See `diff_text_wrap_char_width`.
        let char_width =
            rows::diff_canvas_text_wrap_char_width(window, self.diff_wrap_measure_font_family(cx));
        let pad = rows::diff_canvas_row_horizontal_padding(ui_scale_percent);
        let inline_text_start = if self.diff_show_line_numbers {
            rows::diff_canvas_inline_text_start(ui_scale_percent)
        } else {
            pad
        };
        let single_text_start = if self.diff_show_line_numbers {
            rows::diff_canvas_single_column_text_start(ui_scale_percent)
        } else {
            pad
        };
        // Inline annotate reserves a fixed column at the left, narrowing the
        // available text width for word wrapping.
        let annotation_width = if self.annotation_active() {
            self.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        let inline_columns = diff_wrap_columns_for_width(
            content_width - annotation_width - inline_text_start - pad,
            char_width,
        );

        let (left_w, right_w) =
            crate::view::diff_split_column_widths(content_width, self.diff_split_ratio);
        // The annotation column narrows the left split column; subtract it from
        // the shared wrap width so wrapped text stays within the left column.
        let split_text_width =
            left_w.min(right_w).max(px(0.0)) - annotation_width - single_text_start - pad;
        let split_columns = diff_wrap_columns_for_width(split_text_width, char_width);
        (inline_columns, split_columns)
    }

    /// Columns a wrapped file preview row may use.
    ///
    /// Neither of the diff's two widths describes it: an inline diff row
    /// reserves two gutter cells for the old and new line numbers, and a split
    /// row only gets half the pane. A preview row is one column with one
    /// gutter, so it measures its own.
    pub(in crate::view) fn worktree_preview_wrap_columns(
        &self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> usize {
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let vertical_gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
        let content_width = (self.main_pane_content_width(cx) - vertical_gutter).max(px(0.0));
        let char_width =
            rows::diff_canvas_text_wrap_char_width(window, self.diff_wrap_measure_font_family(cx));
        let pad = rows::diff_canvas_row_horizontal_padding(ui_scale_percent);
        let text_start = if self.diff_show_line_numbers {
            rows::diff_canvas_single_column_text_start(ui_scale_percent)
        } else {
            pad
        };
        let annotation_width = if self.annotation_active() {
            self.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        // The change bar is only drawn for a wholly added or removed file, but
        // it is always subtracted: wrapping a few pixels early is invisible,
        // wrapping late runs the last character under the scrollbar.
        let change_bar = rows::diff_canvas_change_bar_width(ui_scale_percent);
        diff_wrap_columns_for_width(
            content_width - annotation_width - change_bar - text_start - pad,
            char_width,
        )
    }

    /// Widths a wrapped markdown preview row may occupy: the full content
    /// width for the inline and worktree lists, and the narrower of the two
    /// split columns for the side-by-side lists, so both columns wrap
    /// identically and stay row-aligned.
    pub(in crate::view) fn markdown_preview_wrap_widths(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> (Pixels, Pixels) {
        let vertical_gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
        let content_width = (self.main_pane_content_width(cx) - vertical_gutter).max(px(0.0));
        let (left_w, right_w) =
            crate::view::diff_split_column_widths(content_width, self.diff_split_ratio);
        (content_width, left_w.min(right_w).max(px(0.0)))
    }

    pub(super) fn diff_wrap_ranges_for_source_visible_ix(
        &self,
        source_visible_ix: usize,
        inline_columns: usize,
        split_columns: usize,
        preview_columns: usize,
    ) -> (Vec<rows::DiffWrapByteRange>, Vec<rows::DiffWrapByteRange>) {
        // A file preview is one column of plain file lines.
        if self.is_file_preview_active() {
            return (
                diff_wrap_byte_ranges_for_optional_file_diff_text(
                    self.worktree_preview_line_raw_text(source_visible_ix)
                        .as_ref(),
                    preview_columns,
                    self.reveal_whitespace_chars,
                ),
                diff_wrap_empty_byte_ranges(),
            );
        }
        if self.is_collapsed_diff_projection_active() {
            let Some(row) = self.collapsed_visible_row(source_visible_ix) else {
                return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
            };
            return match row {
                CollapsedDiffVisibleRow::HunkHeader { .. } => {
                    (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges())
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } => match self.diff_view {
                    DiffViewMode::Inline => {
                        let Some(row) = self.file_diff_inline_render_data(row_ix) else {
                            return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                        };
                        (
                            diff_wrap_byte_ranges_for_file_diff_text(
                                &row.text,
                                inline_columns,
                                self.reveal_whitespace_chars,
                            ),
                            diff_wrap_empty_byte_ranges(),
                        )
                    }
                    DiffViewMode::Split => {
                        let Some(row) = self.file_diff_split_render_data(row_ix) else {
                            return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                        };
                        (
                            diff_wrap_byte_ranges_for_optional_file_diff_text(
                                row.old.as_ref(),
                                split_columns,
                                self.reveal_whitespace_chars,
                            ),
                            diff_wrap_byte_ranges_for_optional_file_diff_text(
                                row.new.as_ref(),
                                split_columns,
                                self.reveal_whitespace_chars,
                            ),
                        )
                    }
                },
            };
        }

        let Some(mapped_ix) = self.diff_source_mapped_ix_for_visible_ix(source_visible_ix) else {
            return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
        };
        if self.is_file_diff_view_active() {
            return match self.diff_view {
                DiffViewMode::Inline => {
                    if let Some(row) = self.file_diff_inline_render_data(mapped_ix) {
                        return (
                            diff_wrap_byte_ranges_for_file_diff_text(
                                &row.text,
                                inline_columns,
                                self.reveal_whitespace_chars,
                            ),
                            diff_wrap_empty_byte_ranges(),
                        );
                    }
                    let Some(line) = self.file_diff_inline_row(mapped_ix) else {
                        return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                    };
                    let text = self
                        .diff_text_full_line_for_region(source_visible_ix, DiffTextRegion::Inline);
                    (
                        diff_wrap_byte_ranges_for_text(
                            text.as_ref(),
                            Some(crate::view::diff_utils::diff_content_text(&line)),
                            inline_columns,
                            self.reveal_whitespace_chars,
                        ),
                        diff_wrap_empty_byte_ranges(),
                    )
                }
                DiffViewMode::Split => {
                    let Some(row) = self.file_diff_split_render_data(mapped_ix) else {
                        return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                    };
                    (
                        diff_wrap_byte_ranges_for_optional_file_diff_text(
                            row.old.as_ref(),
                            split_columns,
                            self.reveal_whitespace_chars,
                        ),
                        diff_wrap_byte_ranges_for_optional_file_diff_text(
                            row.new.as_ref(),
                            split_columns,
                            self.reveal_whitespace_chars,
                        ),
                    )
                }
            };
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                let click_kind = self
                    .diff_click_kinds
                    .get(mapped_ix)
                    .copied()
                    .unwrap_or(DiffClickKind::Line);
                if click_kind != DiffClickKind::Line {
                    return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                }
                let Some(line) = self.patch_diff_row(mapped_ix) else {
                    return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                };
                let text =
                    self.diff_text_full_line_for_region(source_visible_ix, DiffTextRegion::Inline);
                (
                    diff_wrap_byte_ranges_for_text(
                        text.as_ref(),
                        Some(line.text.as_ref()),
                        inline_columns,
                        self.reveal_whitespace_chars,
                    ),
                    diff_wrap_empty_byte_ranges(),
                )
            }
            DiffViewMode::Split => match self.patch_diff_split_row(mapped_ix) {
                Some(PatchSplitRow::Aligned { row, .. }) => {
                    let left = self.diff_text_full_line_for_region(
                        source_visible_ix,
                        DiffTextRegion::SplitLeft,
                    );
                    let right = self.diff_text_full_line_for_region(
                        source_visible_ix,
                        DiffTextRegion::SplitRight,
                    );
                    (
                        diff_wrap_byte_ranges_for_text(
                            left.as_ref(),
                            row.old.as_ref().map(|text| text.as_ref()),
                            split_columns,
                            self.reveal_whitespace_chars,
                        ),
                        diff_wrap_byte_ranges_for_text(
                            right.as_ref(),
                            row.new.as_ref().map(|text| text.as_ref()),
                            split_columns,
                            self.reveal_whitespace_chars,
                        ),
                    )
                }
                Some(PatchSplitRow::Raw { src_ix, click_kind }) => {
                    if click_kind != DiffClickKind::Line {
                        return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                    }
                    let Some(line) = self.patch_diff_row(src_ix) else {
                        return (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges());
                    };
                    let left = self.diff_text_full_line_for_region(
                        source_visible_ix,
                        DiffTextRegion::SplitLeft,
                    );
                    let right = self.diff_text_full_line_for_region(
                        source_visible_ix,
                        DiffTextRegion::SplitRight,
                    );
                    (
                        diff_wrap_byte_ranges_for_text(
                            left.as_ref(),
                            (!left.is_empty()).then_some(line.text.as_ref()),
                            split_columns,
                            self.reveal_whitespace_chars,
                        ),
                        diff_wrap_byte_ranges_for_text(
                            right.as_ref(),
                            (!right.is_empty()).then_some(line.text.as_ref()),
                            split_columns,
                            self.reveal_whitespace_chars,
                        ),
                    )
                }
                None => (diff_wrap_empty_byte_ranges(), diff_wrap_empty_byte_ranges()),
            },
        }
    }
}
