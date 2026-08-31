use super::*;

pub(super) fn record_resolved_outline_trace(
    path: Option<&std::path::PathBuf>,
    started: Instant,
    pane: &MainPaneView,
    output_line_count: usize,
) {
    let path = path.cloned();
    let elapsed = started.elapsed();
    let (diff_row_count, inline_row_count) = pane.conflict_resolver.two_way_row_counts();
    mergetool_trace::record_with(|| {
        MergetoolTraceEvent::new(MergetoolTraceStage::ResolvedOutlineRecompute, path, elapsed)
            .with_base(MergetoolTraceSideStats::from_text(Some(
                pane.conflict_resolver.three_way_text.base.as_ref(),
            )))
            .with_ours(MergetoolTraceSideStats::from_text(Some(
                pane.conflict_resolver.three_way_text.ours.as_ref(),
            )))
            .with_theirs(MergetoolTraceSideStats::from_text(Some(
                pane.conflict_resolver.three_way_text.theirs.as_ref(),
            )))
            .with_conflict_block_count(Some(conflict_resolver::conflict_count(
                &pane.conflict_resolver.marker_segments,
            )))
            .with_diff_row_count(Some(diff_row_count))
            .with_inline_row_count(Some(inline_row_count))
            .with_resolved_output_line_count(Some(output_line_count))
    });
}

pub(super) struct ResolvedOutlineComputation {
    output_line_count: usize,
    outline: ResolvedOutlineData,
}

pub(super) enum ResolvedOutlineSourceView<'a> {
    ThreeWay {
        base_text: &'a str,
        base_line_starts: &'a [usize],
        ours_text: &'a str,
        ours_line_starts: &'a [usize],
        theirs_text: &'a str,
        theirs_line_starts: &'a [usize],
    },
    TwoWay {
        ours_text: &'a str,
        ours_line_starts: &'a [usize],
        theirs_text: &'a str,
        theirs_line_starts: &'a [usize],
    },
}

impl ResolvedOutlineSourceView<'_> {
    pub(super) fn view_mode(&self) -> ConflictResolverViewMode {
        match self {
            Self::ThreeWay { .. } => ConflictResolverViewMode::ThreeWay,
            Self::TwoWay { .. } => ConflictResolverViewMode::TwoWayDiff,
        }
    }
}

#[derive(Clone)]
pub(super) enum OwnedResolvedOutlineSourceData {
    ThreeWay {
        base_text: Arc<str>,
        base_line_starts: Arc<[usize]>,
        ours_text: Arc<str>,
        ours_line_starts: Arc<[usize]>,
        theirs_text: Arc<str>,
        theirs_line_starts: Arc<[usize]>,
    },
    TwoWay {
        ours_text: Arc<str>,
        ours_line_starts: Arc<[usize]>,
        theirs_text: Arc<str>,
        theirs_line_starts: Arc<[usize]>,
    },
}

impl OwnedResolvedOutlineSourceData {
    pub(super) fn as_view(&self) -> ResolvedOutlineSourceView<'_> {
        match self {
            Self::ThreeWay {
                base_text,
                base_line_starts,
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            } => ResolvedOutlineSourceView::ThreeWay {
                base_text,
                base_line_starts,
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            },
            Self::TwoWay {
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            } => ResolvedOutlineSourceView::TwoWay {
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            },
        }
    }
}

#[derive(Clone)]
pub(super) struct BackgroundResolvedOutlineRecomputeRequest {
    output_text: Arc<str>,
    output_line_count: usize,
    marker_segments: Vec<conflict_resolver::ConflictSegment>,
    block_map: conflict_resolver::ResolvedOutputBlockMap,
    sources: OwnedResolvedOutlineSourceData,
}

pub(super) struct ResolvedOutlineIncrementalBase<'a> {
    text: &'a TextModelSnapshot,
    line_starts: &'a Arc<[usize]>,
    marker_segments: &'a [conflict_resolver::ConflictSegment],
    view_mode: ConflictResolverViewMode,
}

