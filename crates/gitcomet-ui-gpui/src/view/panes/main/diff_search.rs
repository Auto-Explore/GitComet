use super::*;

impl MainPaneView {
    pub(in crate::view) fn active_conflict_target(
        &self,
    ) -> Option<(
        std::path::PathBuf,
        Option<gitcomet_core::domain::FileConflictKind>,
    )> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()? else {
            return None;
        };
        if *area != DiffArea::Unstaged {
            return None;
        }
        let Loadable::Ready(status) = &repo.status else {
            return None;
        };
        let conflict = status
            .unstaged
            .iter()
            .find(|e| e.path == *path && e.kind == FileStatusKind::Conflicted)?;

        Some((path.clone(), conflict.conflict))
    }

    pub(in super::super::super) fn diff_search_recompute_matches(&mut self) {
        if !self.diff_search_active {
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        if !self.is_file_preview_active() && self.active_conflict_target().is_none() {
            self.ensure_diff_visible_indices();
        }

        self.diff_search_recompute_matches_for_current_view();
    }

    pub(super) fn diff_search_recompute_matches_for_current_view(&mut self) {
        self.diff_search_matches.clear();
        self.diff_search_match_ix = None;

        let query = self.diff_search_query.as_ref().trim();
        if query.is_empty() {
            return;
        }

        if self.is_file_preview_active() {
            let Some(line_count) = self.worktree_preview_line_count() else {
                return;
            };
            for ix in 0..line_count {
                let Some(line) = self.worktree_preview_line_text(ix) else {
                    continue;
                };
                if contains_ascii_case_insensitive(line, query) {
                    self.diff_search_matches.push(ix);
                }
            }
        } else if let Some((_path, conflict_kind)) = self.active_conflict_target() {
            if conflict_kind.is_some() || self.conflict_resolver.path.is_some() {
                let ctx =
                    ConflictResolverSearchContext::from_conflict_resolver(&self.conflict_resolver);
                self.diff_search_matches = conflict_resolver_visible_match_indices(query, &ctx);
            }
        } else {
            let total = self.diff_visible_len();
            for visible_ix in 0..total {
                match self.diff_view {
                    DiffViewMode::Inline => {
                        let text =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                        if contains_ascii_case_insensitive(text.as_ref(), query) {
                            self.diff_search_matches.push(visible_ix);
                        }
                    }
                    DiffViewMode::Split => {
                        let left =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitLeft);
                        let right =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitRight);
                        if contains_ascii_case_insensitive(left.as_ref(), query)
                            || contains_ascii_case_insensitive(right.as_ref(), query)
                        {
                            self.diff_search_matches.push(visible_ix);
                        }
                    }
                }
            }
        }

        if !self.diff_search_matches.is_empty() {
            self.diff_search_match_ix = Some(0);
            let first = self.diff_search_matches[0];
            self.diff_search_scroll_to_visible_ix(first);
        }
    }

    pub(in super::super::super) fn diff_search_prev_match(&mut self) {
        if !self.diff_search_active {
            return;
        }

        if self.diff_search_matches.is_empty() {
            self.diff_search_recompute_matches();
        }
        let len = self.diff_search_matches.len();
        if len == 0 {
            return;
        }

        let current = self
            .diff_search_match_ix
            .unwrap_or(0)
            .min(len.saturating_sub(1));
        let next_ix = if current == 0 { len - 1 } else { current - 1 };
        self.diff_search_match_ix = Some(next_ix);
        let target = self.diff_search_matches[next_ix];
        self.diff_search_scroll_to_visible_ix(target);
    }

    pub(in super::super::super) fn diff_search_next_match(&mut self) {
        if !self.diff_search_active {
            return;
        }

        if self.diff_search_matches.is_empty() {
            self.diff_search_recompute_matches();
        }
        let len = self.diff_search_matches.len();
        if len == 0 {
            return;
        }

        let current = self
            .diff_search_match_ix
            .unwrap_or(0)
            .min(len.saturating_sub(1));
        let next_ix = (current + 1) % len;
        self.diff_search_match_ix = Some(next_ix);
        let target = self.diff_search_matches[next_ix];
        self.diff_search_scroll_to_visible_ix(target);
    }

    fn diff_search_scroll_to_visible_ix(&mut self, visible_ix: usize) {
        if self.is_file_preview_active() {
            self.worktree_preview_scroll
                .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
            return;
        }

        if let Some((_path, conflict_kind)) = self.active_conflict_target() {
            if Self::conflict_resolver_strategy(conflict_kind, false).is_some() {
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
            } else {
                self.diff_scroll
                    .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
            }
            return;
        }

        self.diff_scroll
            .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(visible_ix);
        self.diff_selection_range = Some((visible_ix, visible_ix));
    }
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }

    'outer: for start in 0..=(haystack_bytes.len() - needle_bytes.len()) {
        for (offset, needle_byte) in needle_bytes.iter().copied().enumerate() {
            let haystack_byte = haystack_bytes[start + offset];
            if !haystack_byte.eq_ignore_ascii_case(&needle_byte) {
                continue 'outer;
            }
        }
        return true;
    }

    false
}

