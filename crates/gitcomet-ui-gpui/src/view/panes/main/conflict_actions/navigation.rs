//! Conflict navigation: nav targets, jumps, and scrolling the columns and
//! resolved output to a target.

use super::*;
use gitcomet_ui_kit::text_layout::TextLayoutExt as _;

impl MainPaneView {
    #[cfg(test)]
    pub(in crate::view::panes::main) fn conflict_marker_nav_entries(&self) -> Vec<usize> {
        conflict_marker_nav_entries_from_markers(&self.conflict_resolver.resolved_outline.markers)
    }

    #[cfg(test)]
    pub(in crate::view::panes::main) fn conflict_fallback_nav_entries(&self) -> Vec<usize> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => (0..self.conflict_resolver_conflict_count())
                .filter_map(|conflict_ix| {
                    self.conflict_resolver
                        .visible_index_for_conflict(conflict_ix)
                })
                .collect(),
            ConflictResolverViewMode::TwoWayDiff => (0..self.conflict_resolver_conflict_count())
                .filter_map(|conflict_ix| {
                    self.conflict_resolver
                        .two_way_visible_ix_for_conflict(conflict_ix)
                })
                .collect(),
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn conflict_nav_entries(&self) -> Vec<usize> {
        let marker_entries = self.conflict_marker_nav_entries();
        if !marker_entries.is_empty() {
            return marker_entries;
        }
        self.conflict_fallback_nav_entries()
    }

    /// Scroll all conflict resolver column lists to the given item.
    pub(in crate::view) fn conflict_resolver_scroll_all_columns(
        &self,
        target: usize,
        strategy: gpui::ScrollStrategy,
    ) {
        self.conflict_resolver_diff_scroll
            .scroll_to_item_strict(target, strategy);
        self.conflict_preview_ours_scroll
            .scroll_to_item_strict(target, strategy);
        self.conflict_preview_theirs_scroll
            .scroll_to_item_strict(target, strategy);
    }

    /// Bring all column lists to the given item only if it is not already
    /// fully visible (non-strict scroll — a no-op for visible rows). Used by
    /// context-menu invocations so the view doesn't jump under the menu.
    pub(in crate::view) fn conflict_resolver_reveal_all_columns(&self, target: usize) {
        self.conflict_resolver_diff_scroll
            .scroll_to_item(target, gpui::ScrollStrategy::Center);
        self.conflict_preview_ours_scroll
            .scroll_to_item(target, gpui::ScrollStrategy::Center);
        self.conflict_preview_theirs_scroll
            .scroll_to_item(target, gpui::ScrollStrategy::Center);
    }

    /// Bring the resolved output (and its gutter) to the given line only if
    /// it is not already fully visible.
    pub(in crate::view) fn conflict_resolver_reveal_resolved_output_line(
        &self,
        target_line_ix: usize,
        line_count: usize,
    ) {
        if line_count == 0 {
            return;
        }
        let target_line = target_line_ix.min(line_count.saturating_sub(1));
        let target = self.resolved_output_visible_ix_for_line(target_line);
        // The output body is now the editable `TextInput`, driven by
        // `conflict_resolved_output_editor_scroll`. Scroll the line-number gutter
        // to the target row; the gutter↔editor scroll sync (which makes the
        // changed handle the master) then pulls the editor to the same offset.
        // The streamed list is scrolled alongside it so both output renderings
        // land in the same place, matching the strict variant's handle set.
        self.conflict_resolved_preview_scroll
            .scroll_to_item(target, gpui::ScrollStrategy::Center);
        self.conflict_resolved_preview_gutter_scroll
            .scroll_to_item(target, gpui::ScrollStrategy::Center);
        self.place_conflict_resolved_output_editor_at_row(target);
    }

