//! Resolved output editing and its collapsed-context fold projection.

use super::*;

impl MainPaneView {
    /// Immediately append a single line from the two-way split view to resolved output.
    pub(in crate::view) fn conflict_resolver_append_split_line_to_output(
        &mut self,
        row_ix: usize,
        side: ConflictPickSide,
        cx: &mut gpui::Context<Self>,
    ) {
        self.ensure_conflict_resolved_output_materialized(cx);
        let Some(row) = self.conflict_resolver.two_way_split_row_by_source(row_ix) else {
            return;
        };
        let text = match side {
            ConflictPickSide::Ours => row.old.as_deref(),
            ConflictPickSide::Theirs => row.new.as_deref(),
        };
        let Some(line) = text else {
            return;
        };
        let line_ix = match side {
            ConflictPickSide::Ours => row.old_line,
            ConflictPickSide::Theirs => row.new_line,
        }
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| n.checked_sub(1));
        let choice = match side {
            ConflictPickSide::Ours => conflict_resolver::ConflictChoice::Ours,
            ConflictPickSide::Theirs => conflict_resolver::ConflictChoice::Theirs,
        };
        if let Some(line_ix) = line_ix {
            self.conflict_resolver_output_replace_line(line_ix, choice, cx);
            return;
        }
        let line_to_append = line.to_string();
        let theme = self.theme;
        let mut append_line_ix = 0usize;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            let content = input.text();
            append_line_ix = source_line_count(content);
            let insertion = append_line_insertion_text(content, line_to_append.as_str());
            let end = content.len();
            input.replace_utf8_range(end..end, &insertion, cx);
        });
        let next_line_count = self
            .conflict_resolver_input
            .read_with(cx, |input, _| split_line_count(input.text()));
        self.conflict_resolver_scroll_resolved_output_to_line(append_line_ix, next_line_count);
    }

    /// Immediately append a single line from the three-way view to resolved output.
    pub(in crate::view) fn conflict_resolver_append_three_way_line_to_output(
        &mut self,
        line_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        // `line_ix` arrives from input-row menus in aligned-row space (section 30).
        let side = match choice {
            conflict_resolver::ConflictChoice::Base => ThreeWayColumn::Base,
            conflict_resolver::ConflictChoice::Ours => ThreeWayColumn::Ours,
            conflict_resolver::ConflictChoice::Theirs => ThreeWayColumn::Theirs,
            conflict_resolver::ConflictChoice::Both => {
                // Both is chunk-level only, not line-level.
                return;
            }
            _ => return,
        };
        let Some(source_line_ix) = self
            .conflict_resolver
            .three_way_side_line_for_row(side, line_ix)
        else {
            return;
        };
        let Some(replacement) = self
            .conflict_resolver
            .three_way_line_text(side, source_line_ix)
            .map(ToString::to_string)
        else {
            return;
        };
        self.conflict_resolver_output_replace_line_with_text(source_line_ix, &replacement, cx);
    }

    fn schedule_conflict_resolved_output_snapshot_refresh(
        &mut self,
        snapshot: &TextModelSnapshot,
        recent_edit_delta: Option<(std::ops::Range<usize>, std::ops::Range<usize>)>,
        cx: &mut gpui::Context<Self>,
    ) {
        let outline_delta = resolved_outline_delta_for_snapshot_transition(
            &self.conflict_resolved_preview_text,
            snapshot,
            recent_edit_delta,
        );
        let path = self.conflict_resolver.path.clone();
        let source_revision = ResolvedOutputSourceRevision::from_snapshot(snapshot);
        self.conflict_resolved_preview_path = path.clone();
        self.conflict_resolved_preview_source_revision = Some(source_revision);
        self.schedule_conflict_resolved_outline_recompute(path, source_revision, outline_delta, cx);
    }

    pub(in crate::view) fn conflict_resolver_set_output(
        &mut self,
        text: String,
        cx: &mut gpui::Context<Self>,
    ) {
        self.ensure_conflict_resolved_output_materialized(cx);
        let unchanged = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text() == text);
        let theme = self.theme;
        let next_text = text;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            let current = input.text();
            // The same minimal-edit the buffer computes for its own `set_text`,
            // so its edit accounting and ours cannot disagree about the span.
            let Some((old_range, new_range)) =
                crate::kit::utf8_edit_delta_between_texts(current, &next_text)
            else {
                return;
            };
            let replacement = next_text.get(new_range).unwrap_or("");
            // Regenerating the output from the session is not an edit the user
            // typed here, so it must not steal their scroll position. Every
            // caller below decides for itself whether to reveal the changed
            // block; an implicit autoscroll would run later (during paint) and
            // override that decision — sending the view to the end of the
            // replaced span, which for a whole-document rewrite is the bottom
            // of the file.
            input.replace_utf8_range_preserving_view(old_range, replacement, cx);
        });
        let (snapshot, edit_deltas) = self.conflict_resolver_input.update(cx, |input, _| {
            (input.text_snapshot(), input.drain_recent_utf8_edit_deltas())
        });
        let recent_edit_delta = (edit_deltas.len() == 1)
            .then(|| edit_deltas.first().cloned())
            .flatten();
        self.apply_conflict_resolved_output_edit_deltas(edit_deltas, &snapshot.rope());
        if unchanged {
            // Choosing a chunk can flip resolved/unresolved state without changing output text.
            // Force marker/provenance refresh so conflict overlays disappear immediately.
            let path = self.conflict_resolver.path.clone();
            self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
            cx.notify();
        } else {
            self.schedule_conflict_resolved_output_snapshot_refresh(
                &snapshot,
                recent_edit_delta,
                cx,
            );
        }
    }

    /// Replace only the output owned by the selected conflict blocks.
    ///
    /// Context and other manually edited blocks remain byte-for-byte intact.
    pub(super) fn conflict_resolver_replace_mapped_blocks(
        &mut self,
        block_indices: &[usize],
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.conflict_resolver.output_is_protected
            || self.conflict_resolved_output_is_streamed()
            || block_indices.is_empty()
        {
            return false;
        }
        let current_output = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot().rope());
        if !self
            .conflict_resolved_output_block_map
            .is_valid_for(&self.conflict_resolver.marker_segments, &current_output)
        {
            self.conflict_resolved_output_block_map =
                conflict_resolver::ResolvedOutputBlockMap::default();
            return false;
        }

        let blocks: Vec<_> = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
        let mut replacements = Vec::with_capacity(block_indices.len());
        for &block_index in block_indices {
            let (Some(block), Some(range)) = (
                blocks.get(block_index),
                self.conflict_resolved_output_block_map
                    .ranges()
                    .get(block_index),
            ) else {
                return false;
            };
            let replacement = conflict_resolver::generate_resolved_text(&[
                conflict_resolver::ConflictSegment::Block((*block).clone()),
            ]);
            replacements.push((range.clone(), replacement));
        }
        replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.0.start));

        let theme = self.theme;
        let (snapshot, edit_deltas) = self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            for (range, replacement) in replacements {
                input.replace_utf8_range(range, &replacement, cx);
            }
            (input.text_snapshot(), input.drain_recent_utf8_edit_deltas())
        });
        let recent_edit_delta = (edit_deltas.len() == 1)
            .then(|| edit_deltas.first().cloned())
            .flatten();
        self.apply_conflict_resolved_output_edit_deltas(edit_deltas, &snapshot.rope());
        let map_is_valid = self
            .conflict_resolved_output_block_map
            .is_valid_for(&self.conflict_resolver.marker_segments, &snapshot.rope());
        if map_is_valid {
            self.schedule_conflict_resolved_output_snapshot_refresh(
                &snapshot,
                recent_edit_delta,
                cx,
            );
        }
        map_is_valid
    }

    pub(super) fn conflict_resolver_mapped_block_output_line(
        &self,
        block_index: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        self.conflict_resolver_input.read_with(cx, |input, _| {
            let output = input.text();
            self.conflict_resolved_output_block_map
                .is_valid_for(&self.conflict_resolver.marker_segments, output)
                .then_some(())?;
            let start = self
                .conflict_resolved_output_block_map
                .ranges()
                .get(block_index)?
                .start;
            output.get(..start).map(|prefix| {
                prefix
                    .as_bytes()
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count()
            })
        })
    }

    /// Refresh the resolved output after a marker segment change, optionally scrolling to
    /// a specific conflict block. Handles both streamed (projection-based) and eager
    /// (full-text regeneration) modes.
    pub(super) fn conflict_resolver_refresh_output_and_scroll(
        &mut self,
        scroll_to_conflict: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.output_is_protected {
            return;
        }
        let next_projection = conflict_resolver::ResolvedOutputProjection::from_segments(
            &self.conflict_resolver.marker_segments,
        );
        // Streamed only while the buffer has not been materialized yet; size does
        // not demote an already-editable output back to a read-only projection.
        if self.conflict_resolved_output_is_streamed() {
            let output_path = self.conflict_resolver.path.clone();
            self.refresh_streamed_resolved_output_preview_from_projection(
                next_projection,
                output_path.as_ref(),
            );
            if let Some(conflict_ix) = scroll_to_conflict
                && let Some(target_line_ix) = self
                    .conflict_resolved_output_projection
                    .as_ref()
                    .and_then(|projection| projection.conflict_line_range(conflict_ix))
                    .map(|range| range.start)
            {
                self.conflict_resolver_scroll_resolved_output_to_line(
                    target_line_ix,
                    self.conflict_resolved_preview_line_count,
                );
            }
        } else {
            if let Some(conflict_ix) = scroll_to_conflict
                && self.conflict_resolver_replace_mapped_blocks(&[conflict_ix], cx)
            {
                if let Some(target_line_ix) =
                    self.conflict_resolver_mapped_block_output_line(conflict_ix, cx)
                {
                    let line_count = self
                        .conflict_resolver_input
                        .read_with(cx, |input, _| split_line_count(input.text()));
                    self.conflict_resolver_scroll_resolved_output_to_line(
                        target_line_ix,
                        line_count,
                    );
                }
                return;
            }
            let resolved =
                conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
            if let Some(conflict_ix) = scroll_to_conflict {
                let target_output_line = output_line_range_for_conflict_block_in_text(
                    &self.conflict_resolver.marker_segments,
                    &resolved,
                    conflict_ix,
                )
                .map(|range| range.start);
                self.conflict_resolver_set_output(resolved.clone(), cx);
                if let Some(target_line_ix) = target_output_line {
                    self.conflict_resolver_scroll_resolved_output_to_line_in_text(
                        target_line_ix,
                        &resolved,
                    );
                }
            } else {
                self.conflict_resolver_set_output(resolved, cx);
            }
            self.rebuild_conflict_resolved_output_block_map(cx);
        }
    }

    /// Delete the current text selection in the resolved output (used by Cut context action).
    pub(in crate::view) fn conflict_resolver_output_delete_selection(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.ensure_conflict_resolved_output_materialized(cx);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            let selection = input.selected_range();
            // The unresolved-conflict rows are uneditable however the edit is
            // spelled, so a Cut across one takes nothing with it.
            if selection.is_empty() || input.edit_alters_protected_range(&selection, "") {
                return;
            }
            input.set_theme(theme, cx);
            let _ = input.replace_selection_utf8("", cx);
        });
    }

    /// Paste text into the resolved output at the current cursor position (used by Paste context action).
    pub(in crate::view) fn conflict_resolver_output_paste_text(
        &mut self,
        paste_text: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        self.ensure_conflict_resolved_output_materialized(cx);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            let pos = input.cursor_offset().min(input.text().len());
            if input.edit_alters_protected_range(&(pos..pos), paste_text) {
                return;
            }
            input.set_theme(theme, cx);
            input.replace_utf8_range(pos..pos, paste_text, cx);
        });
    }

    /// Replace a line in the resolved output with the source line at the same index from A/B/C.
    pub(in crate::view) fn conflict_resolver_output_replace_line(
        &mut self,
        line_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        self.ensure_conflict_resolved_output_materialized(cx);
        let replacement = self
            .conflict_resolver
            .source_line_text_for_choice(choice, line_ix)
            .map(ToString::to_string);
        let Some(replacement) = replacement else {
            return;
        };
        self.conflict_resolver_output_replace_line_with_text(line_ix, &replacement, cx);
    }

    fn conflict_resolver_output_replace_line_with_text(
        &mut self,
        output_line_ix: usize,
        replacement: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let theme = self.theme;
        let mut scroll_to_line = None;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            let content = input.text();
            if let Some(range) = line_content_byte_range_for_index(content, output_line_ix) {
                input.replace_utf8_range(range, replacement, cx);
                scroll_to_line = Some(output_line_ix);
                return;
            }

            let append_line_ix = source_line_count(content);
            let insertion = append_line_insertion_text(content, replacement);
            let end = content.len();
            input.replace_utf8_range(end..end, &insertion, cx);
            scroll_to_line = Some(append_line_ix);
        });

        if let Some(target_line_ix) = scroll_to_line {
            let line_count = self
                .conflict_resolver_input
                .read_with(cx, |input, _| split_line_count(input.text()));
            self.conflict_resolver_scroll_resolved_output_to_line(target_line_ix, line_count);
        }
    }

    /// Rebuild the resolved-output fold projection for collapsed context mode
    /// (section 30). Output line space; derived from the outline's conflict markers.
    /// Streamed outputs stay unfolded (their row space is already projected).
    pub(in crate::view) fn ensure_resolved_output_visible_projection(&mut self) {
        if !self.conflict_resolver.resolved_output_visible_dirty {
            return;
        }
        let fold = self.conflict_resolver.collapse_context
            && !self.conflict_resolved_output_is_streamed()
            && self.conflict_resolved_preview_line_count > 0;
        if !fold {
            self.conflict_resolver.resolved_output_visible = None;
            self.conflict_resolver.resolved_output_visible_dirty = false;
            return;
        }

        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        for marker in self
            .conflict_resolver
            .resolved_outline
            .markers
            .iter()
            .flatten()
        {
            if marker.is_start {
                ranges.push(marker.range_start..marker.range_end);
            }
        }
        let resolved_flags = vec![false; ranges.len()];
        let projection = conflict_resolver::build_three_way_visible_projection_with_options(
            self.conflict_resolved_preview_line_count,
            &ranges,
            &resolved_flags,
            conflict_resolver::ThreeWayVisibleOptions {
                hide_resolved: false,
                collapse_context: true,
                context_fold_reveals: Some(&self.conflict_resolver.output_context_fold_reveals),
            },
        );
        self.conflict_resolver.resolved_output_visible = Some(projection);
        self.conflict_resolver.resolved_output_visible_dirty = false;
    }

    /// Row count of the resolved output lists (fold projection applied).
    pub(in crate::view) fn resolved_output_visible_len(&self) -> usize {
        self.conflict_resolver
            .resolved_output_visible
            .as_ref()
            .map(|projection| projection.len())
            .unwrap_or(self.conflict_resolved_preview_line_count)
    }

    /// Map a resolved-output visible row to its item (line or fold row).
    pub(in crate::view) fn resolved_output_item_for_visible(
        &self,
        visible_ix: usize,
    ) -> Option<conflict_resolver::ThreeWayVisibleItem> {
        match self.conflict_resolver.resolved_output_visible.as_ref() {
            Some(projection) => projection.get(visible_ix),
            None => (visible_ix < self.conflict_resolved_preview_line_count)
                .then_some(conflict_resolver::ThreeWayVisibleItem::Line(visible_ix)),
        }
    }

    /// Map an output line to the visible row showing it (the fold row when
    /// the line is folded away).
    pub(in crate::view) fn resolved_output_visible_ix_for_line(&self, line: usize) -> usize {
        self.conflict_resolver
            .resolved_output_visible
            .as_ref()
            .and_then(|projection| projection.visible_index_for_source_line(line))
            .unwrap_or(line)
    }

    /// Fully expand one collapsed context fold in the resolved output.
    pub(in crate::view) fn conflict_resolver_expand_output_context_fold(
        &mut self,
        fold_id: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver
            .output_context_fold_reveals
            .entry(fold_id)
            .or_default()
            .expand_all = true;
        self.conflict_resolver.resolved_output_visible_dirty = true;
        cx.notify();
    }

    /// Reveal a step of lines from one edge of a resolved-output fold.
    pub(in crate::view) fn conflict_resolver_reveal_output_context_fold(
        &mut self,
        fold_id: usize,
        from_top: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let reveal = self
            .conflict_resolver
            .output_context_fold_reveals
            .entry(fold_id)
            .or_default();
        if from_top {
            reveal.top = reveal
                .top
                .saturating_add(conflict_resolver::CONFLICT_FOLD_REVEAL_STEP);
        } else {
            reveal.bottom = reveal
                .bottom
                .saturating_add(conflict_resolver::CONFLICT_FOLD_REVEAL_STEP);
        }
        self.conflict_resolver.resolved_output_visible_dirty = true;
        cx.notify();
    }
}