pub(super) fn compute_resolved_outline_computation(
    output_text: &str,
    output_line_count: usize,
    marker_segments: &[conflict_resolver::ConflictSegment],
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
    sources: ResolvedOutlineSourceView<'_>,
) -> ResolvedOutlineComputation {
    let view_mode = sources.view_mode();
    let markers = build_resolved_output_conflict_markers(
        marker_segments,
        output_text,
        output_line_count,
        block_map,
    );
    if should_skip_resolved_outline_provenance(view_mode, output_line_count) {
        return ResolvedOutlineComputation {
            output_line_count,
            outline: ResolvedOutlineData {
                meta: Vec::new(),
                markers,
                sources_index: FxHashSet::default(),
            },
        };
    }

    let mut meta = match sources {
        ResolvedOutlineSourceView::ThreeWay {
            base_text,
            base_line_starts,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => conflict_resolver::compute_resolved_line_provenance_from_text_with_indexed_sources(
            output_text,
            base_text,
            base_line_starts,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        ),
        ResolvedOutlineSourceView::TwoWay {
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => conflict_resolver::compute_resolved_line_provenance_from_text_two_way_indexed_sources(
            output_text,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        ),
    };
    apply_conflict_choice_provenance_hints(&mut meta, marker_segments, output_text, view_mode);
    let sources_index = conflict_resolver::build_resolved_output_line_sources_index_from_text(
        &meta,
        output_text,
        view_mode,
    );

    ResolvedOutlineComputation {
        output_line_count,
        outline: ResolvedOutlineData {
            meta,
            markers,
            sources_index,
        },
    }
}

pub(super) fn compute_resolved_outline_computation_from_projection(
    projection: &conflict_resolver::ResolvedOutputProjection,
    marker_segments: &[conflict_resolver::ConflictSegment],
    view_mode: ConflictResolverViewMode,
    sources: Option<ResolvedOutlineSourceView<'_>>,
) -> ResolvedOutlineComputation {
    let output_line_count = projection.len();
    let block_ranges = projection.conflict_line_ranges();
    let markers = build_resolved_output_conflict_markers_from_block_ranges(
        marker_segments,
        block_ranges,
        output_line_count,
    );
    if should_skip_resolved_outline_provenance(view_mode, output_line_count) {
        return ResolvedOutlineComputation {
            output_line_count,
            outline: ResolvedOutlineData {
                meta: Vec::new(),
                markers,
                sources_index: FxHashSet::default(),
            },
        };
    }

    let Some(sources) = sources else {
        return ResolvedOutlineComputation {
            output_line_count,
            outline: ResolvedOutlineData {
                meta: Vec::new(),
                markers,
                sources_index: FxHashSet::default(),
            },
        };
    };
    let mut source_lookup: FxHashMap<&str, (conflict_resolver::ResolvedLineSource, Option<u32>)> =
        FxHashMap::default();
    match sources {
        ResolvedOutlineSourceView::ThreeWay {
            base_text,
            base_line_starts,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => {
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::C,
                theirs_text,
                theirs_line_starts,
            );
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::B,
                ours_text,
                ours_line_starts,
            );
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::A,
                base_text,
                base_line_starts,
            );
        }
        ResolvedOutlineSourceView::TwoWay {
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => {
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::B,
                theirs_text,
                theirs_line_starts,
            );
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::A,
                ours_text,
                ours_line_starts,
            );
        }
    }

    let mut meta = Vec::with_capacity(output_line_count);
    for line_ix in 0..output_line_count {
        let line = projection
            .line_text(marker_segments, line_ix)
            .unwrap_or(std::borrow::Cow::Borrowed(""));
        let (source, input_line) = source_lookup
            .get(line.as_ref())
            .copied()
            .unwrap_or((conflict_resolver::ResolvedLineSource::Manual, None));
        meta.push(conflict_resolver::ResolvedLineMeta {
            output_line: u32::try_from(line_ix).unwrap_or(u32::MAX),
            source,
            input_line,
        });
    }
    apply_conflict_choice_provenance_hints_for_ranges(
        &mut meta,
        marker_segments,
        block_ranges,
        view_mode,
    );

    let mut sources_index = FxHashSet::default();
    sources_index.reserve(meta.len());
    for (line_ix, line_meta) in meta.iter().enumerate() {
        if line_meta.source == conflict_resolver::ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = line_meta.input_line else {
            continue;
        };
        let Some(line) = projection.line_text(marker_segments, line_ix) else {
            continue;
        };
        sources_index.insert(conflict_resolver::SourceLineKey::new(
            view_mode,
            line_meta.source,
            line_no,
            line.as_ref(),
        ));
    }

    ResolvedOutlineComputation {
        output_line_count,
        outline: ResolvedOutlineData {
            meta,
            markers,
            sources_index,
        },
    }
}