    /// Put the editable output on the same row the gutter was just sent to,
    /// now, rather than leaving it to the prepaint mirror.
    ///
    /// The columns and the gutter are `uniform_list`s: they own a deferred
    /// scroll and consume it in their own prepaint, so they move in the frame
    /// navigation triggers. The editable output is a `TextInput` with no such
    /// mechanism — it can only be dragged along by the gutter afterwards, which
    /// costs a frame at best, and at worst does not happen at all: the mirror is
    /// only attached when a render pass observes the gutter's deferred scroll
    /// still pending, and the fallback offset sync needs *another* render, which
    /// nothing schedules. That is the navigation where the columns jump and the
    /// output stays put until an unrelated event (a mouse move, a poll) repaints
    /// the pane seconds later.
    ///
    /// Computed the way `uniform_list` computes it — same centring, same
    /// clamping, same "already visible rows don't move" rule — so the mirror
    /// that runs afterwards agrees and nothing jitters.
    fn place_conflict_resolved_output_editor_at_row(&self, row_ix: usize) {
        if self.conflict_resolved_output_is_streamed() {
            return;
        }
        let gutter = uniform_list_base_handle(&self.conflict_resolved_preview_gutter_scroll);
        let Some(gutter_y) = centered_reveal_scroll_y(
            row_ix,
            self.conflict_resolved_gutter_row_height,
            gutter.bounds().size.height,
            gutter.max_offset().y,
            gutter.offset().y,
        ) else {
            return;
        };
        let editor = &self.conflict_resolved_output_editor_scroll;
        let offset = editor.offset();
        let editor_y = gutter_y.clamp(-editor.max_offset().y.max(px(0.0)), px(0.0));
        if offset.y != editor_y {
            editor.set_offset(point(offset.x, editor_y));
        }
    }

    /// Record where a merge-tool column row painted its text.
    pub(in crate::view) fn set_conflict_text_hitbox(
        &mut self,
        visible_ix: usize,
        column: ThreeWayColumn,
        hitbox: crate::view::mod_helpers::ConflictTextHitbox,
    ) {
        self.conflict_text_hitboxes
            .insert((visible_ix, column), hitbox);
    }

    /// Scroll the merge tool sideways to the current quick-search match.
    ///
    /// Only the column the match is in is moved: the columns share a horizontal
    /// scroll sync, so it carries the others along, and picking one keeps this
    /// from fighting itself when several columns hold the same text.
    /// Reports whether the row had been painted, which is what tells the caller
    /// to stop retrying.
    pub(in crate::view) fn reveal_conflict_search_match_horizontally(
        &mut self,
        visible_ix: usize,
        matcher: &super::super::diff_search::DiffSearchMatcher,
    ) -> bool {
        let mut painted = false;
        for column in [
            ThreeWayColumn::Base,
            ThreeWayColumn::Ours,
            ThreeWayColumn::Theirs,
        ] {
            let Some(hitbox) = self.conflict_text_hitboxes.get(&(visible_ix, column)) else {
                continue;
            };
            painted = true;
            let painted_text = hitbox.layout.text.clone();
            let Some(range) = self.painted_search_range(painted_text.as_ref(), matcher) else {
                continue;
            };
            let Some(hitbox) = self.conflict_text_hitboxes.get(&(visible_ix, column)) else {
                continue;
            };
            let layout = &hitbox.layout;
            let local_left = layout.x_for_index(range.start.min(layout.len()));
            let local_right = layout.x_for_index(range.end.min(layout.len()));
            let row_left = hitbox.bounds.left();

            let Some(handle) = self.conflict_column_scroll_handle(column) else {
                continue;
            };
            let viewport = handle.bounds();
            let offset = handle.offset();
            // Painted bounds are window space with the scroll already applied.
            let to_content = |x: Pixels| row_left + x - viewport.origin.x - offset.x;
            let Some(target_x) = super::super::helpers::reveal_scroll_x(
                to_content(local_left),
                to_content(local_right),
                viewport.size.width,
                handle.max_offset().x,
                offset.x,
            ) else {
                continue;
            };
            handle.set_offset(point(target_x, offset.y));
        }
        painted
    }

