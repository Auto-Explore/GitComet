//! Resolution commands: picks, choice application, session resolution
//! dispatch, bulk choices, and resolution counts.

use super::*;

impl MainPaneView {
    pub(in crate::view) fn conflict_resolver_apply_pick_target(
        &mut self,
        target: ResolverPickTarget,
        cx: &mut gpui::Context<Self>,
    ) {
        match target {
            ResolverPickTarget::ThreeWayLine { line_ix, choice } => {
                self.conflict_resolver_append_three_way_line_to_output(line_ix, choice, cx);
            }
            ResolverPickTarget::TwoWaySplitLine { row_ix, side } => {
                self.conflict_resolver_append_split_line_to_output(row_ix, side, cx);
            }
            ResolverPickTarget::Chunk {
                conflict_ix,
                choice,
                output_line_ix,
            } => {
                let target_conflict_ix = if let Some(output_line_ix) = output_line_ix {
                    if self.conflict_resolved_output_is_streamed() {
                        self.conflict_resolver_split_chunk_target_for_output_line(
                            conflict_ix,
                            output_line_ix,
                            "",
                        )
                    } else {
                        let current_output = self
                            .conflict_resolver_input
                            .read_with(cx, |i, _| i.text().to_string());
                        self.conflict_resolver_split_chunk_target_for_output_line(
                            conflict_ix,
                            output_line_ix,
                            &current_output,
                        )
                    }
                } else {
                    conflict_ix
                };

                let selected_choices =
                    self.conflict_resolver_selected_choices_for_conflict_ix(target_conflict_ix);
                if selected_choices.contains(&choice) {
                    self.conflict_resolver_reset_choice_for_chunk(target_conflict_ix, choice, cx);
                    return;
                }
                if output_line_ix.is_some()
                    && !selected_choices.is_empty()
                    && self.conflict_resolver_append_choice_for_chunk(
                        target_conflict_ix,
                        choice,
                        cx,
                    )
                {
                    return;
                }

                if self.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay {
                    self.conflict_resolver_pick_three_way_chunk_at(target_conflict_ix, choice, cx);
                } else {
                    self.conflict_resolver_pick_at(target_conflict_ix, choice, cx);
                }
            }
        }
    }

    pub(in crate::view::panes::main) fn conflict_resolver_split_chunk_target_for_output_line(
        &mut self,
        fallback_conflict_ix: usize,
        output_line_ix: usize,
        output_text: &str,
    ) -> usize {
        if self.conflict_resolved_output_is_streamed() {
            let Some(marker) = self
                .conflict_resolver
                .resolved_outline
                .markers
                .get(output_line_ix)
                .copied()
                .flatten()
            else {
                return fallback_conflict_ix;
            };
            let target_conflict_ix = marker.conflict_ix;
            // Streamed bootstrap now keeps one coarse marker range per block.
            // If the user explicitly interacts with a line inside that block,
            // split it lazily and then remap the click to the new subchunk.
            if !split_target_conflict_block_into_subchunks(
                &mut self.conflict_resolver.marker_segments,
                &mut self.conflict_resolver.conflict_region_indices,
                target_conflict_ix,
            ) {
                return target_conflict_ix;
            }
            self.conflict_resolver.display_plan_block_indices.clear();
            self.conflict_resolver_rebuild_visible_map();
            let output_path = self.conflict_resolver.path.clone();
            self.refresh_streamed_resolved_output_preview_from_markers(output_path.as_ref());
            return self
                .conflict_resolver
                .resolved_outline
                .markers
                .get(output_line_ix)
                .copied()
                .flatten()
                .map(|marker| marker.conflict_ix)
                .unwrap_or(target_conflict_ix);
        }

        let Some(marker) = resolved_output_marker_for_line(
            &self.conflict_resolver.marker_segments,
            output_text,
            output_line_ix,
            &self.conflict_resolved_output_block_map,
        ) else {
            return fallback_conflict_ix;
        };
        let target_conflict_ix = marker.conflict_ix;
        let marker_count_for_conflict = resolved_output_markers_for_text(
            &self.conflict_resolver.marker_segments,
            output_text,
            &self.conflict_resolved_output_block_map,
        )
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == target_conflict_ix && m.is_start)
        .count();
        if marker_count_for_conflict <= 1 {
            return target_conflict_ix;
        }