#[derive(Clone, Copy)]
enum ConflictResolverSearchVisibleRows<'a> {
    Projection(&'a conflict_resolver::ThreeWayVisibleProjection),
}

impl<'a> ConflictResolverSearchVisibleRows<'a> {
    fn from_conflict_resolver(
        conflict_resolver: &'a ConflictResolverUiState,
    ) -> ConflictResolverSearchVisibleRows<'a> {
        Self::Projection(conflict_resolver.three_way_visible_projection())
    }

    #[cfg(test)]
    fn len(self) -> usize {
        match self {
            Self::Projection(projection) => projection.len(),
        }
    }

    #[cfg(test)]
    fn get(self, visible_ix: usize) -> Option<conflict_resolver::ThreeWayVisibleItem> {
        match self {
            Self::Projection(projection) => projection.get(visible_ix),
        }
    }
}

#[derive(Clone, Copy)]
enum ConflictResolverSearchTwoWayRows<'a> {
    Streamed {
        split_row_index: &'a conflict_resolver::ConflictSplitRowIndex,
        two_way_split_projection: &'a conflict_resolver::TwoWaySplitProjection,
    },
}

impl<'a> ConflictResolverSearchTwoWayRows<'a> {
    fn from_conflict_resolver(
        conflict_resolver: &'a ConflictResolverUiState,
    ) -> ConflictResolverSearchTwoWayRows<'a> {
        let split_row_index = conflict_resolver
            .split_row_index()
            .expect("streamed conflict resolver must always expose split row index");
        let two_way_split_projection = conflict_resolver
            .two_way_split_projection()
            .expect("streamed conflict resolver must always expose split projection");
        Self::Streamed {
            split_row_index,
            two_way_split_projection,
        }
    }
}

#[cfg(test)]
fn empty_conflict_resolver_search_two_way_rows() -> ConflictResolverSearchTwoWayRows<'static> {
    static EMPTY_INDEX: std::sync::LazyLock<conflict_resolver::ConflictSplitRowIndex> =
        std::sync::LazyLock::new(conflict_resolver::ConflictSplitRowIndex::default);
    static EMPTY_PROJECTION: std::sync::LazyLock<conflict_resolver::TwoWaySplitProjection> =
        std::sync::LazyLock::new(conflict_resolver::TwoWaySplitProjection::default);
    ConflictResolverSearchTwoWayRows::Streamed {
        split_row_index: &EMPTY_INDEX,
        two_way_split_projection: &EMPTY_PROJECTION,
    }
}