    /// The list handle a column's rows are actually tracked by.
    ///
    /// The two-way view reuses the three-way handles for a two-column layout:
    /// its left (Ours) list is tracked by `conflict_resolver_diff_scroll`, and
    /// `conflict_preview_ours_scroll` is never laid out there — writing to it
    /// scrolls nothing. `None` for a column the current mode does not render.
    fn conflict_column_scroll_handle(&self, column: ThreeWayColumn) -> Option<ScrollHandle> {
        let list = match (self.conflict_resolver.view_mode, column) {
            (ConflictResolverViewMode::ThreeWay, ThreeWayColumn::Base) => {
                &self.conflict_resolver_diff_scroll
            }
            (ConflictResolverViewMode::ThreeWay, ThreeWayColumn::Ours) => {
                &self.conflict_preview_ours_scroll
            }
            (ConflictResolverViewMode::TwoWayDiff, ThreeWayColumn::Ours) => {
                &self.conflict_resolver_diff_scroll
            }
            (_, ThreeWayColumn::Theirs) => &self.conflict_preview_theirs_scroll,
            (ConflictResolverViewMode::TwoWayDiff, ThreeWayColumn::Base) => return None,
        };
        Some(uniform_list_base_handle(list))
    }

    /// Bring the resolved output to the line a quick-search hit in the input
    /// columns produced.
    ///
    /// `conflict_resolver_scroll_all_columns` knows only the three column
    /// lists; the output rides handles of its own, so before this a match far
    /// down the file scrolled the inputs and left the output parked at the top.
    /// Reveal, not centre — an output line already on screen must not jump,
    /// matching what `conflict_jump_to_nav_target` does.
    ///
    /// Runs without a `cx` because the whole search-scroll path does, so the
    /// line count comes from the cached `conflict_resolved_preview_line_count`
    /// rather than a fresh editor snapshot; it is refreshed on every output
    /// edit, and it only clamps the target.
    pub(in crate::view) fn conflict_resolver_reveal_search_match_in_output(
        &self,
        visible_ix: usize,
    ) {
        let Some(output_line) = self
            .conflict_resolver
            .output_line_for_visible_row(visible_ix)
        else {
            return;
        };
        self.conflict_resolver_reveal_resolved_output_line(
            output_line,
            self.conflict_resolved_preview_line_count.max(1),
        );
    }