pub(super) fn insert_lookup_from_indexed_text<'a>(
    lookup: &mut FxHashMap<&'a str, (conflict_resolver::ResolvedLineSource, Option<u32>)>,
    source: conflict_resolver::ResolvedLineSource,
    text: &'a str,
    line_starts: &[usize],
) {
    let line_count = indexed_line_count(text, line_starts);
    for line_ix in (0..line_count).rev() {
        let line = rows::resolved_output_line_text(text, line_starts, line_ix);
        lookup.insert(
            line,
            (
                source,
                Some(u32::try_from(line_ix.saturating_add(1)).unwrap_or(u32::MAX)),
            ),
        );
    }
}

pub(super) fn update_line_sources_index_for_range(
    index: &mut FxHashSet<conflict_resolver::SourceLineKey>,
    view_mode: ConflictResolverViewMode,
    meta: &[conflict_resolver::ResolvedLineMeta],
    text: &str,
    line_starts: &[usize],
    line_range: Range<usize>,
    insert: bool,
) {
    if line_range.start >= line_range.end {
        return;
    }
    for line_ix in line_range {
        let Some(line_meta) = meta.get(line_ix) else {
            break;
        };
        if line_meta.source == conflict_resolver::ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = line_meta.input_line else {
            continue;
        };
        let key = conflict_resolver::SourceLineKey::new(
            view_mode,
            line_meta.source,
            line_no,
            rows::resolved_output_line_text(text, line_starts, line_ix),
        );
        if insert {
            index.insert(key);
        } else {
            index.remove(&key);
        }
    }
}

/// The row the resolved-output column measures its width against.
///
/// O(1): the rope carries the widest row in its summary, so the measurement
/// never scans the document. Ties keep the earliest row, matching the linear
/// scan this replaced.
pub(super) fn resolved_output_measure_row(snapshot: &TextModelSnapshot) -> usize {
    snapshot.rope().longest_row() as usize
}