struct ConflictResolverSearchContext<'a> {
    view_mode: ConflictResolverViewMode,
    marker_segments: &'a [conflict_resolver::ConflictSegment],
    three_way_visible: ConflictResolverSearchVisibleRows<'a>,
    three_way_base_text: &'a str,
    three_way_base_line_starts: &'a [usize],
    three_way_ours_text: &'a str,
    three_way_ours_line_starts: &'a [usize],
    three_way_theirs_text: &'a str,
    three_way_theirs_line_starts: &'a [usize],
    two_way_rows: ConflictResolverSearchTwoWayRows<'a>,
}

impl<'a> ConflictResolverSearchContext<'a> {
    fn from_conflict_resolver(conflict_resolver: &'a ConflictResolverUiState) -> Self {
        let (three_way_base_line_starts, three_way_ours_line_starts, three_way_theirs_line_starts) =
            if conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay {
                (
                    conflict_resolver.three_way_line_starts_ref(ThreeWayColumn::Base),
                    conflict_resolver.three_way_line_starts_ref(ThreeWayColumn::Ours),
                    conflict_resolver.three_way_line_starts_ref(ThreeWayColumn::Theirs),
                )
            } else {
                (&[][..], &[][..], &[][..])
            };
        Self {
            view_mode: conflict_resolver.view_mode,
            marker_segments: &conflict_resolver.marker_segments,
            three_way_visible: ConflictResolverSearchVisibleRows::from_conflict_resolver(
                conflict_resolver,
            ),
            three_way_base_text: &conflict_resolver.three_way_text.base,
            three_way_base_line_starts,
            three_way_ours_text: &conflict_resolver.three_way_text.ours,
            three_way_ours_line_starts,
            three_way_theirs_text: &conflict_resolver.three_way_text.theirs,
            three_way_theirs_line_starts,
            two_way_rows: ConflictResolverSearchTwoWayRows::from_conflict_resolver(
                conflict_resolver,
            ),
        }
    }

    #[cfg(test)]
    fn three_way_visible_len(&self) -> usize {
        self.three_way_visible.len()
    }

    #[cfg(test)]
    fn three_way_visible_item(
        &self,
        visible_ix: usize,
    ) -> Option<conflict_resolver::ThreeWayVisibleItem> {
        self.three_way_visible.get(visible_ix)
    }
}

fn conflict_resolver_visible_match_indices(
    query: &str,
    ctx: &ConflictResolverSearchContext<'_>,
) -> Vec<usize> {
    let mut out = Vec::new();
    match ctx.view_mode {
        ConflictResolverViewMode::ThreeWay => {
            let ConflictResolverSearchVisibleRows::Projection(projection) = ctx.three_way_visible;
            search_three_way_via_spans(projection, ctx, query, &mut out);
        }
        ConflictResolverViewMode::TwoWayDiff => {
            let ConflictResolverSearchTwoWayRows::Streamed {
                split_row_index,
                two_way_split_projection,
            } = ctx.two_way_rows;
            let matching_rows = split_row_index.search_matching_rows(ctx.marker_segments, |text| {
                contains_ascii_case_insensitive(text, query)
            });
            for source_row in matching_rows {
                if let Some(vis) = two_way_split_projection.source_to_visible(source_row) {
                    out.push(vis);
                }
            }
        }
    }
    out
}