        if !split_target_conflict_block_into_subchunks(
            &mut self.conflict_resolver.marker_segments,
            &mut self.conflict_resolver.conflict_region_indices,
            target_conflict_ix,
        ) {
            return target_conflict_ix;
        }
        self.conflict_resolver.display_plan_block_indices.clear();
        self.conflict_resolver_rebuild_visible_map();

        resolved_output_marker_for_line(
            &self.conflict_resolver.marker_segments,
            output_text,
            output_line_ix,
            &self.conflict_resolved_output_block_map,
        )
        .map(|m| m.conflict_ix)
        .unwrap_or(target_conflict_ix)
    }

    pub(in crate::view::panes::main) fn conflict_resolver_append_choice_for_chunk(
        &mut self,
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(inserted_conflict_ix) = append_choice_after_conflict_block(
            &mut self.conflict_resolver.marker_segments,
            &mut self.conflict_resolver.conflict_region_indices,
            conflict_ix,
            choice,
        ) else {
            return false;
        };
        self.conflict_resolver.display_plan_block_indices.clear();
        self.conflict_resolver_rebuild_visible_map();
        let _ = self
            .conflict_resolver
            .select_display_conflict(inserted_conflict_ix);
        self.conflict_resolver_refresh_output_and_scroll(Some(inserted_conflict_ix), cx);
        cx.notify();
        true
    }

    pub(in crate::view::panes::main) fn conflict_resolver_reset_choice_for_chunk(
        &mut self,
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        let matching_indices = conflict_group_indices_for_choice(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            conflict_ix,
            choice,
        );
        self.conflict_resolver_reset_block_indices(matching_indices, conflict_ix, cx);
    }

    /// Un-resolve the active conflict regardless of how it was resolved
    /// (section 30: one keypress reverts a pick or auto-resolution).
    pub(in crate::view) fn conflict_resolver_unresolve_active_conflict(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.active_conflict.is_none()
            && let Some(conflict_resolver::ConflictNavTargetId::PlanBlock(block_id)) = self
                .conflict_resolver
                .selected_nav_target_index()
                .and_then(|index| self.conflict_resolver.nav_targets.get(index))
                .map(|target| target.id)
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.dispatch_path(),
            )
        {
            self.store.dispatch(Msg::ConflictReplacePlanBlockSelection {
                repo_id,
                path,
                block_id,
                selection: gitcomet_core::merge::OrderedSelection::new(),
            });
            cx.notify();
            return;
        }
        let Some(conflict_ix) = self.conflict_resolver.active_conflict else {
            return;
        };
        let resolved_flags: Vec<bool> = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.resolved),
                _ => None,
            })
            .collect();
        let matching_indices: Vec<usize> = conflict_group_member_indices_for_ix(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            conflict_ix,
        )
        .into_iter()
        .filter(|&ix| resolved_flags.get(ix).copied().unwrap_or(false))
        .collect();
        self.conflict_resolver_reset_block_indices(matching_indices, conflict_ix, cx);
    }

    fn conflict_resolver_reset_block_indices(
        &mut self,
        mut matching_indices: Vec<usize>,
        conflict_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.output_is_protected || matching_indices.is_empty() {
            return;
        }
        matching_indices.sort_unstable();
        matching_indices.dedup();
        let output_block_indices = matching_indices.clone();

        let mut changed = false;
        for ix in matching_indices.into_iter().rev() {
            changed |= reset_conflict_block_selection(
                &mut self.conflict_resolver.marker_segments,
                &mut self.conflict_resolver.conflict_region_indices,
                ix,
            );
        }
        if !changed {
            return;
        }

        let total_conflicts =
            conflict_resolver::conflict_count(&self.conflict_resolver.marker_segments);
        let selected_conflict =
            (total_conflicts > 0).then(|| conflict_ix.min(total_conflicts.saturating_sub(1)));
        let selected_conflict_ix = selected_conflict.unwrap_or(0);
        self.conflict_resolver.display_plan_block_indices.clear();
        self.conflict_resolver_rebuild_visible_map();
        if let Some(selected_conflict) = selected_conflict {
            let _ = self
                .conflict_resolver
                .select_display_conflict(selected_conflict);
        }
        let target_output_line = if total_conflicts == 0 {
            None
        } else if self.conflict_resolved_output_is_streamed() {
            let output_path = self.conflict_resolver.path.clone();
            self.refresh_streamed_resolved_output_preview_from_markers(output_path.as_ref());
            self.conflict_resolved_output_projection
                .as_ref()
                .and_then(|projection| projection.conflict_line_range(selected_conflict_ix))
                .map(|range| range.start)
        } else {
            if self.conflict_resolver_replace_mapped_blocks(&output_block_indices, cx) {
                let target_output_line =
                    self.conflict_resolver_mapped_block_output_line(selected_conflict_ix, cx);
                if let Some(target_line_ix) = target_output_line {
                    let line_count = self
                        .conflict_resolver_input
                        .read_with(cx, |input, _| split_line_count(input.text()));
                    self.conflict_resolver_scroll_resolved_output_to_line(
                        target_line_ix,
                        line_count,
                    );
                }
                target_output_line
            } else {
                let next = conflict_resolver::generate_resolved_text(
                    &self.conflict_resolver.marker_segments,
                );
                let target_output_line = output_line_range_for_conflict_block_in_text(
                    &self.conflict_resolver.marker_segments,
                    &next,
                    selected_conflict_ix,
                )
                .map(|range| range.start);
                self.conflict_resolver_set_output(next.clone(), cx);
                self.rebuild_conflict_resolved_output_block_map(cx);
                if let Some(target_line_ix) = target_output_line {
                    self.conflict_resolver_scroll_resolved_output_to_line_in_text(
                        target_line_ix,
                        &next,
                    );
                }
                target_output_line
            }
        };
        if let Some(target_line_ix) = target_output_line
            && self.conflict_resolved_output_is_streamed()
        {
            self.conflict_resolver_scroll_resolved_output_to_line(
                target_line_ix,
                self.conflict_resolved_preview_line_count,
            );
        }
        let should_sync_region = self
            .conflict_resolver
            .conflict_region_indices
            .get(selected_conflict_ix)
            .copied()
            .is_some_and(|region_ix| {
                conflict_region_index_is_unique(
                    &self.conflict_resolver.conflict_region_indices,
                    region_ix,
                )
            });
        if should_sync_region {
            if self.conflict_resolved_output_is_streamed() {
                self.conflict_resolver_sync_session_resolutions_from_segments();
            } else {
                let output_text = self
                    .conflict_resolver_input
                    .read_with(cx, |input, _| input.text().to_string());
                self.conflict_resolver_sync_session_resolutions_from_output(&output_text);
            }
        }
        cx.notify();
    }

    /// Validate and apply a choice to the active conflict block, dispatching to
    /// the session store if the region index is unique. Returns `false` if the
    /// block was not found or the choice was invalid (e.g. Base with no ancestor).
    fn conflict_resolver_apply_block_choice(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
    ) -> bool {
        if self.conflict_resolver.output_is_protected {
            return false;
        }
        let selected_plan_target = self
            .conflict_resolver
            .selected_nav_target_index()
            .and_then(|index| self.conflict_resolver.nav_targets.get(index))
            .and_then(|target| match target.id {
                conflict_resolver::ConflictNavTargetId::PlanBlock(block_id) => {
                    Some((block_id, target.display_conflict_index))
                }
                conflict_resolver::ConflictNavTargetId::Region(_)
                | conflict_resolver::ConflictNavTargetId::DisplayBlock(_) => None,
            });
        if let Some((block_id, display_conflict_index)) = selected_plan_target {
            return self.conflict_resolver_apply_plan_block_choice(
                block_id,
                display_conflict_index,
                choice,
            );
        }

        let Some(conflict_ix) = self.conflict_resolver.active_conflict else {
            return false;
        };
        let picked_region_index = self
            .conflict_resolver
            .conflict_region_indices
            .get(conflict_ix)
            .copied()
            .unwrap_or(conflict_ix);
        let dispatch_region_choice = conflict_region_index_is_unique(
            &self.conflict_resolver.conflict_region_indices,
            picked_region_index,
        );
        let dispatch = {
            let Some(block) = self.conflict_resolver_active_block_mut() else {
                return false;
            };
            let has_base = block.base.is_some();
            if choice.contains(gitcomet_core::conflict_output::ConflictOutputSource::Base)
                && !has_base
            {
                return false;
            }
            let to_merge_source = |source| {
                use gitcomet_core::conflict_output::ConflictOutputSource as Output;
                use gitcomet_core::merge::MergeSource;
                match (has_base, source) {
                    (true, Output::Base) => Some(MergeSource::A),
                    (true, Output::Ours) => Some(MergeSource::B),
                    (true, Output::Theirs) => Some(MergeSource::C),
                    (false, Output::Base) => None,
                    (false, Output::Ours) => Some(MergeSource::A),
                    (false, Output::Theirs) => Some(MergeSource::B),
                }
            };

            if choice == conflict_resolver::ConflictChoice::Both {
                block.choice = choice;
                block.resolved = true;
                Some(Ok(gitcomet_core::merge::OrderedSelection::from_sources(
                    choice.iter().filter_map(to_merge_source),
                )))
            } else if choice.len() == 1 {
                let Some(output_source) = choice.first() else {
                    return false;
                };
                let Some(source) = to_merge_source(output_source) else {
                    return false;
                };
                if !block.resolved {
                    block.choice = conflict_resolver::ConflictChoice::empty();
                }
                block.choice.toggle(output_source);
                block.resolved = !block.choice.is_empty();
                Some(Err(source))
            } else {
                block.choice = choice;
                block.resolved = !choice.is_empty();
                Some(Ok(gitcomet_core::merge::OrderedSelection::from_sources(
                    choice.iter().filter_map(to_merge_source),
                )))
            }
        };
        if dispatch_region_choice
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.dispatch_path(),
            )
        {
            match dispatch {
                Some(Err(source)) => self.store.dispatch(Msg::ConflictToggleRegionSource {
                    repo_id,
                    path,
                    region_index: picked_region_index,
                    source,
                }),
                Some(Ok(selection)) => self.store.dispatch(Msg::ConflictReplaceRegionSelection {
                    repo_id,
                    path,
                    region_index: picked_region_index,
                    selection,
                }),
                None => {}
            }
        }
        true
    }

    fn conflict_resolver_apply_plan_block_choice(
        &mut self,
        block_id: gitcomet_core::merge::MergeBlockId,
        display_conflict_index: Option<usize>,
        choice: conflict_resolver::ConflictChoice,
    ) -> bool {
        let Some((has_base, local_source, remote_source)) =
            self.with_conflict_resolver_session(|session| {
                let plan = session.merge_plan.as_ref()?;
                plan.blocks
                    .iter()
                    .any(|block| block.id == block_id)
                    .then_some((plan.has_base(), plan.local_source(), plan.remote_source()))
            })
        else {
            return false;
        };
        let to_merge_source = |source| {
            use gitcomet_core::conflict_output::ConflictOutputSource as Output;
            use gitcomet_core::merge::MergeSource;
            match (has_base, source) {
                (true, Output::Base) => Some(MergeSource::A),
                (true, Output::Ours) => Some(MergeSource::B),
                (true, Output::Theirs) => Some(MergeSource::C),
                (false, Output::Base) => None,
                (false, Output::Ours) => Some(MergeSource::A),
                (false, Output::Theirs) => Some(MergeSource::B),
            }
        };
        if choice
            .iter()
            .any(|source| to_merge_source(source).is_none())
        {
            return false;
        }

        let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.dispatch_path(),
        ) else {
            return false;
        };

        if choice == conflict_resolver::ConflictChoice::Both {
            self.store.dispatch(Msg::ConflictReplacePlanBlockSelection {
                repo_id,
                path,
                block_id,
                selection: gitcomet_core::merge::OrderedSelection::from_sources([
                    local_source,
                    remote_source,
                ]),
            });
        } else if choice.len() == 1 {
            let Some(source) = choice.first().and_then(to_merge_source) else {
                return false;
            };
            self.store.dispatch(Msg::ConflictTogglePlanBlockSource {
                repo_id,
                path,
                block_id,
                source,
            });
        } else {
            self.store.dispatch(Msg::ConflictReplacePlanBlockSelection {
                repo_id,
                path,
                block_id,
                selection: gitcomet_core::merge::OrderedSelection::from_sources(
                    choice.iter().filter_map(to_merge_source),
                ),
            });
        }

        // Preserve the existing immediate feedback for a marker-backed plan
        // target. Plan-only automatic deltas update on the conflict-revision
        // resync, which re-renders their surrounding plain-text projection.
        if let Some(conflict_ix) = display_conflict_index {
            self.conflict_resolver.active_conflict = Some(conflict_ix);
            if let Some(block) = self.conflict_resolver_active_block_mut() {
                if choice == conflict_resolver::ConflictChoice::Both {
                    block.choice = choice;
                    block.resolved = true;
                } else if choice.len() == 1 {
                    let Some(output_source) = choice.first() else {
                        return false;
                    };
                    if !block.resolved {
                        block.choice = conflict_resolver::ConflictChoice::empty();
                    }
                    block.choice.toggle(output_source);
                    block.resolved = !block.choice.is_empty();
                } else {
                    block.choice = choice;
                    block.resolved = !choice.is_empty();
                }
            }
        }
        true
    }

    pub(in crate::view) fn conflict_resolver_sync_session_resolutions_from_output(
        &mut self,
        output_text: &str,
    ) {
        let Some(updates) = conflict_resolver::derive_region_resolution_updates_from_output(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            &self.conflict_resolved_output_block_map,
            output_text,
        ) else {
            return;
        };
        self.conflict_resolver_dispatch_session_resolution_updates(updates);
    }

    pub(in crate::view) fn conflict_resolver_sync_session_resolutions_from_segments(&mut self) {
        let updates = conflict_resolver::derive_region_resolution_updates_from_segments(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
        );
        self.conflict_resolver_dispatch_session_resolution_updates(updates);
    }

    fn conflict_resolver_dispatch_session_resolution_updates(
        &mut self,
        updates: Vec<(
            usize,
            gitcomet_core::conflict_session::ConflictRegionResolution,
        )>,
    ) {
        if updates.is_empty() {
            return;
        }
        let Some(repo_id) = self
            .conflict_resolver
            .repo_id
            .or_else(|| self.active_repo_id())
        else {
            return;
        };
        let Some(path) = self.conflict_resolver.dispatch_path() else {
            return;
        };
        let updates = updates
            .into_iter()
            .map(
                |(region_index, resolution)| gitcomet_state::msg::ConflictRegionResolutionUpdate {
                    region_index,
                    resolution,
                },
            )
            .collect();
        self.store.dispatch(Msg::ConflictSyncRegionResolutions {
            repo_id,
            path,
            updates,
        });
    }

    pub(in crate::view) fn conflict_resolver_reset_output_from_markers(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(current) = self.conflict_resolver.current.as_deref() else {
            return;
        };
        let mut segments = conflict_resolver::parse_conflict_markers(current);
        if conflict_resolver::conflict_count(&segments) == 0 {
            return;
        }
        // Same reason as the bootstrap and resync paths: 2-way markers carry no
        // base section, so without the ancestor every block reads as base-less.
        // That rejects "A (base)" outright and shifts the Ours/Theirs mapping in
        // `conflict_resolver_apply_block_choice` onto the two-input sources, so
        // the store round-trip hands back a selection the blocks cannot express
        // and every pick after a reset looks like it did nothing.
        if let Some(base_text) = self
            .conflict_resolver
            .loaded_file
            .as_ref()
            .and_then(|file| file.base.clone())
        {
            conflict_resolver::populate_block_bases_from_shared_ancestor(&mut segments, base_text);
        }
        // Asking for the markers back is asking for the stage projection over
        // the worktree payload. Record that, or the resync this reset triggers
        // re-derives protection from the very payload the user just overrode.
        self.conflict_resolver.output_protection_waived = true;
        self.conflict_resolver.output_is_protected = false;
        self.conflict_resolver.marker_segments = segments;
        self.conflict_resolver.conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(
                &self.conflict_resolver.marker_segments,
            );
        self.conflict_resolver.display_plan_block_indices.clear();
        self.conflict_resolver.last_autosolve_summary = None;
        self.conflict_resolver.open_summary_counts = None;
        self.conflict_resolver_rebuild_visible_map();
        let _ = self.conflict_resolver.select_display_conflict(0);
        self.conflict_resolver_refresh_output_and_scroll(None, cx);
        if let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.dispatch_path(),
        ) {
            self.store
                .dispatch(Msg::ConflictResetResolutions { repo_id, path });
        }
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolver_conflict_count(&self) -> usize {
        let (total, _) = conflict_resolver::effective_conflict_counts(
            &self.conflict_resolver.marker_segments,
            self.conflict_resolver_session_counts(),
        );
        total
    }

    pub(in crate::view) fn conflict_resolver_session_counts(&self) -> Option<(usize, usize)> {
        let resolver_path = self.conflict_resolver.path.as_ref()?;
        let session = self
            .active_repo()?
            .conflict_state
            .conflict_session
            .as_ref()?;
        if session.path.as_path() != resolver_path.as_path() {
            return None;
        }
        Some((session.total_regions(), session.solved_count()))
    }

    pub(in crate::view) fn conflict_resolver_summary_counts(
        &self,
    ) -> Option<conflict_resolver::ConflictSummaryCounts> {
        let resolver_path = self.conflict_resolver.path.as_ref()?;
        let session = self
            .active_repo()?
            .conflict_state
            .conflict_session
            .as_ref()?;
        if session.path.as_path() != resolver_path.as_path() {
            return None;
        }
        Some(conflict_resolver::conflict_session_summary_counts(session))
    }

    pub(in crate::view::panes::main) fn conflict_resolver_active_block_mut(
        &mut self,
    ) -> Option<&mut conflict_resolver::ConflictBlock> {
        let target = self.conflict_resolver.active_conflict?;
        let mut seen = 0usize;
        for seg in &mut self.conflict_resolver.marker_segments {
            let conflict_resolver::ConflictSegment::Block(block) = seg else {
                continue;
            };
            if seen == target {
                return Some(block);
            }
            seen += 1;
        }
        None
    }

    pub(in crate::view) fn conflict_resolver_pick_at(
        &mut self,
        range_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_resolver.select_display_conflict(range_ix) {
            return;
        }
        self.conflict_resolver_pick_active_conflict(choice, cx);
    }

    pub(in crate::view) fn conflict_resolver_pick_three_way_chunk_at(
        &mut self,
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver_conflict_count() == 0 {
            return;
        }
        if self.conflict_resolver.view_mode != ConflictResolverViewMode::ThreeWay {
            self.conflict_resolver_pick_at(conflict_ix, choice, cx);
            return;
        }

        if !self.conflict_resolver.select_display_conflict(conflict_ix) {
            return;
        }
        self.conflict_resolver.hovered_conflict = None;
        if !self.conflict_resolver_apply_block_choice(choice) {
            return;
        }

        self.conflict_resolver_rebuild_visible_map();
        if self.conflict_resolved_output_is_streamed() {
            let output_path = self.conflict_resolver.path.clone();
            self.refresh_streamed_resolved_output_preview_from_markers(output_path.as_ref());
            if let Some(target_line_ix) = self
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
            if !self.conflict_resolver_replace_mapped_blocks(&[conflict_ix], cx) {
                return;
            }
            if let Some(target_output_line) =
                self.conflict_resolver_mapped_block_output_line(conflict_ix, cx)
            {
                let line_count = self
                    .conflict_resolver_input
                    .read_with(cx, |input, _| split_line_count(input.text()));
                self.conflict_resolver_scroll_resolved_output_to_line(
                    target_output_line,
                    line_count,
                );
            }
        }

        self.conflict_resolver_auto_advance_to_next_unresolved(cx);
        cx.notify();
    }

    /// Confidence tier of the auto-resolve rule applied to a conflict, when
    /// its session region is `AutoResolved` (section 30 gutter badges).
    pub(in crate::view) fn conflict_autosolve_confidence_for_ix(
        &self,
        conflict_ix: usize,
    ) -> Option<gitcomet_core::conflict_session::AutosolveConfidence> {
        let region_ix = self
            .conflict_resolver
            .conflict_region_indices
            .get(conflict_ix)
            .copied()?;
        let session = self
            .active_repo()?
            .conflict_state
            .conflict_session
            .as_ref()?;
        match &session.regions.get(region_ix)?.resolution {
            gitcomet_core::conflict_session::ConflictRegionResolution::AutoResolved {
                confidence,
                ..
            } => Some(*confidence),
            _ => None,
        }
    }

    /// Count conflicts currently resolved by the autosolver (as opposed to
    /// user picks), for the toolbar's "(N auto)" indicator.
    pub(in crate::view) fn conflict_resolver_auto_resolved_count(&self) -> usize {
        (0..self.conflict_resolver_conflict_count())
            .filter(|ix| self.conflict_autosolve_confidence_for_ix(*ix).is_some())
            .count()
    }

    /// Pick-control state for the semantic current delta. A plan-backed target
    /// remains actionable even when it is automatically resolved and therefore
    /// has no displayed marker block.
    pub(in crate::view) fn conflict_resolver_active_pick_state(
        &self,
    ) -> Option<(bool, Vec<conflict_resolver::ConflictChoice>)> {
        let target = self
            .conflict_resolver
            .selected_nav_target_index()
            .and_then(|index| self.conflict_resolver.nav_targets.get(index));
        if let Some(conflict_resolver::ConflictNavTarget {
            id: conflict_resolver::ConflictNavTargetId::PlanBlock(block_id),
            is_delta: true,
            ..
        }) = target
        {
            return self.with_conflict_resolver_session(|session| {
                let plan = session.merge_plan.as_ref()?;
                let block = plan.blocks.iter().find(|block| block.id == *block_id)?;
                let selected = block
                    .selection
                    .iter()
                    .filter_map(|source| {
                        conflict_resolver::choice_for_selection(&source.into(), plan.has_base())
                    })
                    .collect();
                Some((plan.has_base(), selected))
            });
        }

        let conflict_ix = self.conflict_resolver.active_conflict?;
        Some((
            self.conflict_resolver
                .conflict_has_base
                .get(conflict_ix)
                .copied()
                .unwrap_or(false),
            self.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix),
        ))
    }

    pub(in crate::view) fn conflict_resolver_has_active_pick_target(&self) -> bool {
        self.conflict_resolver_active_pick_state().is_some()
    }

    /// Read a value off the conflict session currently loaded in the resolver.
    /// Read the loaded conflict session, or `T::default()` if there is none.
    ///
    /// Reads the **store**, while the resolver around it was built from the UI
    /// model. Production keeps the two in lockstep — `poller.rs` feeds the model
    /// from `store.snapshot()` — so this is sound there, but it is an invariant
    /// nothing enforces. It has already broken once: a test harness that
    /// published state to the model alone left this returning `None`, and every
    /// plan-block pick behind it became a silent no-op that still reported
    /// success. `push_test_state` now publishes to both; anything else that
    /// injects state must do the same.
    pub(super) fn with_conflict_resolver_session<T: Default>(
        &self,
        read: impl FnOnce(&gitcomet_core::conflict_session::ConflictSession) -> T,
    ) -> T {
        let Some(path) = self.conflict_resolver.path.as_deref() else {
            return T::default();
        };
        self.store
            .snapshot()
            .repos
            .iter()
            .find(|repo| Some(repo.id) == self.conflict_resolver.repo_id)
            .and_then(|repo| repo.conflict_state.conflict_session.as_ref())
            .filter(|session| session.path == path)
            .map(read)
            .unwrap_or_default()
    }

    pub(in crate::view) fn conflict_resolver_pick_active_conflict(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        let picked_conflict_index = self
            .conflict_resolver
            .selected_nav_target_index()
            .and_then(|target_index| {
                self.conflict_resolver.nav_targets[target_index].display_conflict_index
            })
            .or(self.conflict_resolver.active_conflict);
        if picked_conflict_index.is_none() && !self.conflict_resolver_has_active_pick_target() {
            return;
        }
        if !self.conflict_resolver_apply_block_choice(choice) {
            return;
        }
        if let Some(picked_conflict_index) = picked_conflict_index {
            self.conflict_resolver_rebuild_visible_map();
            self.conflict_resolver_refresh_output_and_scroll(Some(picked_conflict_index), cx);
            self.conflict_resolver_auto_advance_to_next_unresolved(cx);
        }

        cx.notify();
    }

    /// KDiff3's Choose A/B/C Everywhere: replace every semantic delta,
    /// including automatically selected ones that have no marker region.
    pub(in crate::view) fn conflict_resolver_choose_everywhere(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.output_is_protected {
            return;
        }
        self.conflict_resolver_dispatch_bulk_choice(
            choice,
            gitcomet_state::msg::ConflictBulkScope::AllDeltas,
            cx,
        );
    }

    fn conflict_resolver_dispatch_bulk_choice(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
        scope: gitcomet_state::msg::ConflictBulkScope,
        cx: &mut gpui::Context<Self>,
    ) {
        let bulk_choice = if choice == conflict_resolver::ConflictChoice::Base {
            gitcomet_state::msg::ConflictBulkChoice::Base
        } else if choice == conflict_resolver::ConflictChoice::Ours {
            gitcomet_state::msg::ConflictBulkChoice::Ours
        } else if choice == conflict_resolver::ConflictChoice::Theirs {
            gitcomet_state::msg::ConflictBulkChoice::Theirs
        } else if choice == conflict_resolver::ConflictChoice::Both {
            gitcomet_state::msg::ConflictBulkChoice::Both
        } else {
            return;
        };
        let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.dispatch_path(),
        ) else {
            return;
        };
        self.store.dispatch(Msg::ConflictApplyBulkChoice {
            repo_id,
            path,
            choice: bulk_choice,
            scope,
        });
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolver_resolved_count(&self) -> usize {
        let (_, resolved) = conflict_resolver::effective_conflict_counts(
            &self.conflict_resolver.marker_segments,
            self.conflict_resolver_session_counts(),
        );
        resolved
    }
}
