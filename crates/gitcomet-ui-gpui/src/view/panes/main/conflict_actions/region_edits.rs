//! Region edits: row selection and split, manual alignment, and joining
//! regions.

use super::*;

impl MainPaneView {
    /// section 30 split: begin a drag selection of aligned rows at `aligned_row`
    /// inside conflict block `conflict_ix`. Also selects the block so the
    /// pick affordances follow. No-op when split is unavailable.
    pub(in crate::view) fn conflict_resolver_begin_row_selection(
        &mut self,
        conflict_ix: usize,
        aligned_row: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_resolver.conflict_row_selection_enabled() {
            return;
        }
        crate::press_gesture::claim_press(cx);
        let row = self
            .conflict_resolver
            .clamp_row_to_conflict_block(conflict_ix, aligned_row);
        if !self.conflict_resolver.select_display_conflict(conflict_ix) {
            return;
        }
        self.conflict_resolver.row_selection = Some(ConflictRowSelection {
            conflict_ix,
            anchor_row: row,
            head_row: row,
            selecting: true,
        });
        cx.notify();
    }

    /// Select a row with a keyboard modifier. Shift/Ctrl-click extends the
    /// existing contiguous selection from its anchor; without an existing
    /// selection it starts a single-row selection. Keeping the selection
    /// contiguous matches the split operation's byte-range surgery.
    pub(in crate::view) fn conflict_resolver_click_row_selection(
        &mut self,
        conflict_ix: usize,
        aligned_row: usize,
        modifiers: gpui::Modifiers,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_resolver.conflict_row_selection_enabled() {
            return;
        }
        let row = self
            .conflict_resolver
            .clamp_row_to_conflict_block(conflict_ix, aligned_row);
        let anchor = self
            .conflict_resolver
            .row_selection
            .filter(|selection| selection.conflict_ix == conflict_ix)
            .map(|selection| selection.anchor_row)
            .unwrap_or(row);
        let extend = modifiers.shift || modifiers.control;
        if !self.conflict_resolver.select_display_conflict(conflict_ix) {
            return;
        }
        self.conflict_resolver.row_selection = Some(ConflictRowSelection {
            conflict_ix,
            anchor_row: if extend { anchor } else { row },
            head_row: row,
            selecting: false,
        });
        cx.notify();
    }

    /// Extend the in-progress selection to `aligned_row`, clamped to the
    /// anchored block even when the pointer has entered a neighbouring block.
    pub(in crate::view) fn conflict_resolver_extend_row_selection(
        &mut self,
        _conflict_ix: usize,
        aligned_row: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(mut selection) = self.conflict_resolver.row_selection else {
            return;
        };
        if !selection.selecting {
            return;
        }
        let row = self
            .conflict_resolver
            .clamp_row_to_conflict_block(selection.conflict_ix, aligned_row);
        if row == selection.head_row {
            return;
        }
        selection.head_row = row;
        self.conflict_resolver.row_selection = Some(selection);
        cx.notify();
    }

    /// section 30 split: finish the drag (keeps the selected range for the menu).
    pub(in crate::view) fn conflict_resolver_end_row_selection(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(mut selection) = self.conflict_resolver.row_selection else {
            return;
        };
        if !selection.selecting {
            return;
        }
        selection.selecting = false;
        self.conflict_resolver.row_selection = Some(selection);
        cx.notify();
    }

