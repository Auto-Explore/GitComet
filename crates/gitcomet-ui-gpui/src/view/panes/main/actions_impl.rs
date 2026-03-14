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
                diff_navigation::change_block_entries(self.diff_visible_len(), |visible_ix| {
                    let Some(inline_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return false;
                    };
                    self.file_diff_inline_cache.get(inline_ix).is_some_and(|l| {
                        matches!(
                            l.kind,
                            gitcomet_core::domain::DiffLineKind::Add
                                | gitcomet_core::domain::DiffLineKind::Remove
                        )
                    })
                })
            }
            DiffViewMode::Split => {
                diff_navigation::change_block_entries(self.diff_visible_len(), |visible_ix| {
                    let Some(row_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return false;
                    };
                    self.file_diff_cache_rows.get(row_ix).is_some_and(|row| {
                        !matches!(row.kind, gitcomet_core::file_diff::FileDiffRowKind::Context)
                    })
                })
            }
        }
    }

    pub(super) fn patch_hunk_entries(&self) -> Vec<(usize, usize)> {
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
                    if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
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
                        out.push((visible_ix, src_ix));
                    }
                }
            }
        }
        out
    }

    pub(in crate::view) fn diff_nav_entries(&self) -> Vec<usize> {
        if self.is_file_diff_view_active() {
            return self.file_change_visible_indices();
        }
        self.patch_hunk_entries()
            .into_iter()
            .map(|(visible_ix, _)| visible_ix)
            .collect()
    }

    pub(super) fn conflict_marker_nav_entries(&self) -> Vec<usize> {
        conflict_marker_nav_entries_from_markers(
            &self.conflict_resolver.resolved_output_conflict_markers,
        )
    }

    pub(super) fn conflict_fallback_nav_entries(&self) -> Vec<usize> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => {
                conflict_resolver::unresolved_visible_nav_entries_for_three_way(
                    &self.conflict_resolver.marker_segments,
                    &self.conflict_resolver.three_way_visible_map,
                    &self.conflict_resolver.three_way_conflict_ranges,
                )
            }
            ConflictResolverViewMode::TwoWayDiff => match self.conflict_resolver.diff_mode {
                ConflictDiffMode::Split => {
                    conflict_resolver::unresolved_visible_nav_entries_for_two_way(
                        &self.conflict_resolver.marker_segments,
                        &self.conflict_resolver.diff_row_conflict_map,
                        &self.conflict_resolver.diff_visible_row_indices,
                    )
                }
                ConflictDiffMode::Inline => {
                    conflict_resolver::unresolved_visible_nav_entries_for_two_way(
                        &self.conflict_resolver.marker_segments,
                        &self.conflict_resolver.inline_row_conflict_map,
                        &self.conflict_resolver.inline_visible_row_indices,
                    )
                }
            },
        }
    }

    pub(in crate::view) fn conflict_nav_entries(&self) -> Vec<usize> {
        let marker_entries = self.conflict_marker_nav_entries();
        if !marker_entries.is_empty() {
            return marker_entries;
        }
        self.conflict_fallback_nav_entries()
    }

    pub(super) fn conflict_resolver_visible_ix_for_conflict(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => conflict_resolver::visible_index_for_conflict(
                &self.conflict_resolver.three_way_visible_map,
                &self.conflict_resolver.three_way_conflict_ranges,
                conflict_ix,
            ),
            ConflictResolverViewMode::TwoWayDiff => {
                self.conflict_resolver_two_way_visible_ix_for_conflict(conflict_ix)
            }
        }
    }

    pub(super) fn conflict_resolver_output_line_for_conflict(
        &self,
        conflict_ix: usize,
        output_text: &str,
    ) -> Option<usize> {
        // Prefer the conflict block's start line so keyboard navigation keeps
        // the three-way input panes and resolved output aligned to the same anchor.
        output_line_range_for_conflict_block_in_text(
            &self.conflict_resolver.marker_segments,
            output_text,
            conflict_ix,
        )
        .map(|range| range.start)
        .or_else(|| {
            first_output_marker_line_for_conflict(
                &self.conflict_resolver.resolved_output_conflict_markers,
                conflict_ix,
            )
        })
    }

    pub(super) fn conflict_resolver_scroll_all_views_to_conflict(
        &mut self,
        conflict_ix: usize,
        input_visible_hint: Option<usize>,
        output_line_hint: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(target) = input_visible_hint
            .or_else(|| self.conflict_resolver_visible_ix_for_conflict(conflict_ix))
        {
            self.conflict_resolver_diff_scroll
                .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
        }

        let output_text = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text().to_string());
        let output_line_count = output_text.split('\n').count().max(1);
        if let Some(target_line) = output_line_hint
            .or_else(|| self.conflict_resolver_output_line_for_conflict(conflict_ix, &output_text))
        {
            self.conflict_resolver_scroll_resolved_output_to_line(target_line, output_line_count);
        }
    }

    pub(in crate::view) fn conflict_jump_prev(&mut self, cx: &mut gpui::Context<Self>) {
        let marker_entries = self.conflict_marker_nav_entries();
        let use_marker_nav = !marker_entries.is_empty();
        let entries = if use_marker_nav {
            marker_entries
        } else {
            self.conflict_fallback_nav_entries()
        };
        if entries.is_empty() {
            return;
        }

        let current = self.conflict_resolver.nav_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_prev_target(&entries, current) else {
            return;
        };

        if use_marker_nav {
            if let Some(marker) = self
                .conflict_resolver
                .resolved_output_conflict_markers
                .get(target)
                .copied()
                .flatten()
            {
                let conflict_ix = marker.conflict_ix;
                self.conflict_resolver.active_conflict = conflict_ix;
                self.conflict_resolver_scroll_all_views_to_conflict(conflict_ix, None, None, cx);
            } else {
                self.conflict_resolver_scroll_resolved_output_to_line(
                    target,
                    self.conflict_resolved_preview_line_count.max(1),
                );
            }
        } else {
            let conflict_ix = match self.conflict_resolver.view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    self.conflict_resolver_range_ix_for_visible(target)
                }
                ConflictResolverViewMode::TwoWayDiff => {
                    self.conflict_resolver_two_way_conflict_ix_for_visible(target)
                }
            };

            if let Some(conflict_ix) = conflict_ix {
                self.conflict_resolver.active_conflict = conflict_ix;
                self.conflict_resolver_scroll_all_views_to_conflict(
                    conflict_ix,
                    Some(target),
                    None,
                    cx,
                );
            } else {
                // Fallback: keep input pane navigation even if conflict mapping is unavailable.
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
            }
        }
        self.conflict_resolver.nav_anchor = Some(target);
    }

    pub(in crate::view) fn conflict_jump_next(&mut self, cx: &mut gpui::Context<Self>) {
        let marker_entries = self.conflict_marker_nav_entries();
        let use_marker_nav = !marker_entries.is_empty();
        let entries = if use_marker_nav {
            marker_entries
        } else {
            self.conflict_fallback_nav_entries()
        };
        if entries.is_empty() {
            return;
        }

        let current = self.conflict_resolver.nav_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_next_target(&entries, current) else {
            return;
        };

        if use_marker_nav {
            if let Some(marker) = self
                .conflict_resolver
                .resolved_output_conflict_markers
                .get(target)
                .copied()
                .flatten()
            {
                let conflict_ix = marker.conflict_ix;
                self.conflict_resolver.active_conflict = conflict_ix;
                self.conflict_resolver_scroll_all_views_to_conflict(conflict_ix, None, None, cx);
            } else {
                self.conflict_resolver_scroll_resolved_output_to_line(
                    target,
                    self.conflict_resolved_preview_line_count.max(1),
                );
            }
        } else {
            let conflict_ix = match self.conflict_resolver.view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    self.conflict_resolver_range_ix_for_visible(target)
                }
                ConflictResolverViewMode::TwoWayDiff => {
                    self.conflict_resolver_two_way_conflict_ix_for_visible(target)
                }
            };

            if let Some(conflict_ix) = conflict_ix {
                self.conflict_resolver.active_conflict = conflict_ix;
                self.conflict_resolver_scroll_all_views_to_conflict(
                    conflict_ix,
                    Some(target),
                    None,
                    cx,
                );
            } else {
                // Fallback: keep input pane navigation even if conflict mapping is unavailable.
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
            }
        }
        self.conflict_resolver.nav_anchor = Some(target);
    }

    /// Map a visible index back to the conflict range index it belongs to.
    pub(super) fn conflict_resolver_range_ix_for_visible(&self, vi: usize) -> Option<usize> {
        let item = self.conflict_resolver.three_way_visible_map.get(vi)?;
        match item {
            conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(ri) => Some(*ri),
            conflict_resolver::ThreeWayVisibleItem::Line(line_ix) => self
                .conflict_resolver
                .three_way_line_conflict_map
                .ours
                .get(*line_ix)
                .copied()
                .flatten(),
        }
    }

    pub(super) fn conflict_resolver_two_way_conflict_ix_for_visible(
        &self,
        visible_ix: usize,
    ) -> Option<usize> {
        match self.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => conflict_resolver::two_way_conflict_index_for_visible_row(
                &self.conflict_resolver.diff_row_conflict_map,
                &self.conflict_resolver.diff_visible_row_indices,
                visible_ix,
            ),
            ConflictDiffMode::Inline => conflict_resolver::two_way_conflict_index_for_visible_row(
                &self.conflict_resolver.inline_row_conflict_map,
                &self.conflict_resolver.inline_visible_row_indices,
                visible_ix,
            ),
        }
    }

    pub(super) fn conflict_resolver_two_way_visible_ix_for_conflict(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        match self.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => self
                .conflict_resolver
                .diff_row_conflict_map
                .iter()
                .position(|mapped| *mapped == Some(conflict_ix))
                .and_then(|row_ix| {
                    self.conflict_resolver
                        .diff_visible_row_indices
                        .binary_search(&row_ix)
                        .ok()
                }),
            ConflictDiffMode::Inline => self
                .conflict_resolver
                .inline_row_conflict_map
                .iter()
                .position(|mapped| *mapped == Some(conflict_ix))
                .and_then(|row_ix| {
                    self.conflict_resolver
                        .inline_visible_row_indices
                        .binary_search(&row_ix)
                        .ok()
                }),
        }
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

    pub(in crate::view) fn diff_jump_prev(&mut self) {
        let entries = self.diff_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.diff_selection_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_prev_target(&entries, current) else {
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in crate::view) fn diff_jump_next(&mut self) {
        let entries = self.diff_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.diff_selection_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_next_target(&entries, current) else {
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in crate::view) fn maybe_autoscroll_diff_to_first_change(&mut self) {
        if !self.diff_autoscroll_pending {
            return;
        }
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_autoscroll_pending = false;
            return;
        }
        if self.diff_visible_len() == 0 {
            return;
        }

        let entries = self.diff_nav_entries();
        let target = entries.first().copied().unwrap_or(0);

        self.scroll_diff_to_item(target, gpui::ScrollStrategy::Top);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
        self.diff_autoscroll_pending = false;
    }

    pub(super) fn sync_conflict_resolver(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };

        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };

        let Some(DiffTarget::WorkingTree { path, area }) = repo.diff_state.diff_target.as_ref()
        else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };
        if *area != DiffArea::Unstaged {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        }

        let conflict_entry = match &repo.status {
            Loadable::Ready(status) => status.unstaged.iter().find(|e| {
                e.path == *path && e.kind == gitcomet_core::domain::FileStatusKind::Conflicted
            }),
            _ => None,
        };
        let Some(conflict_entry) = conflict_entry else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };
        let conflict_kind = conflict_entry.conflict;

        let path = path.clone();

        let should_load = repo.conflict_state.conflict_file_path.as_ref() != Some(&path)
            && !matches!(repo.conflict_state.conflict_file, Loadable::Loading);
        if should_load {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            let theme = self.theme;
            self.conflict_resolver_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text("", cx);
            });
            self.store.dispatch(Msg::LoadConflictFile { repo_id, path });
            return;
        }

        let Loadable::Ready(Some(file)) = &repo.conflict_state.conflict_file else {
            return;
        };
        if file.path != path {
            return;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        file.base.hash(&mut hasher);
        file.ours.hash(&mut hasher);
        file.theirs.hash(&mut hasher);
        file.current.hash(&mut hasher);
        let source_hash = hasher.finish();

        let needs_rebuild = self.conflict_resolver.repo_id != Some(repo_id)
            || self.conflict_resolver.path.as_ref() != Some(&path)
            || self.conflict_resolver.source_hash != Some(source_hash);

        // When the file content hasn't changed but state-side conflict data has
        // been updated (e.g. hide_resolved toggled externally, bulk picks, or
        // autosolve applied from state), do a lightweight re-sync that re-applies
        // session resolutions and rebuilds visible maps without recomputing the
        // expensive diff/highlight data.
        if !needs_rebuild {
            if self.conflict_resolver.conflict_rev != repo.conflict_state.conflict_rev {
                self.resync_conflict_resolver_from_state(cx);
            }
            return;
        }

        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_segments_cache_inline.clear();
        self.conflict_diff_query_segments_cache_split.clear();
        self.conflict_diff_query_segments_cache_inline.clear();
        self.conflict_diff_query_cache_query = SharedString::default();

        // Use the ConflictSession from state for strategy if available,
        // otherwise fall back to local computation.
        let (conflict_strategy, is_binary) = if let Some(session) =
            &repo.conflict_state.conflict_session
        {
            let binary =
                session.base.is_binary() || session.ours.is_binary() || session.theirs.is_binary();
            (Some(session.strategy), binary)
        } else {
            let has_non_text =
                |bytes: &Option<Vec<u8>>, text: &Option<String>| bytes.is_some() && text.is_none();
            let binary = has_non_text(&file.base_bytes, &file.base)
                || has_non_text(&file.ours_bytes, &file.ours)
                || has_non_text(&file.theirs_bytes, &file.theirs);
            (
                Self::conflict_resolver_strategy(conflict_kind, binary),
                binary,
            )
        };
        let conflict_syntax_language = rows::diff_syntax_language_for_path(&path);

        // For binary conflicts, populate minimal state and return early.
        if is_binary {
            let binary_side_sizes = [
                file.base_bytes.as_ref().map(|b| b.len()),
                file.ours_bytes.as_ref().map(|b| b.len()),
                file.theirs_bytes.as_ref().map(|b| b.len()),
            ];
            self.conflict_resolver = ConflictResolverUiState {
                repo_id: Some(repo_id),
                path: Some(path),
                conflict_syntax_language,
                source_hash: Some(source_hash),
                is_binary_conflict: true,
                binary_side_sizes,
                strategy: conflict_strategy,
                conflict_kind,
                last_autosolve_summary: None,
                conflict_rev: repo.conflict_state.conflict_rev,
                ..ConflictResolverUiState::default()
            };
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        }

        let fallback_resolved = if let Some(cur) = file.current.as_deref() {
            cur.to_string()
        } else if let Some(ours) = file.ours.as_deref() {
            ours.to_string()
        } else if let Some(theirs) = file.theirs.as_deref() {
            theirs.to_string()
        } else {
            String::new()
        };
        let mut marker_segments = if let Some(cur) = file.current.as_deref() {
            let segments = conflict_resolver::parse_conflict_markers(cur);
            if conflict_resolver::conflict_count(&segments) > 0 {
                segments
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let ours_text = file.ours.as_deref().unwrap_or("");
        let theirs_text = file.theirs.as_deref().unwrap_or("");
        let base_text = file.base.as_deref().unwrap_or("");

        // When conflict markers are 2-way (no base section), populate block.base
        // from the git ancestor file so "A (base)" picks work.
        if !base_text.is_empty() {
            conflict_resolver::populate_block_bases_from_ancestor(&mut marker_segments, base_text);
        }
        let mut conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(&marker_segments);
        if let Some(session) = &repo.conflict_state.conflict_session {
            let applied = conflict_resolver::apply_session_region_resolutions_with_index_map(
                &mut marker_segments,
                &session.regions,
            );
            conflict_region_indices = applied.block_region_indices;
        }

        let resolved = if marker_segments.is_empty() {
            fallback_resolved
        } else {
            conflict_resolver::generate_resolved_text(&marker_segments)
        };

        let diff_rows = gitcomet_core::file_diff::side_by_side_rows(ours_text, theirs_text);
        let inline_rows = conflict_resolver::build_inline_rows(&diff_rows);
        let (diff_row_conflict_map, inline_row_conflict_map) =
            conflict_resolver::map_two_way_rows_to_conflicts(
                &marker_segments,
                &diff_rows,
                &inline_rows,
            );

        let three_way_text = ThreeWaySides {
            base: SharedString::from(base_text.to_string()),
            ours: SharedString::from(ours_text.to_string()),
            theirs: SharedString::from(theirs_text.to_string()),
        };
        let three_way_line_starts = ThreeWaySides {
            base: build_line_starts(base_text),
            ours: build_line_starts(ours_text),
            theirs: build_line_starts(theirs_text),
        };
        let side_line_count = |text: &str| {
            if text.is_empty() {
                0
            } else {
                count_newlines(text).saturating_add(1)
            }
        };
        let three_way_base_len = side_line_count(base_text);
        let three_way_ours_len = side_line_count(ours_text);
        let three_way_theirs_len = side_line_count(theirs_text);
        let three_way_len = three_way_base_len
            .max(three_way_ours_len)
            .max(three_way_theirs_len);

        let three_way_conflict_maps = conflict_resolver::build_three_way_conflict_maps(
            &marker_segments,
            three_way_base_len,
            three_way_ours_len,
            three_way_theirs_len,
        );

        let view_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.view_mode
        } else if matches!(
            conflict_strategy,
            Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver)
        ) && file.base.is_some()
        {
            ConflictResolverViewMode::ThreeWay
        } else {
            ConflictResolverViewMode::TwoWayDiff
        };

        let hide_resolved = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.hide_resolved
        } else {
            repo.conflict_state.conflict_hide_resolved
        };
        let diff_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.diff_mode
        } else {
            ConflictDiffMode::Split
        };
        let nav_anchor = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.nav_anchor
        } else {
            None
        };
        let active_conflict = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            let total = conflict_resolver::conflict_count(&marker_segments);
            if total == 0 {
                0
            } else {
                self.conflict_resolver.active_conflict.min(total - 1)
            }
        } else {
            0
        };
        let resolver_preview_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.resolver_preview_mode
        } else {
            ConflictResolverPreviewMode::default()
        };

        let (
            three_way_word_highlights_base,
            three_way_word_highlights_ours,
            three_way_word_highlights_theirs,
            diff_word_highlights_split,
        ) = {
            let (base, ours, theirs) = conflict_resolver::compute_three_way_word_highlights(
                &three_way_text.base,
                &three_way_line_starts.base,
                &three_way_text.ours,
                &three_way_line_starts.ours,
                &three_way_text.theirs,
                &three_way_line_starts.theirs,
                &marker_segments,
            );
            let split = conflict_resolver::compute_two_way_word_highlights(&diff_rows);
            (base, ours, theirs, split)
        };

        self.conflict_three_way_segments_cache.clear();

        let three_way_visible_map = conflict_resolver::build_three_way_visible_map(
            three_way_len,
            &three_way_conflict_maps.conflict_ranges,
            &marker_segments,
            hide_resolved,
        );
        let diff_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &diff_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );
        let inline_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &inline_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );

        self.conflict_resolver = ConflictResolverUiState {
            repo_id: Some(repo_id),
            path: Some(path),
            conflict_syntax_language,
            source_hash: Some(source_hash),
            current: file.current.clone(),
            marker_segments,
            conflict_region_indices,
            active_conflict,
            hovered_conflict: None,
            view_mode,
            diff_rows,
            inline_rows,
            three_way_text,
            three_way_line_starts,
            three_way_len,
            three_way_conflict_ranges: three_way_conflict_maps.conflict_ranges,
            three_way_line_conflict_map: ThreeWaySides {
                base: three_way_conflict_maps.base_line_conflict_map,
                ours: three_way_conflict_maps.ours_line_conflict_map,
                theirs: three_way_conflict_maps.theirs_line_conflict_map,
            },
            conflict_has_base: three_way_conflict_maps.conflict_has_base,
            three_way_word_highlights: ThreeWaySides {
                base: three_way_word_highlights_base,
                ours: three_way_word_highlights_ours,
                theirs: three_way_word_highlights_theirs,
            },
            diff_word_highlights_split,
            diff_mode,
            nav_anchor,
            hide_resolved,
            three_way_visible_map,
            diff_row_conflict_map,
            inline_row_conflict_map,
            diff_visible_row_indices,
            inline_visible_row_indices,
            is_binary_conflict: false,
            binary_side_sizes: [None; 3],
            strategy: conflict_strategy,
            conflict_kind,
            last_autosolve_summary: None,
            conflict_rev: repo.conflict_state.conflict_rev,
            resolver_pending_recompute_seq: 0,
            resolved_line_meta: Vec::new(),
            resolved_output_conflict_markers: Vec::new(),
            resolved_output_line_sources_index: HashSet::default(),
            markdown_preview: ConflictResolverMarkdownPreviewState::default(),
            resolver_preview_mode,
        };

        let line_ending = crate::kit::TextInput::detect_line_ending(&resolved);
        let theme = self.theme;
        let mut output_hasher = std::collections::hash_map::DefaultHasher::new();
        resolved.hash(&mut output_hasher);
        let output_hash = output_hasher.finish();
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_line_ending(line_ending);
            input.set_text(resolved, cx);
        });
        let output_path = self.conflict_resolver.path.clone();
        self.conflict_resolved_preview_path = output_path.clone();
        self.conflict_resolved_preview_source_hash = Some(output_hash);
        self.schedule_conflict_resolved_outline_recompute(output_path, output_hash, None, cx);

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
    }

    /// Lightweight re-sync when `conflict_rev` changed but file content is the
    /// same. Re-parses markers, re-applies session resolutions, reads
    /// `hide_resolved` from state, and rebuilds visible maps — without
    /// recomputing the expensive diff rows and word highlights.
    pub(super) fn resync_conflict_resolver_from_state(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return;
        };
        let Loadable::Ready(Some(file)) = &repo.conflict_state.conflict_file else {
            return;
        };

        // Re-parse marker segments from original current text.
        let mut marker_segments = if let Some(cur) = file.current.as_deref() {
            let segments = conflict_resolver::parse_conflict_markers(cur);
            if conflict_resolver::conflict_count(&segments) > 0 {
                segments
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let base_text = file.base.as_deref().unwrap_or("");

        // Re-populate bases from ancestor (needed for 2-way markers).
        if !base_text.is_empty() {
            conflict_resolver::populate_block_bases_from_ancestor(&mut marker_segments, base_text);
        }
        let mut conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(&marker_segments);

        // Re-apply session region resolutions from state.
        if let Some(session) = &repo.conflict_state.conflict_session {
            let applied = conflict_resolver::apply_session_region_resolutions_with_index_map(
                &mut marker_segments,
                &session.regions,
            );
            conflict_region_indices = applied.block_region_indices;
        }

        // Regenerate resolved text.
        let resolved = if marker_segments.is_empty() {
            if let Some(cur) = file.current.as_deref() {
                cur.to_string()
            } else if let Some(ours) = file.ours.as_deref() {
                ours.to_string()
            } else if let Some(theirs) = file.theirs.as_deref() {
                theirs.to_string()
            } else {
                String::new()
            }
        } else {
            conflict_resolver::generate_resolved_text(&marker_segments)
        };

        // Read hide_resolved from state (authoritative source).
        let hide_resolved = repo.conflict_state.conflict_hide_resolved;

        let three_way_conflict_maps = conflict_resolver::build_three_way_conflict_maps(
            &marker_segments,
            self.conflict_resolver
                .three_way_line_count(ThreeWayColumn::Base),
            self.conflict_resolver
                .three_way_line_count(ThreeWayColumn::Ours),
            self.conflict_resolver
                .three_way_line_count(ThreeWayColumn::Theirs),
        );

        // Recompute row→conflict maps using existing diff/inline rows.
        let (diff_row_conflict_map, inline_row_conflict_map) =
            conflict_resolver::map_two_way_rows_to_conflicts(
                &marker_segments,
                &self.conflict_resolver.diff_rows,
                &self.conflict_resolver.inline_rows,
            );

        // Rebuild visible maps.
        let three_way_visible_map = conflict_resolver::build_three_way_visible_map(
            self.conflict_resolver.three_way_len,
            &three_way_conflict_maps.conflict_ranges,
            &marker_segments,
            hide_resolved,
        );
        let diff_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &diff_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );
        let inline_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &inline_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );

        // Clamp active_conflict to new conflict count.
        let total = conflict_resolver::conflict_count(&marker_segments);
        let active_conflict = if total == 0 {
            0
        } else {
            self.conflict_resolver.active_conflict.min(total - 1)
        };

        let new_rev = repo.conflict_state.conflict_rev;

        // Update only the fields that change during a state re-sync.
        self.conflict_resolver.marker_segments = marker_segments;
        self.conflict_resolver.conflict_region_indices = conflict_region_indices;
        self.conflict_resolver.hide_resolved = hide_resolved;
        self.conflict_resolver.three_way_conflict_ranges = three_way_conflict_maps.conflict_ranges;
        self.conflict_resolver.three_way_line_conflict_map.base =
            three_way_conflict_maps.base_line_conflict_map;
        self.conflict_resolver.three_way_line_conflict_map.ours =
            three_way_conflict_maps.ours_line_conflict_map;
        self.conflict_resolver.three_way_line_conflict_map.theirs =
            three_way_conflict_maps.theirs_line_conflict_map;
        self.conflict_resolver.conflict_has_base = three_way_conflict_maps.conflict_has_base;
        self.conflict_resolver.three_way_visible_map = three_way_visible_map;
        self.conflict_resolver.diff_row_conflict_map = diff_row_conflict_map;
        self.conflict_resolver.inline_row_conflict_map = inline_row_conflict_map;
        self.conflict_resolver.diff_visible_row_indices = diff_visible_row_indices;
        self.conflict_resolver.inline_visible_row_indices = inline_visible_row_indices;
        self.conflict_resolver.active_conflict = active_conflict;
        self.conflict_resolver.conflict_syntax_language = self
            .conflict_resolver
            .path
            .as_ref()
            .and_then(rows::diff_syntax_language_for_path);
        if self
            .conflict_resolver
            .hovered_conflict
            .is_some_and(|(ix, _)| ix >= total)
        {
            self.conflict_resolver.hovered_conflict = None;
        }
        self.conflict_resolver.conflict_rev = new_rev;

        // Clear segment caches since marker_segments changed.
        self.clear_conflict_diff_style_caches();
        self.conflict_three_way_segments_cache.clear();

        // Update the resolved text input.
        let line_ending = crate::kit::TextInput::detect_line_ending(&resolved);
        let theme = self.theme;
        let mut output_hasher = std::collections::hash_map::DefaultHasher::new();
        resolved.hash(&mut output_hasher);
        let output_hash = output_hasher.finish();
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_line_ending(line_ending);
            input.set_text(resolved, cx);
        });
        let output_path = self.conflict_resolver.path.clone();
        self.conflict_resolved_preview_path = output_path.clone();
        self.conflict_resolved_preview_source_hash = Some(output_hash);
        self.schedule_conflict_resolved_outline_recompute(output_path, output_hash, None, cx);

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
    }

    pub(in crate::view) fn conflict_resolver_set_mode(
        &mut self,
        mode: ConflictDiffMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.diff_mode == mode {
            return;
        }
        self.conflict_resolver.diff_mode = mode;
        self.conflict_resolver.nav_anchor = None;
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolver_set_view_mode(
        &mut self,
        view_mode: ConflictResolverViewMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.view_mode == view_mode {
            return;
        }
        self.conflict_resolver.view_mode = view_mode;
        self.conflict_resolver.nav_anchor = None;
        self.conflict_resolver.hovered_conflict = None;
        let path = self.conflict_resolver.path.clone();
        self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolver_toggle_hide_resolved(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.hide_resolved = !self.conflict_resolver.hide_resolved;
        self.conflict_resolver_rebuild_visible_map();
        // If we just hid resolved conflicts, ensure active_conflict points to
        // an unresolved block so the user doesn't stare at a collapsed row.
        if self.conflict_resolver.hide_resolved
            && let Some(next) = conflict_resolver::next_unresolved_conflict_index(
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.active_conflict,
            )
        {
            self.conflict_resolver.active_conflict = next;
        }
        if let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.path.clone(),
        ) {
            self.store.dispatch(Msg::ConflictSetHideResolved {
                repo_id,
                path,
                hide_resolved: self.conflict_resolver.hide_resolved,
            });
        }
        cx.notify();
    }

    pub(super) fn conflict_resolver_rebuild_visible_map(&mut self) {
        let three_way_conflict_maps = conflict_resolver::build_three_way_conflict_maps(
            &self.conflict_resolver.marker_segments,
            self.conflict_resolver
                .three_way_line_count(ThreeWayColumn::Base),
            self.conflict_resolver
                .three_way_line_count(ThreeWayColumn::Ours),
            self.conflict_resolver
                .three_way_line_count(ThreeWayColumn::Theirs),
        );
        self.conflict_resolver.three_way_conflict_ranges = three_way_conflict_maps.conflict_ranges;
        self.conflict_resolver.three_way_line_conflict_map.base =
            three_way_conflict_maps.base_line_conflict_map;
        self.conflict_resolver.three_way_line_conflict_map.ours =
            three_way_conflict_maps.ours_line_conflict_map;
        self.conflict_resolver.three_way_line_conflict_map.theirs =
            three_way_conflict_maps.theirs_line_conflict_map;
        self.conflict_resolver.conflict_has_base = three_way_conflict_maps.conflict_has_base;
        self.conflict_resolver.three_way_visible_map =
            conflict_resolver::build_three_way_visible_map(
                self.conflict_resolver.three_way_len,
                &self.conflict_resolver.three_way_conflict_ranges,
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.hide_resolved,
            );
        let block_count = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter(|seg| matches!(seg, conflict_resolver::ConflictSegment::Block(_)))
            .count();
        if self
            .conflict_resolver
            .hovered_conflict
            .is_some_and(|(ix, _)| ix >= block_count)
        {
            self.conflict_resolver.hovered_conflict = None;
        }
        let (split_map, inline_map) = conflict_resolver::map_two_way_rows_to_conflicts(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.diff_rows,
            &self.conflict_resolver.inline_rows,
        );
        self.conflict_resolver.diff_row_conflict_map = split_map;
        self.conflict_resolver.inline_row_conflict_map = inline_map;
        self.conflict_resolver.diff_visible_row_indices =
            conflict_resolver::build_two_way_visible_indices(
                &self.conflict_resolver.diff_row_conflict_map,
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.hide_resolved,
            );
        self.conflict_resolver.inline_visible_row_indices =
            conflict_resolver::build_two_way_visible_indices(
                &self.conflict_resolver.inline_row_conflict_map,
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.hide_resolved,
            );
    }

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
            ResolverPickTarget::TwoWayInlineLine { row_ix } => {
                self.conflict_resolver_append_inline_line_to_output(row_ix, cx);
            }
            ResolverPickTarget::Chunk {
                conflict_ix,
                choice,
                output_line_ix,
            } => {
                let target_conflict_ix = if let Some(output_line_ix) = output_line_ix {
                    let current_output = self
                        .conflict_resolver_input
                        .read_with(cx, |i, _| i.text().to_string());
                    self.conflict_resolver_split_chunk_target_for_output_line(
                        conflict_ix,
                        output_line_ix,
                        &current_output,
                    )
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

    pub(super) fn conflict_resolver_split_chunk_target_for_output_line(
        &mut self,
        fallback_conflict_ix: usize,
        output_line_ix: usize,
        output_text: &str,
    ) -> usize {
        let Some(marker) = resolved_output_marker_for_line(
            &self.conflict_resolver.marker_segments,
            output_text,
            output_line_ix,
        ) else {
            return fallback_conflict_ix;
        };
        let target_conflict_ix = marker.conflict_ix;
        let marker_count_for_conflict =
            resolved_output_markers_for_text(&self.conflict_resolver.marker_segments, output_text)
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
        self.conflict_resolver_rebuild_visible_map();

        resolved_output_marker_for_line(
            &self.conflict_resolver.marker_segments,
            output_text,
            output_line_ix,
        )
        .map(|m| m.conflict_ix)
        .unwrap_or(target_conflict_ix)
    }

    pub(super) fn conflict_resolver_append_choice_for_chunk(
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
        self.conflict_resolver.active_conflict = inserted_conflict_ix;

        let next =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let target_output_line = output_line_range_for_conflict_block_in_text(
            &self.conflict_resolver.marker_segments,
            &next,
            inserted_conflict_ix,
        )
        .map(|range| range.start);
        self.conflict_resolver_set_output(next.clone(), cx);
        if let Some(target_line_ix) = target_output_line {
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(target_line_ix, &next);
        }
        self.conflict_resolver_rebuild_visible_map();
        cx.notify();
        true
    }

    pub(super) fn conflict_resolver_reset_choice_for_chunk(
        &mut self,
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut matching_indices = conflict_group_indices_for_choice(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            conflict_ix,
            choice,
        );
        if matching_indices.is_empty() {
            return;
        }
        matching_indices.sort_unstable();
        matching_indices.dedup();

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
        self.conflict_resolver.active_conflict = if total_conflicts == 0 {
            0
        } else {
            conflict_ix.min(total_conflicts.saturating_sub(1))
        };

        let next =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let target_output_line = if total_conflicts == 0 {
            None
        } else {
            output_line_range_for_conflict_block_in_text(
                &self.conflict_resolver.marker_segments,
                &next,
                self.conflict_resolver.active_conflict,
            )
            .map(|range| range.start)
        };
        self.conflict_resolver_set_output(next.clone(), cx);
        if let Some(target_line_ix) = target_output_line {
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(target_line_ix, &next);
        }
        self.conflict_resolver_rebuild_visible_map();
        let should_sync_region = self
            .conflict_resolver
            .conflict_region_indices
            .get(self.conflict_resolver.active_conflict)
            .copied()
            .is_some_and(|region_ix| {
                conflict_region_index_is_unique(
                    &self.conflict_resolver.conflict_region_indices,
                    region_ix,
                )
            });
        if should_sync_region {
            self.conflict_resolver_sync_session_resolutions_from_output(&next);
        }
        cx.notify();
    }

    /// Immediately append a single line from the two-way split view to resolved output.
    pub(in crate::view) fn conflict_resolver_append_split_line_to_output(
        &mut self,
        row_ix: usize,
        side: ConflictPickSide,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.conflict_resolver.diff_rows.get(row_ix) else {
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

    /// Immediately append a single line from the two-way inline view to resolved output.
    pub(in crate::view) fn conflict_resolver_append_inline_line_to_output(
        &mut self,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.conflict_resolver.inline_rows.get(ix) else {
            return;
        };
        if row.content.is_empty() {
            return;
        }
        let line_ix = row
            .new_line
            .or(row.old_line)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| n.checked_sub(1));
        let choice = match row.side {
            ConflictPickSide::Ours => conflict_resolver::ConflictChoice::Ours,
            ConflictPickSide::Theirs => conflict_resolver::ConflictChoice::Theirs,
        };
        if let Some(line_ix) = line_ix {
            self.conflict_resolver_output_replace_line(line_ix, choice, cx);
            return;
        }
        let line_to_append = row.content.clone();
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
        let line = match choice {
            conflict_resolver::ConflictChoice::Base => self
                .conflict_resolver
                .three_way_line_text(ThreeWayColumn::Base, line_ix),
            conflict_resolver::ConflictChoice::Ours => self
                .conflict_resolver
                .three_way_line_text(ThreeWayColumn::Ours, line_ix),
            conflict_resolver::ConflictChoice::Theirs => self
                .conflict_resolver
                .three_way_line_text(ThreeWayColumn::Theirs, line_ix),
            conflict_resolver::ConflictChoice::Both => {
                // Both is chunk-level only, not line-level.
                return;
            }
        };
        let Some(_) = line else {
            return;
        };
        self.conflict_resolver_output_replace_line(line_ix, choice, cx);
    }

    pub(in crate::view) fn conflict_resolver_set_output(
        &mut self,
        text: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let unchanged = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text() == text);
        let theme = self.theme;
        let next_text = text;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            if input.text() == next_text {
                return;
            }
            let current = input.text();
            let old = current.as_bytes();
            let new = next_text.as_bytes();
            let old_len = old.len();
            let new_len = new.len();

            let mut prefix = 0usize;
            let prefix_max = old_len.min(new_len);
            while prefix < prefix_max && old[prefix] == new[prefix] {
                prefix = prefix.saturating_add(1);
            }
            while prefix > 0
                && (!current.is_char_boundary(prefix) || !next_text.is_char_boundary(prefix))
            {
                prefix = prefix.saturating_sub(1);
            }

            let mut suffix = 0usize;
            while suffix < old_len.saturating_sub(prefix)
                && suffix < new_len.saturating_sub(prefix)
                && old[old_len.saturating_sub(1 + suffix)]
                    == new[new_len.saturating_sub(1 + suffix)]
            {
                suffix = suffix.saturating_add(1);
            }
            while suffix > 0
                && (!current.is_char_boundary(old_len.saturating_sub(suffix))
                    || !next_text.is_char_boundary(new_len.saturating_sub(suffix)))
            {
                suffix = suffix.saturating_sub(1);
            }

            let old_range = prefix..old_len.saturating_sub(suffix);
            let replacement = next_text
                .get(prefix..new_len.saturating_sub(suffix))
                .unwrap_or("");
            input.replace_utf8_range(old_range, replacement, cx);
        });
        if unchanged {
            // Choosing a chunk can flip resolved/unresolved state without changing output text.
            // Force marker/provenance refresh so conflict overlays disappear immediately.
            let path = self.conflict_resolver.path.clone();
            self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
            cx.notify();
        }
    }

    /// Delete the current text selection in the resolved output (used by Cut context action).
    pub(in crate::view) fn conflict_resolver_output_delete_selection(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            let selection = input.selected_range();
            if selection.is_empty() {
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
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            let pos = input.cursor_offset().min(input.text().len());
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
        let is_three_way = self.conflict_resolver.view_mode
            == conflict_resolver::ConflictResolverViewMode::ThreeWay;

        let replacement: Option<String> = if is_three_way {
            match choice {
                conflict_resolver::ConflictChoice::Base => self
                    .conflict_resolver
                    .three_way_line_text(ThreeWayColumn::Base, line_ix)
                    .map(ToString::to_string),
                conflict_resolver::ConflictChoice::Ours => self
                    .conflict_resolver
                    .three_way_line_text(ThreeWayColumn::Ours, line_ix)
                    .map(ToString::to_string),
                conflict_resolver::ConflictChoice::Theirs => self
                    .conflict_resolver
                    .three_way_line_text(ThreeWayColumn::Theirs, line_ix)
                    .map(ToString::to_string),
                conflict_resolver::ConflictChoice::Both => return,
            }
        } else {
            let target_line_no = u32::try_from(line_ix + 1).ok();
            match choice {
                conflict_resolver::ConflictChoice::Ours => self
                    .conflict_resolver
                    .diff_rows
                    .iter()
                    .find(|r| target_line_no.is_some_and(|no| r.old_line == Some(no)))
                    .and_then(|r| r.old.clone())
                    .or_else(|| {
                        self.conflict_resolver
                            .diff_rows
                            .get(line_ix)
                            .and_then(|r| r.old.clone())
                    }),
                conflict_resolver::ConflictChoice::Theirs => self
                    .conflict_resolver
                    .diff_rows
                    .iter()
                    .find(|r| target_line_no.is_some_and(|no| r.new_line == Some(no)))
                    .and_then(|r| r.new.clone())
                    .or_else(|| {
                        self.conflict_resolver
                            .diff_rows
                            .get(line_ix)
                            .and_then(|r| r.new.clone())
                    }),
                _ => return,
            }
        };
        let Some(replacement) = replacement else {
            return;
        };

        let theme = self.theme;
        let mut scroll_to_line = None;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            let content = input.text();
            if let Some(range) = line_content_byte_range_for_index(content, line_ix) {
                input.replace_utf8_range(range, &replacement, cx);
                scroll_to_line = Some(line_ix);
                return;
            }

            let append_line_ix = source_line_count(content);
            let insertion = append_line_insertion_text(content, &replacement);
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

    pub(in crate::view) fn conflict_resolver_sync_session_resolutions_from_output(
        &mut self,
        output_text: &str,
    ) {
        let Some(repo_id) = self
            .conflict_resolver
            .repo_id
            .or_else(|| self.active_repo_id())
        else {
            return;
        };
        let Some(path) = self.conflict_resolver.path.clone() else {
            return;
        };
        let Some(updates) = conflict_resolver::derive_region_resolution_updates_from_output(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            output_text,
        ) else {
            return;
        };
        if updates.is_empty() {
            return;
        }
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
        let segments = conflict_resolver::parse_conflict_markers(current);
        if conflict_resolver::conflict_count(&segments) == 0 {
            return;
        }
        self.conflict_resolver.marker_segments = segments;
        self.conflict_resolver.conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(
                &self.conflict_resolver.marker_segments,
            );
        self.conflict_resolver.active_conflict = 0;
        self.conflict_resolver.last_autosolve_summary = None;
        self.conflict_resolver_rebuild_visible_map();
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        self.conflict_resolver_set_output(resolved, cx);
        if let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.path.clone(),
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

    pub(super) fn conflict_resolver_session_counts(&self) -> Option<(usize, usize)> {
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

    pub(super) fn conflict_resolver_active_block_mut(
        &mut self,
    ) -> Option<&mut conflict_resolver::ConflictBlock> {
        let target = self.conflict_resolver.active_conflict;
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
        self.conflict_resolver.active_conflict = range_ix;
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

        let Some(block) = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                _ => None,
            })
            .nth(conflict_ix)
        else {
            return;
        };

        let Some(replacement_lines) = replacement_lines_for_conflict_block(block, choice) else {
            return;
        };
        let current_output = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let output_range = output_line_range_for_conflict_block_in_text(
            &self.conflict_resolver.marker_segments,
            &current_output,
            conflict_ix,
        );
        let Some(output_range) = output_range else {
            return;
        };

        self.conflict_resolver.active_conflict = conflict_ix;
        self.conflict_resolver.hovered_conflict = None;
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
        {
            let Some(active_block) = self.conflict_resolver_active_block_mut() else {
                return;
            };
            if matches!(choice, conflict_resolver::ConflictChoice::Base)
                && active_block.base.is_none()
            {
                return;
            }
            active_block.choice = choice;
            active_block.resolved = true;
        }
        if dispatch_region_choice
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.path.clone(),
            )
        {
            let region_choice = match choice {
                conflict_resolver::ConflictChoice::Base => {
                    gitcomet_state::msg::ConflictRegionChoice::Base
                }
                conflict_resolver::ConflictChoice::Ours => {
                    gitcomet_state::msg::ConflictRegionChoice::Ours
                }
                conflict_resolver::ConflictChoice::Theirs => {
                    gitcomet_state::msg::ConflictRegionChoice::Theirs
                }
                conflict_resolver::ConflictChoice::Both => {
                    gitcomet_state::msg::ConflictRegionChoice::Both
                }
            };
            self.store.dispatch(Msg::ConflictSetRegionChoice {
                repo_id,
                path,
                region_index: picked_region_index,
                choice: region_choice,
            });
        }

        let target_output_line = output_range.start;
        let next = replace_output_lines_in_range(&current_output, output_range, &replacement_lines);
        self.conflict_resolver_set_output(next.clone(), cx);
        self.conflict_resolver_scroll_resolved_output_to_line_in_text(target_output_line, &next);
        self.conflict_resolver_rebuild_visible_map();

        // Auto-advance to the next unresolved conflict (kdiff3-style).
        let current = self.conflict_resolver.active_conflict;
        if let Some(next_unresolved) = conflict_resolver::next_unresolved_conflict_index(
            &self.conflict_resolver.marker_segments,
            current,
        )
        .filter(|&next| next != current)
        {
            self.conflict_resolver.active_conflict = next_unresolved;
            let target_visible_ix = conflict_resolver::visible_index_for_conflict(
                &self.conflict_resolver.three_way_visible_map,
                &self.conflict_resolver.three_way_conflict_ranges,
                self.conflict_resolver.active_conflict,
            );
            if let Some(vi) = target_visible_ix {
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(vi, gpui::ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolver_pick_active_conflict(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver_conflict_count() == 0 {
            return;
        }
        let picked_conflict_index = self.conflict_resolver.active_conflict;
        let picked_region_index = self
            .conflict_resolver
            .conflict_region_indices
            .get(picked_conflict_index)
            .copied()
            .unwrap_or(picked_conflict_index);
        let dispatch_region_choice = conflict_region_index_is_unique(
            &self.conflict_resolver.conflict_region_indices,
            picked_region_index,
        );
        {
            let Some(block) = self.conflict_resolver_active_block_mut() else {
                return;
            };
            if matches!(choice, conflict_resolver::ConflictChoice::Base) && block.base.is_none() {
                return;
            }
            block.choice = choice;
            block.resolved = true;
        }
        if dispatch_region_choice
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.path.clone(),
            )
        {
            let region_choice = match choice {
                conflict_resolver::ConflictChoice::Base => {
                    gitcomet_state::msg::ConflictRegionChoice::Base
                }
                conflict_resolver::ConflictChoice::Ours => {
                    gitcomet_state::msg::ConflictRegionChoice::Ours
                }
                conflict_resolver::ConflictChoice::Theirs => {
                    gitcomet_state::msg::ConflictRegionChoice::Theirs
                }
                conflict_resolver::ConflictChoice::Both => {
                    gitcomet_state::msg::ConflictRegionChoice::Both
                }
            };
            self.store.dispatch(Msg::ConflictSetRegionChoice {
                repo_id,
                path,
                region_index: picked_region_index,
                choice: region_choice,
            });
        }
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let target_output_line = output_line_range_for_conflict_block_in_text(
            &self.conflict_resolver.marker_segments,
            &resolved,
            picked_conflict_index,
        )
        .map(|range| range.start);
        self.conflict_resolver_set_output(resolved.clone(), cx);
        if let Some(target_line_ix) = target_output_line {
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(
                target_line_ix,
                &resolved,
            );
        }
        self.conflict_resolver_rebuild_visible_map();

        // Auto-advance to the next unresolved conflict (kdiff3-style).
        let current = self.conflict_resolver.active_conflict;
        if let Some(next_unresolved) = conflict_resolver::next_unresolved_conflict_index(
            &self.conflict_resolver.marker_segments,
            current,
        )
        .filter(|&next| next != current)
        {
            self.conflict_resolver.active_conflict = next_unresolved;
            let target_visible_ix = match self.conflict_resolver.view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    conflict_resolver::visible_index_for_conflict(
                        &self.conflict_resolver.three_way_visible_map,
                        &self.conflict_resolver.three_way_conflict_ranges,
                        self.conflict_resolver.active_conflict,
                    )
                }
                ConflictResolverViewMode::TwoWayDiff => self
                    .conflict_resolver_two_way_visible_ix_for_conflict(
                        self.conflict_resolver.active_conflict,
                    ),
            };
            if let Some(vi) = target_visible_ix {
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(vi, gpui::ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolver_resolved_count(&self) -> usize {
        let (_, resolved) = conflict_resolver::effective_conflict_counts(
            &self.conflict_resolver.marker_segments,
            self.conflict_resolver_session_counts(),
        );
        resolved
    }

    pub(super) fn dispatch_conflict_autosolve_telemetry(
        &self,
        mode: gitcomet_state::msg::ConflictAutosolveMode,
        total_conflicts_before: usize,
        total_conflicts_after: usize,
        unresolved_before: usize,
        unresolved_after: usize,
        stats: gitcomet_state::msg::ConflictAutosolveStats,
    ) {
        let Some(repo_id) = self
            .conflict_resolver
            .repo_id
            .or_else(|| self.active_repo_id())
        else {
            return;
        };
        self.store.dispatch(Msg::RecordConflictAutosolveTelemetry {
            repo_id,
            path: self.conflict_resolver.path.clone(),
            mode,
            total_conflicts_before,
            total_conflicts_after,
            unresolved_before,
            unresolved_after,
            stats,
        });
    }

    /// Apply safe auto-resolve rules to all unresolved conflict blocks.
    /// Updates the resolved output text and notifies the UI.
    pub(in crate::view) fn conflict_resolver_auto_resolve(&mut self, cx: &mut gpui::Context<Self>) {
        let total_before = self.conflict_resolver_conflict_count();
        if total_before == 0 {
            return;
        }
        let unresolved_before =
            total_before.saturating_sub(self.conflict_resolver_resolved_count());
        // Pass 1: safe whole-block auto-resolve.
        let pass1 = conflict_resolver::auto_resolve_segments_with_options(
            &mut self.conflict_resolver.marker_segments,
            false,
        );
        // Pass 2: heuristic subchunk splitting — split remaining unresolved
        // blocks into finer line-level subchunks where possible.
        let pass2 = conflict_resolver::auto_resolve_segments_pass2_with_region_indices(
            &mut self.conflict_resolver.marker_segments,
            &mut self.conflict_resolver.conflict_region_indices,
        );
        let pass1_after_split = if pass2 > 0 {
            // Re-run Pass 1 on newly created sub-blocks (they may now
            // satisfy whole-block rules after splitting).
            conflict_resolver::auto_resolve_segments_with_options(
                &mut self.conflict_resolver.marker_segments,
                false,
            )
        } else {
            0
        };
        let count = pass1 + pass2 + pass1_after_split;
        if count > 0 {
            let resolved =
                conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
            self.conflict_resolver_set_output(resolved, cx);
            self.conflict_resolver_rebuild_visible_map();
            // Keep focus aligned with unresolved navigation after auto-resolve.
            if let Some(next_unresolved) = conflict_resolver::next_unresolved_conflict_index(
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.active_conflict,
            ) {
                self.conflict_resolver.active_conflict = next_unresolved;
            }
        }
        let total_after = self.conflict_resolver_conflict_count();
        let unresolved_after = total_after.saturating_sub(self.conflict_resolver_resolved_count());
        let stats = gitcomet_state::msg::ConflictAutosolveStats {
            pass1,
            pass2_split: pass2,
            pass1_after_split,
            regex: 0,
            history: 0,
        };
        self.conflict_resolver.last_autosolve_summary = Some(
            conflict_resolver::format_autosolve_trace_summary(
                conflict_resolver::AutosolveTraceMode::Safe,
                unresolved_before,
                unresolved_after,
                &stats,
            )
            .into(),
        );
        self.dispatch_conflict_autosolve_telemetry(
            gitcomet_state::msg::ConflictAutosolveMode::Safe,
            total_before,
            total_after,
            unresolved_before,
            unresolved_after,
            stats,
        );
        if count > 0
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.path.clone(),
            )
        {
            self.store.dispatch(Msg::ConflictApplyAutosolve {
                repo_id,
                path,
                mode: gitcomet_state::msg::ConflictAutosolveMode::Safe,
                whitespace_normalize: false,
            });
        }
        cx.notify();
    }
}