/// Search three-way source texts by iterating projection spans directly.
///
/// This avoids the per-visible-item O(log spans) projection lookup by walking
/// spans sequentially and extracting line text from the three source texts.
fn search_three_way_via_spans(
    projection: &conflict_resolver::ThreeWayVisibleProjection,
    ctx: &ConflictResolverSearchContext<'_>,
    query: &str,
    out: &mut Vec<usize>,
) {
    fn line_text<'a>(text: &'a str, line_starts: &[usize], line_ix: usize) -> &'a str {
        if text.is_empty() {
            return "";
        }
        let text_len = text.len();
        let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
        if start >= text_len {
            return "";
        }
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text_len)
            .min(text_len);
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        text.get(start..end).unwrap_or("")
    }

    for span in projection.spans() {
        match *span {
            conflict_resolver::ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } => {
                for i in 0..len {
                    let line_ix = source_line_start + i;
                    let base = line_text(
                        ctx.three_way_base_text,
                        ctx.three_way_base_line_starts,
                        line_ix,
                    );
                    let ours = line_text(
                        ctx.three_way_ours_text,
                        ctx.three_way_ours_line_starts,
                        line_ix,
                    );
                    let theirs = line_text(
                        ctx.three_way_theirs_text,
                        ctx.three_way_theirs_line_starts,
                        line_ix,
                    );
                    if contains_ascii_case_insensitive(base, query)
                        || contains_ascii_case_insensitive(ours, query)
                        || contains_ascii_case_insensitive(theirs, query)
                    {
                        out.push(visible_start + i);
                    }
                }
            }
            conflict_resolver::ThreeWayVisibleSpan::CollapsedResolvedBlock {
                visible_index,
                conflict_ix,
            } => {
                let choice_label = conflict_choice_for_index(ctx.marker_segments, conflict_ix)
                    .map(conflict_choice_label)
                    .unwrap_or("?");
                let summary = format!("Resolved: picked {choice_label}");
                if contains_ascii_case_insensitive(&summary, query) {
                    out.push(visible_index);
                }
            }
        }
    }
}

fn conflict_choice_for_index(
    segments: &[conflict_resolver::ConflictSegment],
    conflict_ix: usize,
) -> Option<conflict_resolver::ConflictChoice> {
    segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block.choice),
            _ => None,
        })
        .nth(conflict_ix)
}

fn conflict_choice_label(choice: conflict_resolver::ConflictChoice) -> &'static str {
    match choice {
        conflict_resolver::ConflictChoice::Base => "Base (A)",
        conflict_resolver::ConflictChoice::Ours => "Local (B)",
        conflict_resolver::ConflictChoice::Theirs => "Remote (C)",
        conflict_resolver::ConflictChoice::Both => "Local+Remote (B+C)",
    }
}