    /// section 30 split: split the active row selection into its own conflict(s).
    /// Dispatches `Msg::ConflictSplitRegion`; the state round-trip rebuilds
    /// the resolver (which also clears the selection).
    pub(in crate::view) fn conflict_resolver_split_selection(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((region_index, boundaries)) =
            self.conflict_resolver.split_boundaries_for_selection()
        else {
            return;
        };
        if let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.dispatch_path(),
        ) {
            self.store.dispatch(Msg::ConflictSplitRegion {
                repo_id,
                path,
                region_index,
                boundaries,
                expected_conflict_rev: self.conflict_resolver.conflict_rev,
            });
        }
        // Keep the selection until the state round-trip confirms the edit.
        // A stale repo/path/session can make the reducer reject the request;
        // retaining it lets the user retry instead of silently losing work.
        cx.notify();
    }

    /// KDiff3 manual diff help: mark `line` of `column` for the next Ctrl+Y.
    ///
    /// `extend` grows that column's mark from its anchor. No-op when the
    /// current conflict has no real aligned row space to pin against.
    pub(in crate::view) fn conflict_resolver_mark_alignment_line(
        &mut self,
        column: ThreeWayColumn,
        line: usize,
        extend: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_resolver.manual_alignment_enabled() {
            return;
        }
        self.conflict_resolver
            .set_alignment_selection(column, line, extend);
        cx.notify();
    }

    /// KDiff3 manual diff help: drop the pending marks without pinning them.
    pub(in crate::view) fn conflict_resolver_clear_alignment_marks(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let cleared = self.conflict_resolver.clear_alignment_selections();
        if cleared {
            cx.notify();
        }
        cleared
    }

    /// KDiff3's `Ctrl+Y`: pin the marked lines onto one another and replan.
    ///
    /// Returns whether a request was dispatched. The marks are dropped
    /// immediately; the state round-trip rebuilds the resolver from the new
    /// plan, and a rejected entry simply leaves the plan as it was.
    pub(in crate::view) fn conflict_resolver_align_manually(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(alignment) = self
            .conflict_resolver
            .manual_alignment_from_selections(self.conflict_resolver_session_has_base())
        else {
            return false;
        };
        let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.dispatch_path(),
        ) else {
            return false;
        };
        self.store.dispatch(Msg::ConflictAddManualAlignment {
            repo_id,
            path,
            alignment,
            expected_conflict_rev: self.conflict_resolver.conflict_rev,
        });
        self.conflict_resolver.clear_alignment_selections();
        cx.notify();
        true
    }

    /// KDiff3's `Ctrl+Shift+Y`: drop every pinned alignment and replan.
    ///
    /// Also clears any pending marks, so one keystroke returns the file to its
    /// automatic alignment. Returns whether anything was dispatched.
    pub(in crate::view) fn conflict_resolver_clear_manual_alignments(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let cleared_marks = self.conflict_resolver.clear_alignment_selections();
        let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.dispatch_path(),
        ) else {
            if cleared_marks {
                cx.notify();
            }
            return cleared_marks;
        };
        self.store.dispatch(Msg::ConflictClearManualAlignments {
            repo_id,
            path,
            expected_conflict_rev: self.conflict_resolver.conflict_rev,
        });
        cx.notify();
        true
    }

    /// Whether the loaded session's plan carries a base, which decides whether
    /// a pinned entry uses three-input or true two-input source mapping.
    fn conflict_resolver_session_has_base(&self) -> bool {
        self.with_conflict_resolver_session(|session| {
            session
                .merge_plan
                .as_ref()
                .is_some_and(gitcomet_core::merge::MergePlan::has_base)
        })
    }

    /// Whether the loaded session already has pinned manual alignments.
    pub(in crate::view) fn conflict_resolver_has_manual_alignments(&self) -> bool {
        self.with_conflict_resolver_session(|session| !session.manual_alignments.is_empty())
    }

    /// How many source columns carry a pending alignment mark.
    pub(in crate::view) fn conflict_resolver_alignment_marked_columns(&self) -> usize {
        ThreeWayColumn::ALL
            .iter()
            .filter(|column| self.conflict_resolver.alignment_selection[**column].is_some())
            .count()
    }

    pub(in crate::view) fn conflict_resolver_join_regions(
        &mut self,
        target: ConflictResolverJoinTarget,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.repo_id != Some(target.repo_id)
            || self.conflict_resolver.dispatch_path().as_ref() != Some(&target.path)
            || self.conflict_resolver.conflict_rev != target.conflict_rev
        {
            return;
        }
        let snapshot = self.store.snapshot();
        let target_is_current = snapshot
            .repos
            .iter()
            .find(|repo| repo.id == target.repo_id)
            .is_some_and(|repo| {
                repo.conflict_state.conflict_rev == target.conflict_rev
                    && repo.conflict_state.conflict_file_path.as_deref()
                        == Some(target.path.as_path())
                    && repo
                        .conflict_state
                        .conflict_session
                        .as_ref()
                        .is_some_and(|session| {
                            session.path == target.path.as_path()
                                && target
                                    .first_region_index
                                    .checked_add(1)
                                    .is_some_and(|next| next < session.regions.len())
                        })
            });
        if !target_is_current {
            return;
        }

        if let Some(conflict_ix) = self
            .conflict_resolver
            .conflict_region_indices
            .iter()
            .position(|&region_index| region_index == target.first_region_index)
        {
            let _ = self.conflict_resolver.select_display_conflict(conflict_ix);
        }
        self.conflict_resolver.row_selection = None;
        self.store.dispatch(Msg::ConflictJoinRegions {
            repo_id: target.repo_id,
            path: target.path,
            region_index: target.first_region_index,
            expected_conflict_rev: target.conflict_rev,
        });
        cx.notify();
    }
}