impl MainPaneView {
    pub(super) fn resolved_outline_source_view(&self) -> ResolvedOutlineSourceView<'_> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => ResolvedOutlineSourceView::ThreeWay {
                base_text: &self.conflict_resolver.three_way_text.base,
                base_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Base),
                ours_text: &self.conflict_resolver.three_way_text.ours,
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Ours),
                theirs_text: &self.conflict_resolver.three_way_text.theirs,
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Theirs),
            },
            ConflictResolverViewMode::TwoWayDiff => ResolvedOutlineSourceView::TwoWay {
                ours_text: &self.conflict_resolver.three_way_text.ours,
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Ours),
                theirs_text: &self.conflict_resolver.three_way_text.theirs,
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Theirs),
            },
        }
    }

    /// Snapshot everything the outline recompute needs, so it can run detached.
    ///
    /// This materializes the output, and unlike the syntax path it is not an
    /// artifact worth removing: the outline assigns a
    /// provenance to *every* row by comparing its text against the three source
    /// sides, so the work is O(document) whatever it reads through, and the copy
    /// is a small constant beside it.
    ///
    /// What keeps that off the keystroke path is *where* it is called from, not
    /// its cost: both the production task and the synchronous test arm build the
    /// request only once the debounce
    /// (`CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS`) has settled and the recompute
    /// is going to run. Hoisting this call above that check charges every
    /// keystroke for a copy of the document that is then discarded.
    pub(super) fn background_resolved_outline_recompute_request(
        &self,
        output_snapshot: &TextModelSnapshot,
    ) -> BackgroundResolvedOutlineRecomputeRequest {
        let output_text: Arc<str> = output_snapshot.as_shared_string().into();
        let output_line_count = output_snapshot.shared_line_starts().len().max(1);
        let sources = match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => OwnedResolvedOutlineSourceData::ThreeWay {
                base_text: self.conflict_resolver.three_way_text.base.clone().into(),
                base_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Base),
                ours_text: self.conflict_resolver.three_way_text.ours.clone().into(),
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Ours),
                theirs_text: self.conflict_resolver.three_way_text.theirs.clone().into(),
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Theirs),
            },
            ConflictResolverViewMode::TwoWayDiff => OwnedResolvedOutlineSourceData::TwoWay {
                ours_text: self.conflict_resolver.three_way_text.ours.clone().into(),
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Ours),
                theirs_text: self.conflict_resolver.three_way_text.theirs.clone().into(),
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Theirs),
            },
        };

        BackgroundResolvedOutlineRecomputeRequest {
            output_text,
            output_line_count,
            marker_segments: self.conflict_resolver.marker_segments.clone(),
            block_map: self.conflict_resolved_output_block_map.clone(),
            sources,
        }
    }

    pub(super) fn stash_current_conflict_resolved_outline_state(&mut self) {
        let line_count = self.conflict_resolved_preview_line_count;
        if line_count == 0
            || self.conflict_resolver.resolved_outline.meta.len() != line_count
            || self.conflict_resolver.resolved_outline.markers.len() != line_count
        {
            return;
        }

        self.conflict_resolved_outline_stash = Some(StashedResolvedOutlineState {
            text: self.conflict_resolved_preview_text.clone(),
            line_starts: self.conflict_resolved_preview_line_starts.clone(),
            marker_segments: self.conflict_resolver.marker_segments.clone(),
            view_mode: self.conflict_resolver.view_mode,
            outline: self.conflict_resolver.resolved_outline.clone(),
        });
    }

    pub(super) fn resolved_outline_incremental_base(
        &self,
    ) -> Option<ResolvedOutlineIncrementalBase<'_>> {
        if self.conflict_resolved_output_is_streamed() {
            return None;
        }
        if let Some(stash) = self.conflict_resolved_outline_stash.as_ref() {
            return Some(ResolvedOutlineIncrementalBase {
                text: &stash.text,
                line_starts: &stash.line_starts,
                marker_segments: &stash.marker_segments,
                view_mode: stash.view_mode,
            });
        }

        let line_count = self.conflict_resolved_preview_line_count;
        if line_count == 0
            || self.conflict_resolver.resolved_outline.meta.len() != line_count
            || self.conflict_resolver.resolved_outline.markers.len() != line_count
        {
            return None;
        }

        Some(ResolvedOutlineIncrementalBase {
            text: &self.conflict_resolved_preview_text,
            line_starts: &self.conflict_resolved_preview_line_starts,
            marker_segments: &self.conflict_resolver.marker_segments,
            view_mode: self.conflict_resolver.view_mode,
        })
    }

    pub(super) fn sync_conflict_resolved_preview_snapshot(
        &mut self,
        output_snapshot: &TextModelSnapshot,
        path: Option<&std::path::PathBuf>,
        clear_outline: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if clear_outline {
            self.stash_current_conflict_resolved_outline_state();
        }
        self.conflict_resolved_preview_source_revision =
            Some(ResolvedOutputSourceRevision::from_snapshot(output_snapshot));
        self.conflict_resolved_preview_line_starts = output_snapshot.shared_line_starts();
        self.conflict_resolved_preview_syntax_language =
            path.and_then(rows::diff_syntax_language_for_path);
        self.conflict_resolved_preview_line_count = output_snapshot.line_count().max(1);
        self.conflict_resolved_output_measure_row = resolved_output_measure_row(output_snapshot);
        // Syntax no longer *waits* on this debounce — it tracks the buffer on
        // the keystroke, in the `cx.observe` on `conflict_resolver_input`. The
        // call stays because this method is also how the language arrives
        // (from `path`) and how a wholesale text replacement lands, neither of
        // which produces edit deltas. It reparses only if the buffer actually
        // differs from what the tree already describes, so on the common path
        // it is a version bump and nothing more.
        self.refresh_conflict_resolved_output_syntax(output_snapshot, None, cx);
        self.conflict_resolved_preview_text = output_snapshot.clone();

        if clear_outline {
            self.conflict_resolver.resolved_outline = ResolvedOutlineData::default();
            self.conflict_resolver.resolved_output_visible_dirty = true;
            self.conflict_resolver.resolved_outline_gutter_rows.clear();
        }
    }

    pub(super) fn apply_resolved_outline_computation(
        &mut self,
        path: Option<&std::path::PathBuf>,
        trace_started: Instant,
        computed: ResolvedOutlineComputation,
    ) {
        self.conflict_resolved_outline_stash = None;
        self.conflict_resolver.resolved_outline = computed.outline;
        self.conflict_resolver.resolved_output_visible_dirty = true;
        self.conflict_resolver.resolved_outline_gutter_rows.clear();
        record_resolved_outline_trace(path, trace_started, self, computed.output_line_count);
    }

    pub(in crate::view) fn recompute_conflict_resolved_outline_and_provenance(
        &mut self,
        path: Option<&std::path::PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_is_streamed() {
            let _ = cx;
            self.refresh_streamed_resolved_output_preview_from_markers(path);
            return;
        }
        let _perf_scope = perf::span(ViewPerfSpan::RecomputeResolvedOutline);
        let trace_started = Instant::now();
        let output_snapshot = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot());
        let output_text = output_snapshot.as_ref();
        let output_line_count = output_snapshot.shared_line_starts().len().max(1);
        let computed = compute_resolved_outline_computation(
            output_text,
            output_line_count,
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolved_output_block_map,
            self.resolved_outline_source_view(),
        );
        self.sync_conflict_resolved_preview_snapshot(&output_snapshot, path, false, cx);
        self.apply_resolved_outline_computation(path, trace_started, computed);
    }

    pub(super) fn recompute_conflict_resolved_outline_and_provenance_incremental(
        &mut self,
        path: Option<&std::path::PathBuf>,
        delta: ResolvedOutlineDelta,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.conflict_resolved_output_is_streamed() {
            let _ = path;
            let _ = delta;
            let _ = cx;
            return false;
        }
        let Some(base) = self.resolved_outline_incremental_base() else {
            return false;
        };
        let old_text_snapshot = base.text.clone();
        let old_text = old_text_snapshot.as_ref();
        let output_snapshot = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot());
        let output_text = output_snapshot.as_ref();
        let old_line_starts = base.line_starts.clone();
        let old_line_count = old_line_starts.len().max(1);
        let new_line_starts = output_snapshot.shared_line_starts();
        let new_line_count = new_line_starts.len().max(1);
        if old_line_starts.is_empty() {
            return false;
        }
        let used_stash = self.conflict_resolved_outline_stash.is_some();
        let delta = if used_stash {
            resolved_outline_delta_between_texts(old_text, output_text)
        } else {
            Some(delta)
        };
        let Some(delta) = delta else {
            return false;
        };
        if delta.old_range.start > delta.old_range.end
            || delta.new_range.start > delta.new_range.end
            || delta.old_range.end > old_text.len()
            || delta.new_range.end > output_text.len()
        {
            return false;
        }

        let old_dirty_lines = dirty_byte_range_to_line_range(
            old_line_starts.as_ref(),
            old_text.len(),
            delta.old_range.clone(),
        );
        let new_dirty_lines = dirty_byte_range_to_line_range(
            new_line_starts.as_ref(),
            output_text.len(),
            delta.new_range.clone(),
        );
        let mut old_affected = old_dirty_lines.clone();
        let mut new_affected = new_dirty_lines.clone();
        old_affected.start = old_affected.start.saturating_sub(1);
        old_affected.end = old_affected.end.saturating_add(1).min(old_line_count);
        new_affected.start = new_affected.start.saturating_sub(1);
        new_affected.end = new_affected.end.saturating_add(1).min(new_line_count);

        let Some(old_block_ranges) =
            resolved_output_conflict_block_ranges_in_text(base.marker_segments, old_text)
        else {
            return false;
        };
        let new_block_ranges = match resolved_output_conflict_block_line_ranges(
            &self.conflict_resolver.marker_segments,
            output_text,
            &self.conflict_resolved_output_block_map,
        ) {
            Some(ranges) if ranges.len() == old_block_ranges.len() => ranges,
            _ => remap_resolved_output_conflict_block_ranges_for_delta(
                old_block_ranges.as_slice(),
                old_dirty_lines.clone(),
                new_dirty_lines.clone(),
                new_line_count,
            ),
        };
        if old_block_ranges.len() != new_block_ranges.len() {
            return false;
        }

        let mut touched_conflicts: FxHashSet<usize> = FxHashSet::default();
        for (conflict_ix, range) in old_block_ranges.iter().enumerate() {
            if line_ranges_intersect(range, &old_affected) {
                touched_conflicts.insert(conflict_ix);
            }
        }
        for (conflict_ix, range) in new_block_ranges.iter().enumerate() {
            if line_ranges_intersect(range, &new_affected) {
                touched_conflicts.insert(conflict_ix);
            }
        }
        for conflict_ix in &touched_conflicts {
            if let Some(old_range) = old_block_ranges.get(*conflict_ix) {
                old_affected.start = old_affected.start.min(old_range.start);
                old_affected.end = old_affected.end.max(old_range.end).min(old_line_count);
            }
            if let Some(new_range) = new_block_ranges.get(*conflict_ix) {
                new_affected.start = new_affected.start.min(new_range.start);
                new_affected.end = new_affected.end.max(new_range.end).min(new_line_count);
            }
        }

        let mut recompute_conflicts = Vec::new();
        for (conflict_ix, new_range) in new_block_ranges.iter().enumerate() {
            if line_ranges_intersect(new_range, &new_affected) {
                recompute_conflicts.push(conflict_ix);
                if let Some(old_range) = old_block_ranges.get(conflict_ix) {
                    old_affected.start = old_affected.start.min(old_range.start);
                    old_affected.end = old_affected.end.max(old_range.end).min(old_line_count);
                }
                new_affected.start = new_affected.start.min(new_range.start);
                new_affected.end = new_affected.end.max(new_range.end).min(new_line_count);
            }
        }
        if old_affected.start != new_affected.start {
            return false;
        }

        let old_view_mode = base.view_mode;
        let new_view_mode = self.conflict_resolver.view_mode;
        let middle_meta = {
            let mut source_lookup: FxHashMap<
                &str,
                (conflict_resolver::ResolvedLineSource, Option<u32>),
            > = FxHashMap::default();
            match new_view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::C,
                        &self.conflict_resolver.three_way_text.theirs,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Theirs),
                    );
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::B,
                        &self.conflict_resolver.three_way_text.ours,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Ours),
                    );
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::A,
                        &self.conflict_resolver.three_way_text.base,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Base),
                    );
                }
                ConflictResolverViewMode::TwoWayDiff => {
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::B,
                        &self.conflict_resolver.three_way_text.theirs,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Theirs),
                    );
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::A,
                        &self.conflict_resolver.three_way_text.ours,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Ours),
                    );
                }
            }

            let mut middle_meta = Vec::with_capacity(new_affected.len());
            for line_ix in new_affected.clone() {
                let output_line =
                    rows::resolved_output_line_text(output_text, new_line_starts.as_ref(), line_ix);
                let (mut source, mut input_line) = source_lookup
                    .get(output_line)
                    .copied()
                    .unwrap_or((conflict_resolver::ResolvedLineSource::Manual, None));
                if new_dirty_lines.contains(&line_ix) {
                    source = conflict_resolver::ResolvedLineSource::Manual;
                    input_line = None;
                }
                middle_meta.push(conflict_resolver::ResolvedLineMeta {
                    output_line: u32::try_from(line_ix).unwrap_or(u32::MAX),
                    source,
                    input_line,
                });
            }
            middle_meta
        };

        let old_outline = if used_stash {
            self.conflict_resolved_outline_stash
                .as_ref()
                .map(|stash| stash.outline.clone())
                .unwrap_or_default()
        } else {
            std::mem::take(&mut self.conflict_resolver.resolved_outline)
        };
        let old_meta = old_outline.meta;
        let old_markers = old_outline.markers;
        let mut next_sources_index = old_outline.sources_index;
        let line_delta = new_affected.len() as isize - old_affected.len() as isize;

        let mut next_meta = Vec::with_capacity(new_line_count);
        next_meta.extend(
            old_meta
                .iter()
                .take(old_affected.start.min(old_meta.len()))
                .cloned(),
        );
        next_meta.extend(middle_meta);
        for entry in old_meta.iter().skip(old_affected.end.min(old_meta.len())) {
            let mut shifted = entry.clone();
            shifted.output_line =
                u32::try_from(shifted_line_index(entry.output_line as usize, line_delta))
                    .unwrap_or(u32::MAX);
            next_meta.push(shifted);
        }
        apply_conflict_choice_provenance_hints(
            &mut next_meta,
            &self.conflict_resolver.marker_segments,
            output_text,
            new_view_mode,
        );

        let mut next_markers = vec![None; new_line_count];
        for (line_ix, marker) in old_markers
            .iter()
            .copied()
            .enumerate()
            .take(old_affected.start.min(old_markers.len()))
        {
            if line_ix < new_line_count {
                next_markers[line_ix] = marker;
            }
        }
        for (old_line_ix, marker) in old_markers
            .iter()
            .copied()
            .enumerate()
            .skip(old_affected.end.min(old_markers.len()))
        {
            let Some(marker) = marker else {
                continue;
            };
            let new_line_ix = shifted_line_index(old_line_ix, line_delta);
            if new_line_ix < new_line_count {
                next_markers[new_line_ix] = Some(shift_resolved_output_marker(marker, line_delta));
            }
        }
        let blocks: Vec<&conflict_resolver::ConflictBlock> = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                _ => None,
            })
            .collect();
        for conflict_ix in recompute_conflicts {
            let block = blocks[conflict_ix];
            let range = new_block_ranges[conflict_ix].clone();
            let marker_ranges = conflict_marker_ranges_for_block(block, range);
            write_conflict_markers_for_ranges(
                &mut next_markers,
                conflict_ix,
                !block.resolved,
                marker_ranges.as_slice(),
            );
        }

        update_line_sources_index_for_range(
            &mut next_sources_index,
            old_view_mode,
            old_meta.as_slice(),
            old_text,
            old_line_starts.as_ref(),
            old_affected.clone(),
            false,
        );
        update_line_sources_index_for_range(
            &mut next_sources_index,
            new_view_mode,
            next_meta.as_slice(),
            output_text,
            new_line_starts.as_ref(),
            new_affected.clone(),
            true,
        );

        self.conflict_resolved_preview_syntax_language =
            path.and_then(rows::diff_syntax_language_for_path);
        self.conflict_resolved_preview_source_revision = Some(
            ResolvedOutputSourceRevision::from_snapshot(&output_snapshot),
        );
        self.conflict_resolved_preview_line_count = new_line_count;
        self.conflict_resolved_preview_line_starts = new_line_starts;
        self.conflict_resolved_output_measure_row = resolved_output_measure_row(&output_snapshot);
        // The text already reached the live tree on the keystroke. This call is
        // here for what the outline recompute itself changed: the language (the
        // path may have only just resolved) and the unresolved-conflict overlay,
        // which is derived from the marker segments this delta rewrote. It
        // reparses only if the buffer really is different.
        self.refresh_conflict_resolved_output_syntax(&output_snapshot, None, cx);
        self.conflict_resolved_outline_stash = None;
        self.conflict_resolver.resolved_outline = ResolvedOutlineData {
            meta: next_meta,
            markers: next_markers,
            sources_index: next_sources_index,
        };
        self.conflict_resolver.resolved_output_visible_dirty = true;
        self.conflict_resolver.resolved_outline_gutter_rows.clear();
        self.conflict_resolved_preview_text = output_snapshot;
        true
    }

    pub(in crate::view) fn conflict_resolver_scroll_resolved_output_to_line(
        &self,
        target_line_ix: usize,
        line_count: usize,
    ) {
        if line_count == 0 {
            return;
        }
        // Deferred item scrolls apply at the next layout pass, so they work
        // before the lists have ever laid out (initial open) and cannot be
        // clamped against stale bounds. Scrolling the gutter and output
        // lists together leaves the per-frame offset sync nothing to
        // arbitrate, which previously ping-ponged the output back to the
        // top of the file.
        let target_line = target_line_ix.min(line_count.saturating_sub(1));
        // Collapsed context mode: the output lists are in fold-projected row
        // space, so address the row showing the line (or its fold).
        let target = self.resolved_output_visible_ix_for_line(target_line);
        self.conflict_resolved_preview_scroll
            .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.conflict_resolved_preview_gutter_scroll
            .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
    }

    pub(in crate::view) fn conflict_resolver_scroll_resolved_output_to_line_in_text(
        &self,
        target_line_ix: usize,
        output_text: &str,
    ) {
        let line_count = count_newlines(output_text).saturating_add(1);
        self.conflict_resolver_scroll_resolved_output_to_line(target_line_ix, line_count);
    }

    pub(in crate::view) fn schedule_conflict_resolved_outline_recompute(
        &mut self,
        path: Option<std::path::PathBuf>,
        source_revision: ResolvedOutputSourceRevision,
        delta: Option<ResolvedOutlineDelta>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_is_streamed() {
            let _ = source_revision;
            let _ = delta;
            self.refresh_streamed_resolved_output_preview_from_markers(path.as_ref());
            cx.notify();
            return;
        }
        self.conflict_resolver.resolver_pending_recompute_seq = self
            .conflict_resolver
            .resolver_pending_recompute_seq
            .wrapping_add(1);
        let seq = self.conflict_resolver.resolver_pending_recompute_seq;

        #[cfg(test)]
        {
            let did_incremental = delta.clone().is_some_and(|delta| {
                self.recompute_conflict_resolved_outline_and_provenance_incremental(
                    path.as_ref(),
                    delta,
                    cx,
                )
            });
            if did_incremental {
                cx.notify();
                return;
            }

            let trace_started = Instant::now();
            let output_snapshot = self
                .conflict_resolver_input
                .read_with(cx, |input, _| input.text_snapshot());
            let background_delay = self
                .conflict_resolved_outline_background_delay_override
                .unwrap_or_default();
            self.sync_conflict_resolved_preview_snapshot(&output_snapshot, path.as_ref(), true, cx);

            if background_delay.is_zero()
                && self.conflict_resolver.resolver_pending_recompute_seq == seq
                && self.conflict_resolved_preview_source_revision == Some(source_revision)
                && self.conflict_resolved_preview_path.as_ref() == path.as_ref()
            {
                // Built here rather than above so this arm matches production,
                // where the request is assembled inside the debounced task. It
                // copies the document, so hoisting it would charge every
                // keystroke for an outline that only runs once per burst.
                let request = self.background_resolved_outline_recompute_request(&output_snapshot);
                let computed = compute_resolved_outline_computation(
                    request.output_text.as_ref(),
                    request.output_line_count,
                    &request.marker_segments,
                    &request.block_map,
                    request.sources.as_view(),
                );
                self.apply_resolved_outline_computation(path.as_ref(), trace_started, computed);
            }

            cx.notify();
        }

        #[cfg(not(test))]
        {
            cx.spawn(
                async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                    smol::Timer::after(Duration::from_millis(
                        CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS,
                    ))
                    .await;
                    let request = view.update(cx, |this, cx| {
                        if this.conflict_resolver.resolver_pending_recompute_seq != seq {
                            return None;
                        }
                        if this.conflict_resolved_preview_source_revision != Some(source_revision)
                            || this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                        {
                            return None;
                        }
                        let did_incremental = delta.clone().is_some_and(|delta| {
                            this.recompute_conflict_resolved_outline_and_provenance_incremental(
                                path.as_ref(),
                                delta,
                                cx,
                            )
                        });
                        if !did_incremental {
                            let trace_started = Instant::now();
                            let output_snapshot = this
                                .conflict_resolver_input
                                .read_with(cx, |input, _| input.text_snapshot());
                            let request = this
                                .background_resolved_outline_recompute_request(&output_snapshot);
                            let background_delay = Duration::default();
                            this.sync_conflict_resolved_preview_snapshot(
                                &output_snapshot,
                                path.as_ref(),
                                true,
                                cx,
                            );
                            cx.notify();
                            return Some((request, trace_started, background_delay));
                        }

                        cx.notify();
                        None
                    });
                    let Some((request, trace_started, background_delay)) = request.ok().flatten()
                    else {
                        return;
                    };

                    if !background_delay.is_zero() {
                        smol::Timer::after(background_delay).await;
                    }

                    let compute_outline = move || {
                        compute_resolved_outline_computation(
                            request.output_text.as_ref(),
                            request.output_line_count,
                            &request.marker_segments,
                            &request.block_map,
                            request.sources.as_view(),
                        )
                    };
                    let computed = smol::unblock(compute_outline).await;

                    let _ = view.update(cx, |this, cx| {
                        if this.conflict_resolver.resolver_pending_recompute_seq != seq {
                            return;
                        }
                        if this.conflict_resolved_preview_source_revision != Some(source_revision)
                            || this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                        {
                            return;
                        }

                        this.apply_resolved_outline_computation(
                            path.as_ref(),
                            trace_started,
                            computed,
                        );
                        cx.notify();
                    });
                },
            )
            .detach();
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn recompute_conflict_resolved_outline_for_tests(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let path = self.conflict_resolver.path.clone();
        self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
    }

    #[cfg(test)]
    pub(in crate::view) fn set_conflict_resolved_outline_background_delay_override_for_tests(
        &mut self,
        delay: Duration,
    ) {
        self.conflict_resolved_outline_background_delay_override = Some(delay);
    }
}
