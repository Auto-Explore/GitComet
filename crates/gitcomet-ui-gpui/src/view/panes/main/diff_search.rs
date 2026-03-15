use super::*;

impl MainPaneView {
    fn active_conflict_target(
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
            let is_conflict_resolver =
                Self::conflict_resolver_strategy(conflict_kind, false).is_some();

            match (is_conflict_resolver, self.diff_view) {
                (true, _) => {
                    let ctx = ConflictResolverSearchContext {
                        view_mode: self.conflict_resolver.view_mode,
                        diff_mode: self.conflict_resolver.diff_mode,
                        marker_segments: &self.conflict_resolver.marker_segments,
                        three_way_visible_map: &self.conflict_resolver.three_way_visible_map,
                        three_way_base_text: &self.conflict_resolver.three_way_text.base,
                        three_way_base_line_starts: &self
                            .conflict_resolver
                            .three_way_line_starts
                            .base,
                        three_way_ours_text: &self.conflict_resolver.three_way_text.ours,
                        three_way_ours_line_starts: &self
                            .conflict_resolver
                            .three_way_line_starts
                            .ours,
                        three_way_theirs_text: &self.conflict_resolver.three_way_text.theirs,
                        three_way_theirs_line_starts: &self
                            .conflict_resolver
                            .three_way_line_starts
                            .theirs,
                        diff_visible_row_indices: &self.conflict_resolver.diff_visible_row_indices,
                        inline_visible_row_indices: &self
                            .conflict_resolver
                            .inline_visible_row_indices,
                        diff_rows: &self.conflict_resolver.diff_rows,
                        inline_rows: &self.conflict_resolver.inline_rows,
                    };
                    self.diff_search_matches = conflict_resolver_visible_match_indices(query, &ctx);
                }
                (false, DiffViewMode::Split) => {
                    for (ix, row) in self.conflict_resolver.diff_rows.iter().enumerate() {
                        if row
                            .old
                            .as_deref()
                            .is_some_and(|s| contains_ascii_case_insensitive(s, query))
                            || row
                                .new
                                .as_deref()
                                .is_some_and(|s| contains_ascii_case_insensitive(s, query))
                        {
                            self.diff_search_matches.push(ix);
                        }
                    }
                }
                (false, DiffViewMode::Inline) => {
                    for (ix, row) in self.conflict_resolver.inline_rows.iter().enumerate() {
                        if contains_ascii_case_insensitive(row.content.as_str(), query) {
                            self.diff_search_matches.push(ix);
                        }
                    }
                }
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

struct ConflictResolverSearchContext<'a> {
    view_mode: ConflictResolverViewMode,
    diff_mode: ConflictDiffMode,
    marker_segments: &'a [conflict_resolver::ConflictSegment],
    three_way_visible_map: &'a [conflict_resolver::ThreeWayVisibleItem],
    three_way_base_text: &'a str,
    three_way_base_line_starts: &'a [usize],
    three_way_ours_text: &'a str,
    three_way_ours_line_starts: &'a [usize],
    three_way_theirs_text: &'a str,
    three_way_theirs_line_starts: &'a [usize],
    diff_visible_row_indices: &'a [usize],
    inline_visible_row_indices: &'a [usize],
    diff_rows: &'a [gitcomet_core::file_diff::FileDiffRow],
    inline_rows: &'a [conflict_resolver::ConflictInlineRow],
}

fn conflict_resolver_visible_match_indices(
    query: &str,
    ctx: &ConflictResolverSearchContext<'_>,
) -> Vec<usize> {
    let mut out = Vec::new();
    match ctx.view_mode {
        ConflictResolverViewMode::ThreeWay => {
            for (visible_ix, item) in ctx.three_way_visible_map.iter().copied().enumerate() {
                if three_way_visible_item_matches_query(item, ctx, query) {
                    out.push(visible_ix);
                }
            }
        }
        ConflictResolverViewMode::TwoWayDiff => match ctx.diff_mode {
            ConflictDiffMode::Split => {
                for (visible_ix, &row_ix) in ctx.diff_visible_row_indices.iter().enumerate() {
                    let Some(row) = ctx.diff_rows.get(row_ix) else {
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
                        out.push(visible_ix);
                    }
                }
            }
            ConflictDiffMode::Inline => {
                for (visible_ix, &row_ix) in ctx.inline_visible_row_indices.iter().enumerate() {
                    let Some(row) = ctx.inline_rows.get(row_ix) else {
                        continue;
                    };
                    if contains_ascii_case_insensitive(row.content.as_str(), query) {
                        out.push(visible_ix);
                    }
                }
            }
        },
    }
    out
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
        ConflictResolverSearchContext, conflict_resolver_visible_match_indices,
        contains_ascii_case_insensitive,
    };
    use crate::view::conflict_resolver::{
        ConflictBlock, ConflictChoice, ConflictDiffMode, ConflictResolverViewMode, ConflictSegment,
        ThreeWayVisibleItem,
    };
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

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
            ours: "ours".into(),
            theirs: "theirs".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        })];
        let diff_rows = vec![FileDiffRow {
            kind: FileDiffRowKind::Modify,
            old_line: Some(1),
            new_line: Some(1),
            old: Some("split-only".into()),
            new: Some("split-only".into()),
            eof_newline: None,
        }];
        let inline_rows = vec![crate::view::conflict_resolver::ConflictInlineRow {
            side: crate::view::conflict_resolver::ConflictPickSide::Ours,
            kind: DiffLineKind::Add,
            old_line: Some(1),
            new_line: Some(1),
            content: "inline-only".into(),
        }];
        let three_way_visible_map = vec![ThreeWayVisibleItem::Line(0)];
        let three_way_base_text = "base text\n";
        let three_way_ours_text = "needle\n";
        let three_way_theirs_text = "remote text\n";
        let three_way_base_line_starts = vec![0];
        let three_way_ours_line_starts = vec![0];
        let three_way_theirs_line_starts = vec![0];

        let three_way_ctx = ConflictResolverSearchContext {
            view_mode: ConflictResolverViewMode::ThreeWay,
            diff_mode: ConflictDiffMode::Split,
            marker_segments: &marker_segments,
            three_way_visible_map: &three_way_visible_map,
            three_way_base_text,
            three_way_base_line_starts: &three_way_base_line_starts,
            three_way_ours_text,
            three_way_ours_line_starts: &three_way_ours_line_starts,
            three_way_theirs_text,
            three_way_theirs_line_starts: &three_way_theirs_line_starts,
            diff_visible_row_indices: &[0],
            inline_visible_row_indices: &[0],
            diff_rows: &diff_rows,
            inline_rows: &inline_rows,
        };

        assert_eq!(
            conflict_resolver_visible_match_indices("needle", &three_way_ctx),
            vec![0]
        );
        assert!(
            conflict_resolver_visible_match_indices("split-only", &three_way_ctx).is_empty(),
            "three-way search should ignore two-way rows",
        );

        let two_way_ctx = ConflictResolverSearchContext {
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            diff_mode: ConflictDiffMode::Split,
            marker_segments: &marker_segments,
            three_way_visible_map: &three_way_visible_map,
            three_way_base_text,
            three_way_base_line_starts: &three_way_base_line_starts,
            three_way_ours_text,
            three_way_ours_line_starts: &three_way_ours_line_starts,
            three_way_theirs_text,
            three_way_theirs_line_starts: &three_way_theirs_line_starts,
            diff_visible_row_indices: &[0],
            inline_visible_row_indices: &[0],
            diff_rows: &diff_rows,
            inline_rows: &inline_rows,
        };
        assert_eq!(
            conflict_resolver_visible_match_indices("split-only", &two_way_ctx),
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
        let three_way_visible_map = vec![ThreeWayVisibleItem::CollapsedBlock(0)];

        let ctx = ConflictResolverSearchContext {
            view_mode: ConflictResolverViewMode::ThreeWay,
            diff_mode: ConflictDiffMode::Split,
            marker_segments: &marker_segments,
            three_way_visible_map: &three_way_visible_map,
            three_way_base_text: "",
            three_way_base_line_starts: &[],
            three_way_ours_text: "",
            three_way_ours_line_starts: &[],
            three_way_theirs_text: "",
            three_way_theirs_line_starts: &[],
            diff_visible_row_indices: &[],
            inline_visible_row_indices: &[],
            diff_rows: &[],
            inline_rows: &[],
        };

        assert_eq!(
            conflict_resolver_visible_match_indices("resolved", &ctx),
            vec![0]
        );
        assert_eq!(
            conflict_resolver_visible_match_indices("remote", &ctx),
            vec![0]
        );
    }
}