#[cfg(test)]
fn three_way_visible_item_matches_query(
    item: conflict_resolver::ThreeWayVisibleItem,
    ctx: &ConflictResolverSearchContext<'_>,
    query: &str,
) -> bool {
    fn line_text<'a>(text: &'a str, line_starts: &[usize], line_ix: usize) -> &'a str {
        if text.is_empty() {
            return "";
        }
        let text_len = text.len();
        let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
        if start >= text_len {
            return "";
        }
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text_len)
            .min(text_len);
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        text.get(start..end).unwrap_or("")
    }

    match item {
        conflict_resolver::ThreeWayVisibleItem::Line(ix) => {
            let base = line_text(ctx.three_way_base_text, ctx.three_way_base_line_starts, ix);
            let ours = line_text(ctx.three_way_ours_text, ctx.three_way_ours_line_starts, ix);
            let theirs = line_text(
                ctx.three_way_theirs_text,
                ctx.three_way_theirs_line_starts,
                ix,
            );

            contains_ascii_case_insensitive(base, query)
                || contains_ascii_case_insensitive(ours, query)
                || contains_ascii_case_insensitive(theirs, query)
        }
        conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(conflict_ix) => {
            let choice_label = conflict_choice_for_index(ctx.marker_segments, conflict_ix)
                .map(conflict_choice_label)
                .unwrap_or("?");
            let summary = format!("Resolved: picked {choice_label}");
            contains_ascii_case_insensitive(&summary, query)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConflictResolverSearchContext, ConflictResolverSearchTwoWayRows,
        ConflictResolverSearchVisibleRows, conflict_resolver_visible_match_indices,
        contains_ascii_case_insensitive, empty_conflict_resolver_search_two_way_rows,
        three_way_visible_item_matches_query,
    };
    use crate::view::conflict_resolver;
    use crate::view::conflict_resolver::{
        ConflictBlock, ConflictChoice, ConflictResolverViewMode, ConflictSegment,
        ConflictSplitRowIndex, TwoWaySplitProjection, build_three_way_visible_projection,
    };
    use crate::view::{
        ConflictModeState, ConflictResolverUiState, StreamedConflictState, ThreeWaySides,
    };

    fn three_way_search_context<'a>(
        marker_segments: &'a [ConflictSegment],
        visible: &'a conflict_resolver::ThreeWayVisibleProjection,
        base: (&'a str, &'a [usize]),
        ours: (&'a str, &'a [usize]),
        theirs: (&'a str, &'a [usize]),
    ) -> ConflictResolverSearchContext<'a> {
        ConflictResolverSearchContext {
            view_mode: ConflictResolverViewMode::ThreeWay,
            marker_segments,
            three_way_visible: ConflictResolverSearchVisibleRows::Projection(visible),
            three_way_base_text: base.0,
            three_way_base_line_starts: base.1,
            three_way_ours_text: ours.0,
            three_way_ours_line_starts: ours.1,
            three_way_theirs_text: theirs.0,
            three_way_theirs_line_starts: theirs.1,
            two_way_rows: empty_conflict_resolver_search_two_way_rows(),
        }
    }

    #[test]
    fn matches_empty_needle() {
        assert!(contains_ascii_case_insensitive("abc", ""));
    }

    #[test]
    fn matches_case_insensitively() {
        assert!(contains_ascii_case_insensitive("Hello", "he"));
        assert!(contains_ascii_case_insensitive("Hello", "HEL"));
        assert!(contains_ascii_case_insensitive("Hello", "lo"));
    }

    #[test]
    fn does_not_match_absent_substring() {
        assert!(!contains_ascii_case_insensitive("Hello", "world"));
    }

    #[test]
    fn conflict_search_three_way_mode_uses_three_way_visible_rows() {
        let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base".into()),
            ours: "needle\n".into(),
            theirs: "remote\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        })];
        let three_way_visible_projection =
            build_three_way_visible_projection(1, &[0..1], &marker_segments, false);
        let three_way_base_text = "base text\n";
        let three_way_ours_text = "needle\n";
        let three_way_theirs_text = "remote text\n";
        let three_way_base_line_starts = vec![0];
        let three_way_ours_line_starts = vec![0];
        let three_way_theirs_line_starts = vec![0];

        let three_way_ctx = ConflictResolverSearchContext {
            view_mode: ConflictResolverViewMode::ThreeWay,
            marker_segments: &marker_segments,
            three_way_visible: ConflictResolverSearchVisibleRows::Projection(
                &three_way_visible_projection,
            ),
            three_way_base_text,
            three_way_base_line_starts: &three_way_base_line_starts,
            three_way_ours_text,
            three_way_ours_line_starts: &three_way_ours_line_starts,
            three_way_theirs_text,
            three_way_theirs_line_starts: &three_way_theirs_line_starts,
            two_way_rows: empty_conflict_resolver_search_two_way_rows(),
        };

        assert_eq!(
            conflict_resolver_visible_match_indices("needle", &three_way_ctx),
            vec![0]
        );
        assert!(
            conflict_resolver_visible_match_indices("split-only", &three_way_ctx).is_empty(),
            "three-way search should ignore two-way rows",
        );

        let index = ConflictSplitRowIndex::new(&marker_segments, 1);
        let projection = TwoWaySplitProjection::new(&index, &marker_segments, false);
        let two_way_ctx = ConflictResolverSearchContext {
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            marker_segments: &marker_segments,
            three_way_visible: ConflictResolverSearchVisibleRows::Projection(
                &three_way_visible_projection,
            ),
            three_way_base_text,
            three_way_base_line_starts: &three_way_base_line_starts,
            three_way_ours_text,
            three_way_ours_line_starts: &three_way_ours_line_starts,
            three_way_theirs_text,
            three_way_theirs_line_starts: &three_way_theirs_line_starts,
            two_way_rows: ConflictResolverSearchTwoWayRows::Streamed {
                split_row_index: &index,
                two_way_split_projection: &projection,
            },
        };
        assert_eq!(
            conflict_resolver_visible_match_indices("needle", &two_way_ctx),
            vec![0]
        );
    }

    #[test]
    fn conflict_search_three_way_collapsed_rows_match_choice_summary() {
        let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base".into()),
            ours: "ours".into(),
            theirs: "theirs".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        })];
        let three_way_visible_projection =
            build_three_way_visible_projection(1, &[0..1], &marker_segments, true);

        let ctx = three_way_search_context(
            &marker_segments,
            &three_way_visible_projection,
            ("", &[]),
            ("", &[]),
            ("", &[]),
        );

        assert_eq!(
            conflict_resolver_visible_match_indices("resolved", &ctx),
            vec![0]
        );
        assert_eq!(
            conflict_resolver_visible_match_indices("remote", &ctx),
            vec![0]
        );
    }

    #[test]
    fn conflict_search_three_way_projection_uses_streamed_visible_rows() {
        let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base".into()),
            ours: "needle\n".into(),
            theirs: "remote\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let conflict_ranges = vec![0..1];
        let three_way_visible_projection =
            build_three_way_visible_projection(1, &conflict_ranges, &marker_segments, false);

        let ctx = three_way_search_context(
            &marker_segments,
            &three_way_visible_projection,
            ("base\n", &[0]),
            ("needle\n", &[0]),
            ("remote\n", &[0]),
        );

        assert_eq!(
            conflict_resolver_visible_match_indices("needle", &ctx),
            vec![0]
        );
    }

    #[test]
    fn three_way_span_search_matches_per_item_search() {
        // Build a multi-line conflict with text + block segments and verify
        // that span-based search (projection path) yields the same results
        // as per-item search (map path).
        let marker_segments = vec![
            ConflictSegment::Text("header\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base_needle\nbase_plain\n".into()),
                ours: "ours_plain\nours_needle\n".into(),
                theirs: "theirs_plain\ntheirs_plain\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("footer\n".into()),
        ];

        // Three-way line count = max(text_lines) across segments = 1 + 2 + 1 = 4
        let three_way_len = 4;
        let conflict_ranges = vec![1..3]; // lines 1..3 are the conflict block

        let base_text = "header\nbase_needle\nbase_plain\nfooter\n";
        let ours_text = "header\nours_plain\nours_needle\nfooter\n";
        let theirs_text = "header\ntheirs_plain\ntheirs_plain\nfooter\n";
        let base_line_starts = vec![0, 7, 19, 30];
        let ours_line_starts = vec![0, 7, 18, 30];
        let theirs_line_starts = vec![0, 7, 21, 35];

        let projection = build_three_way_visible_projection(
            three_way_len,
            &conflict_ranges,
            &marker_segments,
            false,
        );

        let projection_ctx = three_way_search_context(
            &marker_segments,
            &projection,
            (base_text, &base_line_starts),
            (ours_text, &ours_line_starts),
            (theirs_text, &theirs_line_starts),
        );
        let proj_matches = conflict_resolver_visible_match_indices("needle", &projection_ctx);
        let manual_matches: Vec<usize> = (0..projection_ctx.three_way_visible_len())
            .filter(|&visible_ix| {
                projection_ctx
                    .three_way_visible_item(visible_ix)
                    .is_some_and(|item| {
                        three_way_visible_item_matches_query(item, &projection_ctx, "needle")
                    })
            })
            .collect();

        assert_eq!(
            manual_matches, proj_matches,
            "span-based search must produce same results as per-item search"
        );
        assert!(
            !proj_matches.is_empty(),
            "should find at least one needle match"
        );
    }

    #[test]
    fn two_way_source_text_search_matches_row_based_search() {
        // Build segments, create a ConflictSplitRowIndex + TwoWaySplitProjection,
        // and verify the source-text search path finds the same visible indices
        // as the old row-generation path.
        let marker_segments = vec![
            ConflictSegment::Text("context_line\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "alpha\nneedle_ours\ngamma\n".into(),
                theirs: "delta\nepsilon\nneedle_theirs\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
        ];
        let index = ConflictSplitRowIndex::new(&marker_segments, 1);
        let proj = TwoWaySplitProjection::new(&index, &marker_segments, false);

        let query = "needle";

        // Source-text search path (new):
        let matching_rows = index.search_matching_rows(&marker_segments, |text| {
            contains_ascii_case_insensitive(text, query)
        });
        let mut source_text_matches: Vec<usize> = matching_rows
            .into_iter()
            .filter_map(|r| proj.source_to_visible(r))
            .collect();
        source_text_matches.sort();

        // Row-generation search path (old):
        let mut row_based_matches = Vec::new();
        for visible_ix in 0..proj.visible_len() {
            let Some((source_ix, _)) = proj.get(visible_ix) else {
                continue;
            };
            let Some(row) = index.row_at(&marker_segments, source_ix) else {
                continue;
            };
            if row
                .old
                .as_deref()
                .is_some_and(|s| contains_ascii_case_insensitive(s, query))
                || row
                    .new
                    .as_deref()
                    .is_some_and(|s| contains_ascii_case_insensitive(s, query))
            {
                row_based_matches.push(visible_ix);
            }
        }

        assert_eq!(
            source_text_matches, row_based_matches,
            "source-text search must match row-based search"
        );
        assert!(
            !source_text_matches.is_empty(),
            "should find needle matches"
        );
    }

    #[test]
    fn three_way_span_search_handles_collapsed_blocks() {
        // Verify that collapsed resolved blocks are searchable via span search.
        let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".into()),
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        })];
        let conflict_ranges = vec![0..1];
        let projection =
            build_three_way_visible_projection(1, &conflict_ranges, &marker_segments, true);

        let ctx = three_way_search_context(
            &marker_segments,
            &projection,
            ("base\n", &[0]),
            ("ours\n", &[0]),
            ("theirs\n", &[0]),
        );

        // Collapsed block summary should match "Resolved" and "Remote".
        assert_eq!(
            conflict_resolver_visible_match_indices("resolved", &ctx),
            vec![0]
        );
        assert_eq!(
            conflict_resolver_visible_match_indices("remote", &ctx),
            vec![0]
        );
        // Should not match line content since it's collapsed.
        assert!(
            conflict_resolver_visible_match_indices("ours", &ctx).is_empty(),
            "collapsed block should not expose line content in search"
        );
    }

    #[test]
    fn search_context_from_conflict_resolver_uses_streamed_mode_state() {
        let mut conflict_resolver = ConflictResolverUiState {
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            mode_state: ConflictModeState::Streamed(StreamedConflictState::default()),
            ..ConflictResolverUiState::default()
        };
        conflict_resolver.marker_segments = vec![ConflictSegment::Text("context\n".into())];
        conflict_resolver.three_way_line_starts = ThreeWaySides {
            base: Vec::new().into(),
            ours: vec![0].into(),
            theirs: vec![0].into(),
        };
        conflict_resolver.three_way_text = ThreeWaySides {
            base: "".into(),
            ours: "context".into(),
            theirs: "context".into(),
        };

        let ctx = ConflictResolverSearchContext::from_conflict_resolver(&conflict_resolver);

        assert!(matches!(
            ctx.three_way_visible,
            ConflictResolverSearchVisibleRows::Projection(_)
        ));
        assert!(matches!(
            ctx.two_way_rows,
            ConflictResolverSearchTwoWayRows::Streamed { .. }
        ));
    }
}