    pub(in crate::view::panes::main) fn conflict_resolver_visible_ix_for_conflict(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => self
                .conflict_resolver
                .visible_index_for_conflict(conflict_ix),
            ConflictResolverViewMode::TwoWayDiff => {
                self.conflict_resolver_two_way_visible_ix_for_conflict(conflict_ix)
            }
        }
    }

    pub(in crate::view::panes::main) fn conflict_resolver_output_line_for_conflict(
        &self,
        conflict_ix: usize,
        output_text: &str,
    ) -> Option<usize> {
        // Prefer the conflict block's start line so keyboard navigation keeps
        // the three-way input panes and resolved output aligned to the same anchor.
        if self.conflict_resolved_output_is_streamed() {
            self.conflict_resolved_output_projection
                .as_ref()
                .and_then(|projection| projection.conflict_line_range(conflict_ix))
                .map(|range| range.start)
        } else {
            output_line_range_for_conflict_block_in_text(
                &self.conflict_resolver.marker_segments,
                output_text,
                conflict_ix,
            )
            .map(|range| range.start)
        }
        .or_else(|| {
            first_output_marker_line_for_conflict(
                &self.conflict_resolver.resolved_outline.markers,
                conflict_ix,
            )
        })
    }

    pub(super) fn conflict_resolver_refresh_nav_targets(&mut self) {
        let block_count =
            conflict_resolver::conflict_count(&self.conflict_resolver.marker_segments);
        let display_aligned_ranges: Vec<Option<std::ops::Range<usize>>> =
            if self.conflict_resolver.three_way_conflict_ranges[ThreeWayColumn::Ours].len()
                == block_count
            {
                self.conflict_resolver.three_way_conflict_ranges[ThreeWayColumn::Ours]
                    .iter()
                    .cloned()
                    .map(Some)
                    .collect()
            } else {
                self.conflict_resolver
                    .conflict_region_indices
                    .iter()
                    .map(|region_index| {
                        self.conflict_resolver
                            .original_region_aligned_ranges
                            .get(*region_index)
                            .cloned()
                            .flatten()
                    })
                    .collect()
            };
        let session = self
            .active_repo()
            .and_then(|repo| repo.conflict_state.conflict_session.as_ref())
            .filter(|session| {
                self.conflict_resolver.path.as_deref() == Some(session.path.as_path())
            });
        let targets = conflict_resolver::build_conflict_nav_targets(
            session,
            &self.conflict_resolver.original_region_aligned_ranges,
            &self.conflict_resolver.conflict_region_indices,
            &display_aligned_ranges,
            &self.conflict_resolver.marker_segments,
        );
        self.conflict_resolver.reconcile_nav_targets(targets);
    }

    pub(super) fn conflict_resolver_visible_ix_for_nav_target(
        &self,
        target: &conflict_resolver::ConflictNavTarget,
    ) -> Option<usize> {
        let displayed = target
            .display_conflict_index
            .and_then(|index| self.conflict_resolver_visible_ix_for_conflict(index));
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => target
                .aligned_rows
                .as_ref()
                .and_then(|range| {
                    self.conflict_resolver
                        .visible_index_for_aligned_row(range.start)
                })
                .or(displayed),
            ConflictResolverViewMode::TwoWayDiff
                if self.conflict_resolver.two_way_uses_aligned_rows() =>
            {
                target
                    .aligned_rows
                    .as_ref()
                    .and_then(|range| {
                        self.conflict_resolver
                            .visible_index_for_aligned_row(range.start)
                    })
                    .or(displayed)
            }
            ConflictResolverViewMode::TwoWayDiff => displayed,
        }
    }

    fn conflict_resolver_output_line_for_nav_target(
        &self,
        target: &conflict_resolver::ConflictNavTarget,
        output_text: &str,
    ) -> Option<usize> {
        target
            .display_conflict_index
            .and_then(|conflict_index| {
                self.conflict_resolver_output_line_for_conflict(conflict_index, output_text)
            })
            .or_else(|| {
                self.conflict_resolver
                    .output_line_for_nav_target_provenance(target)
            })
    }

    pub(in crate::view) fn conflict_jump_to_nav_target(
        &mut self,
        target_index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_resolver.select_nav_target(target_index) {
            return;
        }
        let target = self.conflict_resolver.nav_targets[target_index].clone();
        // Reveal rather than centre, in each pane's own line space, the way
        // KDiff3's `getBestFirstLine` does: a target already on screen does not
        // move the view at all. Centring both panes independently is what made
        // navigation nudge them a few rows apart, since they are the two halves
        // of the split and do not have the same height.
        if let Some(visible_index) = self.conflict_resolver_visible_ix_for_nav_target(&target) {
            self.conflict_resolver_reveal_all_columns(visible_index);
        }

        // The snapshot shares the buffer's text and its line index, so this is
        // an `Arc` clone rather than the full copy-and-rescan of the output this
        // used to do on every jump.
        let output_snapshot = (!self.conflict_resolved_output_is_streamed()).then(|| {
            self.conflict_resolver_input
                .read_with(cx, |input, _| input.text_snapshot())
        });
        let output_line_count = output_snapshot
            .as_ref()
            .map(|snapshot| snapshot.shared_line_starts().len().max(1))
            .unwrap_or_else(|| self.conflict_resolved_preview_line_count.max(1));
        if let Some(output_line) = self.conflict_resolver_output_line_for_nav_target(
            &target,
            output_snapshot
                .as_ref()
                .map(|snapshot| snapshot.as_str())
                .unwrap_or(""),
        ) {
            self.conflict_resolver_reveal_resolved_output_line(output_line, output_line_count);
        }
        cx.notify();
    }

    pub(in crate::view) fn conflict_jump_prev(&mut self, cx: &mut gpui::Context<Self>) {
        let target = conflict_resolver::previous_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Conflict,
        );
        if let Some(target) = target {
            self.conflict_jump_to_nav_target(target, cx);
        }
    }

    pub(in crate::view) fn conflict_jump_next(&mut self, cx: &mut gpui::Context<Self>) {
        let target = conflict_resolver::next_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Conflict,
        );
        if let Some(target) = target {
            self.conflict_jump_to_nav_target(target, cx);
        }
    }

    pub(in crate::view) fn conflict_has_prev(&self) -> bool {
        conflict_resolver::previous_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Conflict,
        )
        .is_some()
    }

    pub(in crate::view) fn conflict_has_next(&self) -> bool {
        conflict_resolver::next_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Conflict,
        )
        .is_some()
    }

    pub(in crate::view) fn conflict_has_prev_delta(&self) -> bool {
        conflict_resolver::previous_conflict_nav_target_index(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Delta,
        )
        .is_some()
    }

    pub(in crate::view) fn conflict_has_next_delta(&self) -> bool {
        conflict_resolver::next_conflict_nav_target_index(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Delta,
        )
        .is_some()
    }

    /// Jump to the first changed merge target.
    pub(in crate::view) fn conflict_jump_first(&mut self, cx: &mut gpui::Context<Self>) {
        let target = self
            .conflict_resolver
            .nav_targets
            .iter()
            .position(|target| target.is_delta);
        if let Some(target) = target {
            self.conflict_jump_to_nav_target(target, cx);
        }
    }

    /// Jump to the last changed merge target.
    pub(in crate::view) fn conflict_jump_last(&mut self, cx: &mut gpui::Context<Self>) {
        let target = self
            .conflict_resolver
            .nav_targets
            .iter()
            .rposition(|target| target.is_delta);
        if let Some(target) = target {
            self.conflict_jump_to_nav_target(target, cx);
        }
    }

    pub(in crate::view) fn conflict_jump_next_unresolved(&mut self, cx: &mut gpui::Context<Self>) {
        let target = conflict_resolver::next_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Unresolved,
        );
        if let Some(target) = target {
            self.conflict_jump_to_nav_target(target, cx);
        }
    }

    pub(in crate::view) fn conflict_jump_prev_unresolved(&mut self, cx: &mut gpui::Context<Self>) {
        let target = conflict_resolver::previous_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Unresolved,
        );
        if let Some(target) = target {
            self.conflict_jump_to_nav_target(target, cx);
        }
    }

    pub(in crate::view) fn conflict_has_next_unresolved(&self) -> bool {
        conflict_resolver::next_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Unresolved,
        )
        .is_some()
    }

    pub(in crate::view) fn conflict_has_prev_unresolved(&self) -> bool {
        conflict_resolver::previous_conflict_nav_target_index_or_sole_anchor(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Unresolved,
        )
        .is_some()
    }

    pub(in crate::view::panes::main) fn conflict_resolver_two_way_visible_ix_for_conflict(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        self.conflict_resolver
            .two_way_visible_ix_for_conflict(conflict_ix)
    }

    /// Advance to the next unresolved conflict after a pick (kdiff3-style).
    pub(super) fn conflict_resolver_auto_advance_to_next_unresolved(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.mergetool_auto_advance {
            return;
        }
        let Some(current_display) = self.conflict_resolver.active_conflict else {
            return;
        };
        let Some(current_target) = self.conflict_resolver.selected_nav_target_index() else {
            return;
        };
        let current_is_resolved = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.resolved),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .nth(current_display)
            .unwrap_or(false);
        if !current_is_resolved {
            return;
        }
        let next_unresolved = conflict_resolver::next_conflict_nav_target_index(
            &self.conflict_resolver.nav_targets,
            self.conflict_resolver.nav_anchor,
            conflict_resolver::ConflictNavTargetFilter::Unresolved,
        )
        .or_else(|| {
            self.conflict_resolver
                .nav_targets
                .iter()
                .enumerate()
                .find(|(index, target)| *index != current_target && target.unresolved)
                .map(|(index, _)| index)
        });
        if let Some(next_unresolved) = next_unresolved {
            self.conflict_jump_to_nav_target(next_unresolved, cx);
        }
    }

    /// Select a conflict as the active one without picking a side (section 30:
    /// clicking a conflict block body selects it).
    pub(in crate::view) fn conflict_resolver_select_conflict(
        &mut self,
        conflict_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if conflict_ix >= self.conflict_resolver_conflict_count() {
            return;
        }
        if !self.conflict_resolver.select_display_conflict(conflict_ix) {
            return;
        }
        cx.notify();
    }
}
