use super::*;
use gitcomet_core::domain::Diff;
use memchr::memchr2_iter;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::borrow::Cow;

#[derive(Clone, Copy)]
pub(in crate::view) struct AsciiCaseInsensitiveNeedle<'a> {
    bytes: &'a [u8],
    first_lower: u8,
    first_upper: u8,
    last_lower: u8,
    last_upper: u8,
}

impl<'a> AsciiCaseInsensitiveNeedle<'a> {
    #[inline]
    pub(in crate::view) fn new(needle: &'a str) -> Option<Self> {
        let bytes = needle.as_bytes();
        let (&first, &last) = bytes.first().zip(bytes.last())?;

        Some(Self {
            bytes,
            first_lower: first.to_ascii_lowercase(),
            first_upper: first.to_ascii_uppercase(),
            last_lower: last.to_ascii_lowercase(),
            last_upper: last.to_ascii_uppercase(),
        })
    }

    #[inline]
    pub(in crate::view) fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub(in crate::view) fn is_match(self, haystack: &str) -> bool {
        let haystack_bytes = haystack.as_bytes();
        let needle_len = self.bytes.len();
        let Some(last_start) = haystack_bytes.len().checked_sub(needle_len) else {
            return false;
        };

        if needle_len == 1 {
            return memchr2_iter(self.first_lower, self.first_upper, haystack_bytes)
                .next()
                .is_some();
        }

        let middle = &self.bytes[1..needle_len - 1];
        for start in memchr2_iter(
            self.first_lower,
            self.first_upper,
            &haystack_bytes[..=last_start],
        ) {
            let last = haystack_bytes[start + needle_len - 1];
            if last != self.last_lower && last != self.last_upper {
                continue;
            }

            if haystack_bytes[start + 1..start + needle_len - 1].eq_ignore_ascii_case(middle) {
                return true;
            }
        }

        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum DiffSearchQueryReuse {
    None,
    SameSemantics,
    Refinement,
}

#[derive(Clone, Debug, Default)]
pub(in crate::view) struct DiffSearchVisibleTrigramIndex {
    postings: FxHashMap<u32, Vec<u32>>,
}

pub(in crate::view) enum DiffSearchVisibleCandidates<'a> {
    All,
    Indexed(&'a [u32]),
    None,
}

impl DiffSearchVisibleTrigramIndex {
    pub(in crate::view) fn insert_text(
        &mut self,
        visible_ix: u32,
        text: &str,
        trigrams: &mut SmallVec<[u32; 64]>,
    ) {
        collect_unique_ascii_folded_byte_trigrams(text.as_bytes(), trigrams);
        for trigram in trigrams.iter().copied() {
            self.postings.entry(trigram).or_default().push(visible_ix);
        }
    }

    pub(in crate::view) fn finish(mut self) -> Self {
        for indices in self.postings.values_mut() {
            indices.shrink_to_fit();
        }
        self
    }

    pub(in crate::view) fn candidates<'a>(
        &'a self,
        needle: &[u8],
    ) -> DiffSearchVisibleCandidates<'a> {
        if needle.len() < 3 {
            return DiffSearchVisibleCandidates::All;
        }

        let mut trigrams = SmallVec::<[u32; 64]>::new();
        collect_unique_ascii_folded_byte_trigrams(needle, &mut trigrams);

        let mut best: Option<&[u32]> = None;
        for trigram in trigrams.iter() {
            let Some(postings) = self.postings.get(trigram).map(Vec::as_slice) else {
                return DiffSearchVisibleCandidates::None;
            };
            if best.is_none_or(|current| postings.len() < current.len()) {
                best = Some(postings);
            }
        }

        match best {
            Some(postings) => DiffSearchVisibleCandidates::Indexed(postings),
            None => DiffSearchVisibleCandidates::All,
        }
    }
}

pub(in crate::view) fn build_resolved_output_trigram_index(
    text: &str,
    line_starts: &[usize],
    line_count: usize,
) -> DiffSearchVisibleTrigramIndex {
    let mut index = DiffSearchVisibleTrigramIndex::default();
    let mut trigrams = SmallVec::<[u32; 64]>::new();
    for line_ix in 0..line_count {
        index.insert_text(
            line_ix as u32,
            rows::resolved_output_line_text(text, line_starts, line_ix),
            &mut trigrams,
        );
    }
    index.finish()
}

#[inline]
fn diff_search_displayed_text_matches_query(
    query: AsciiCaseInsensitiveNeedle<'_>,
    text: &str,
    expanded_tabs: &mut String,
) -> bool {
    if !text.contains('\t') {
        return query.is_match(text);
    }

    expanded_tabs.clear();
    for ch in text.chars() {
        match ch {
            '\t' => expanded_tabs.push_str("    "),
            _ => expanded_tabs.push(ch),
        }
    }
    query.is_match(expanded_tabs.as_str())
}

pub(in crate::view) fn diff_search_split_row_texts_match_query(
    query: AsciiCaseInsensitiveNeedle<'_>,
    left: Option<&str>,
    right: Option<&str>,
    expanded_tabs: &mut String,
) -> bool {
    if let Some(text) = left
        && diff_search_displayed_text_matches_query(query, text, expanded_tabs)
    {
        return true;
    }

    right.is_some_and(|text| diff_search_displayed_text_matches_query(query, text, expanded_tabs))
}

#[inline]
pub(in crate::view) fn diff_search_query_reuse(
    previous_query: &str,
    next_query: &str,
) -> DiffSearchQueryReuse {
    let previous_query = previous_query.trim();
    let next_query = next_query.trim();
    if next_query
        .as_bytes()
        .eq_ignore_ascii_case(previous_query.as_bytes())
    {
        return DiffSearchQueryReuse::SameSemantics;
    }

    if !previous_query.is_empty()
        && next_query.len() > previous_query.len()
        && next_query
            .as_bytes()
            .get(..previous_query.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(previous_query.as_bytes()))
    {
        return DiffSearchQueryReuse::Refinement;
    }

    DiffSearchQueryReuse::None
}

fn collect_unique_ascii_folded_byte_trigrams(bytes: &[u8], trigrams: &mut SmallVec<[u32; 64]>) {
    trigrams.clear();
    if bytes.len() < 3 {
        return;
    }

    trigrams.extend(bytes.windows(3).map(encode_ascii_folded_byte_trigram));
    trigrams.sort_unstable();
    trigrams.dedup();
}

fn encode_ascii_folded_byte_trigram(window: &[u8]) -> u32 {
    debug_assert_eq!(window.len(), 3);
    (u32::from(window[0].to_ascii_lowercase()) << 16)
        | (u32::from(window[1].to_ascii_lowercase()) << 8)
        | u32::from(window[2].to_ascii_lowercase())
}

fn inline_patch_diff_search_text<'a>(
    diff: &'a Diff,
    diff_click_kinds: &[DiffClickKind],
    diff_header_display_cache: &'a HashMap<usize, SharedString>,
    src_ix: usize,
) -> Option<Cow<'a, str>> {
    let line = diff.lines.get(src_ix)?;
    let click_kind = diff_click_kinds
        .get(src_ix)
        .copied()
        .unwrap_or(DiffClickKind::Line);
    if matches!(
        click_kind,
        DiffClickKind::HunkHeader | DiffClickKind::FileHeader
    ) && let Some(display) = diff_header_display_cache.get(&src_ix)
    {
        return Some(Cow::Borrowed(display.as_ref()));
    }

    if !line.text.contains('\t') {
        return Some(Cow::Borrowed(line.text.as_ref()));
    }

    let mut expanded = String::with_capacity(line.text.len());
    for ch in line.text.chars() {
        match ch {
            '\t' => expanded.push_str("    "),
            _ => expanded.push(ch),
        }
    }
    Some(Cow::Owned(expanded))
}

fn inline_patch_diff_src_ix_for_visible_ix(
    diff_visible_inline_map: Option<&super::diff_cache::PatchInlineVisibleMap>,
    diff_visible_indices: &[usize],
    visible_ix: usize,
) -> Option<usize> {
    if let Some(map) = diff_visible_inline_map {
        return map.src_ix_for_visible_ix(visible_ix);
    }
    diff_visible_indices.get(visible_ix).copied()
}

fn inline_patch_diff_visible_ix_matches_query(
    diff: &Diff,
    diff_click_kinds: &[DiffClickKind],
    diff_header_display_cache: &HashMap<usize, SharedString>,
    diff_visible_inline_map: Option<&super::diff_cache::PatchInlineVisibleMap>,
    diff_visible_indices: &[usize],
    query: AsciiCaseInsensitiveNeedle<'_>,
    visible_ix: usize,
) -> bool {
    let Some(src_ix) = inline_patch_diff_src_ix_for_visible_ix(
        diff_visible_inline_map,
        diff_visible_indices,
        visible_ix,
    ) else {
        return false;
    };
    inline_patch_diff_search_text(diff, diff_click_kinds, diff_header_display_cache, src_ix)
        .is_some_and(|text| query.is_match(text.as_ref()))
}

fn resolved_output_line_ix_matches_query(
    text: &str,
    line_starts: &[usize],
    query: AsciiCaseInsensitiveNeedle<'_>,
    line_ix: usize,
) -> bool {
    query.is_match(rows::resolved_output_line_text(text, line_starts, line_ix))
}

fn retain_refined_visible_matches(
    matches: &mut Vec<usize>,
    candidates: DiffSearchVisibleCandidates<'_>,
    mut visible_ix_matches_query: impl FnMut(usize) -> bool,
) {
    match candidates {
        DiffSearchVisibleCandidates::None => {
            matches.clear();
        }
        DiffSearchVisibleCandidates::All => {
            matches.retain(|&visible_ix| visible_ix_matches_query(visible_ix));
        }
        DiffSearchVisibleCandidates::Indexed(candidate_visible_rows) => {
            if candidate_visible_rows.len() >= matches.len() {
                matches.retain(|&visible_ix| visible_ix_matches_query(visible_ix));
                return;
            }

            let mut read_ix = 0usize;
            let mut write_ix = 0usize;
            let mut candidate_ix = 0usize;

            while read_ix < matches.len() && candidate_ix < candidate_visible_rows.len() {
                let visible_ix = matches[read_ix];
                let candidate_visible_ix = candidate_visible_rows[candidate_ix] as usize;
                if visible_ix < candidate_visible_ix {
                    read_ix += 1;
                    continue;
                }
                if visible_ix > candidate_visible_ix {
                    candidate_ix += 1;
                    continue;
                }

                if visible_ix_matches_query(visible_ix) {
                    matches[write_ix] = visible_ix;
                    write_ix += 1;
                }
                read_ix += 1;
                candidate_ix += 1;
            }

            matches.truncate(write_ix);
        }
    }
}

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

    pub(super) fn diff_search_recompute_matches_for_query_change(&mut self, previous_query: &str) {
        if !self.diff_search_active {
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        self.diff_search_match_ix = None;
        let query_text = self.diff_search_query.clone();
        let query_text = query_text.as_ref().trim();

        let Some(query) = AsciiCaseInsensitiveNeedle::new(query_text) else {
            self.diff_search_matches.clear();
            return;
        };

        match diff_search_query_reuse(previous_query, query_text) {
            DiffSearchQueryReuse::SameSemantics => {}
            DiffSearchQueryReuse::Refinement if self.diff_search_can_refine_current_matches() => {
                let mut previous_matches = std::mem::take(&mut self.diff_search_matches);
                if !(self
                    .diff_search_try_refine_worktree_preview_matches(query, &mut previous_matches)
                    || self
                        .diff_search_try_refine_inline_patch_matches(query, &mut previous_matches))
                {
                    if self.is_file_diff_view_active() && self.diff_view == DiffViewMode::Split {
                        let mut expanded_tabs = String::new();
                        previous_matches.retain(|&visible_ix| {
                            self.diff_search_file_diff_split_visible_row_matches_query(
                                query,
                                visible_ix,
                                &mut expanded_tabs,
                            )
                        });
                    } else {
                        previous_matches.retain(|&visible_ix| {
                            self.diff_search_visible_row_matches_query(query, visible_ix)
                        });
                    }
                }
                self.diff_search_matches = previous_matches;
            }
            DiffSearchQueryReuse::None | DiffSearchQueryReuse::Refinement => {
                self.diff_search_scan_current_view_with_needle(query);
            }
        }

        self.diff_search_finalize_matches();
    }

    pub(super) fn diff_search_recompute_matches_for_current_view(&mut self) {
        self.diff_search_match_ix = None;
        let query_text = self.diff_search_query.clone();

        let Some(query) = AsciiCaseInsensitiveNeedle::new(query_text.as_ref().trim()) else {
            self.diff_search_matches.clear();
            return;
        };

        self.diff_search_scan_current_view_with_needle(query);
        self.diff_search_finalize_matches();
    }

    fn diff_search_scan_current_view_with_needle(&mut self, query: AsciiCaseInsensitiveNeedle<'_>) {
        self.diff_search_matches.clear();

        if self.is_file_preview_active() {
            let Some(line_count) = self.worktree_preview_line_count() else {
                return;
            };
            let source_text = self.worktree_preview_text.as_ref();
            let line_starts = self.worktree_preview_line_starts.as_ref();
            if let Some(index) = self.worktree_preview_search_trigram_index.as_ref() {
                match index.candidates(query.as_bytes()) {
                    DiffSearchVisibleCandidates::None => {}
                    DiffSearchVisibleCandidates::All => {
                        for ix in 0..line_count {
                            if resolved_output_line_ix_matches_query(
                                source_text,
                                line_starts,
                                query,
                                ix,
                            ) {
                                self.diff_search_matches.push(ix);
                            }
                        }
                    }
                    DiffSearchVisibleCandidates::Indexed(candidate_rows) => {
                        for &ix in candidate_rows {
                            let ix = ix as usize;
                            if resolved_output_line_ix_matches_query(
                                source_text,
                                line_starts,
                                query,
                                ix,
                            ) {
                                self.diff_search_matches.push(ix);
                            }
                        }
                    }
                }
            } else {
                for ix in 0..line_count {
                    if resolved_output_line_ix_matches_query(source_text, line_starts, query, ix) {
                        self.diff_search_matches.push(ix);
                    }
                }
            }
        } else if let Some((_path, conflict_kind)) = self.active_conflict_target() {
            if conflict_kind.is_some() || self.conflict_resolver.path.is_some() {
                let ctx =
                    ConflictResolverSearchContext::from_conflict_resolver(&self.conflict_resolver);
                self.diff_search_matches =
                    conflict_resolver_visible_match_indices_with_needle(query, &ctx);
            }
        } else {
            if self.diff_view == DiffViewMode::Inline
                && !self.is_file_diff_view_active()
                && self.diff_search_scan_inline_patch_diff_with_needle(query)
            {
                return;
            }

            let total = self.diff_visible_len();
            if self.diff_view == DiffViewMode::Split && self.is_file_diff_view_active() {
                let mut expanded_tabs = String::new();
                for visible_ix in 0..total {
                    if self.diff_search_file_diff_split_visible_row_matches_query(
                        query,
                        visible_ix,
                        &mut expanded_tabs,
                    ) {
                        self.diff_search_matches.push(visible_ix);
                    }
                }
                return;
            }

            for visible_ix in 0..total {
                match self.diff_view {
                    DiffViewMode::Inline => {
                        let text =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                        if query.is_match(text.as_ref()) {
                            self.diff_search_matches.push(visible_ix);
                        }
                    }
                    DiffViewMode::Split => {
                        let left =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitLeft);
                        let right =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitRight);
                        if query.is_match(left.as_ref()) || query.is_match(right.as_ref()) {
                            self.diff_search_matches.push(visible_ix);
                        }
                    }
                }
            }
        }
    }

    fn diff_search_scan_inline_patch_diff_with_needle(
        &mut self,
        query: AsciiCaseInsensitiveNeedle<'_>,
    ) -> bool {
        let diff = match self.active_repo().map(|repo| &repo.diff_state.diff) {
            Some(Loadable::Ready(diff)) => Arc::clone(diff),
            _ => return false,
        };

        if self.diff_search_inline_patch_trigram_index.is_none() {
            let mut index = DiffSearchVisibleTrigramIndex::default();
            let mut trigrams = SmallVec::<[u32; 64]>::new();
            if let Some(map) = self.diff_visible_inline_map.as_ref() {
                map.for_each_visible_src_ix(|visible_ix, src_ix| {
                    if let Some(text) = inline_patch_diff_search_text(
                        diff.as_ref(),
                        &self.diff_click_kinds,
                        &self.diff_header_display_cache,
                        src_ix,
                    ) {
                        index.insert_text(visible_ix as u32, text.as_ref(), &mut trigrams);
                    }
                });
            } else {
                for (visible_ix, &src_ix) in self.diff_visible_indices.iter().enumerate() {
                    if let Some(text) = inline_patch_diff_search_text(
                        diff.as_ref(),
                        &self.diff_click_kinds,
                        &self.diff_header_display_cache,
                        src_ix,
                    ) {
                        index.insert_text(visible_ix as u32, text.as_ref(), &mut trigrams);
                    }
                }
            }
            self.diff_search_inline_patch_trigram_index = Some(index.finish());
        }

        let index = self
            .diff_search_inline_patch_trigram_index
            .as_ref()
            .expect("inline patch diff trigram index initialized");
        let diff_click_kinds = &self.diff_click_kinds;
        let diff_header_display_cache = &self.diff_header_display_cache;
        let diff_visible_inline_map = self.diff_visible_inline_map.as_ref();
        let diff_visible_indices = &self.diff_visible_indices;
        let matches = &mut self.diff_search_matches;

        match index.candidates(query.bytes) {
            DiffSearchVisibleCandidates::None => {}
            DiffSearchVisibleCandidates::All => {
                let total = diff_visible_inline_map
                    .map(super::diff_cache::PatchInlineVisibleMap::visible_len)
                    .unwrap_or(diff_visible_indices.len());
                for visible_ix in 0..total {
                    if inline_patch_diff_visible_ix_matches_query(
                        diff.as_ref(),
                        diff_click_kinds,
                        diff_header_display_cache,
                        diff_visible_inline_map,
                        diff_visible_indices,
                        query,
                        visible_ix,
                    ) {
                        matches.push(visible_ix);
                    }
                }
            }
            DiffSearchVisibleCandidates::Indexed(candidate_visible_rows) => {
                for &visible_ix in candidate_visible_rows {
                    let visible_ix = visible_ix as usize;
                    if inline_patch_diff_visible_ix_matches_query(
                        diff.as_ref(),
                        diff_click_kinds,
                        diff_header_display_cache,
                        diff_visible_inline_map,
                        diff_visible_indices,
                        query,
                        visible_ix,
                    ) {
                        matches.push(visible_ix);
                    }
                }
            }
        }

        true
    }

    fn diff_search_file_diff_split_visible_row_matches_query(
        &self,
        query: AsciiCaseInsensitiveNeedle<'_>,
        visible_ix: usize,
        expanded_tabs: &mut String,
    ) -> bool {
        if !self.is_file_diff_view_active() || self.diff_view != DiffViewMode::Split {
            return false;
        }
        let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
            return false;
        };
        let Some(provider) = self.file_diff_row_provider.as_ref() else {
            return false;
        };
        let Some((left, right)) = provider.split_row_texts(mapped_ix) else {
            return false;
        };
        diff_search_split_row_texts_match_query(query, left, right, expanded_tabs)
    }

    fn diff_search_can_refine_current_matches(&self) -> bool {
        self.is_file_preview_active() || self.active_conflict_target().is_none()
    }

    fn diff_search_try_refine_inline_patch_matches(
        &self,
        query: AsciiCaseInsensitiveNeedle<'_>,
        previous_matches: &mut Vec<usize>,
    ) -> bool {
        if self.is_file_preview_active()
            || self.active_conflict_target().is_some()
            || self.diff_view != DiffViewMode::Inline
            || self.is_file_diff_view_active()
        {
            return false;
        }

        let Some(diff) = self.active_repo().map(|repo| &repo.diff_state.diff) else {
            return false;
        };
        let Loadable::Ready(diff) = diff else {
            return false;
        };
        let Some(index) = self.diff_search_inline_patch_trigram_index.as_ref() else {
            return false;
        };

        let diff_click_kinds = &self.diff_click_kinds;
        let diff_header_display_cache = &self.diff_header_display_cache;
        let diff_visible_inline_map = self.diff_visible_inline_map.as_ref();
        let diff_visible_indices = &self.diff_visible_indices;
        retain_refined_visible_matches(
            previous_matches,
            index.candidates(query.as_bytes()),
            |visible_ix| {
                inline_patch_diff_visible_ix_matches_query(
                    diff.as_ref(),
                    diff_click_kinds,
                    diff_header_display_cache,
                    diff_visible_inline_map,
                    diff_visible_indices,
                    query,
                    visible_ix,
                )
            },
        );
        true
    }

    fn diff_search_try_refine_worktree_preview_matches(
        &self,
        query: AsciiCaseInsensitiveNeedle<'_>,
        previous_matches: &mut Vec<usize>,
    ) -> bool {
        if !self.is_file_preview_active() {
            return false;
        }
        let Some(index) = self.worktree_preview_search_trigram_index.as_ref() else {
            return false;
        };

        let source_text = self.worktree_preview_text.as_ref();
        let line_starts = self.worktree_preview_line_starts.as_ref();
        retain_refined_visible_matches(
            previous_matches,
            index.candidates(query.as_bytes()),
            |line_ix| {
                resolved_output_line_ix_matches_query(source_text, line_starts, query, line_ix)
            },
        );
        true
    }

    fn diff_search_visible_row_matches_query(
        &self,
        query: AsciiCaseInsensitiveNeedle<'_>,
        visible_ix: usize,
    ) -> bool {
        if self.is_file_preview_active() {
            return self
                .worktree_preview_line_text(visible_ix)
                .is_some_and(|line| query.is_match(line));
        }

        match self.diff_view {
            DiffViewMode::Inline => query.is_match(
                self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline)
                    .as_ref(),
            ),
            DiffViewMode::Split => {
                if self.is_file_diff_view_active() {
                    let mut expanded_tabs = String::new();
                    return self.diff_search_file_diff_split_visible_row_matches_query(
                        query,
                        visible_ix,
                        &mut expanded_tabs,
                    );
                }
                let left = self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitLeft);
                let right = self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitRight);
                query.is_match(left.as_ref()) || query.is_match(right.as_ref())
            }
        }
    }

    fn diff_search_finalize_matches(&mut self) {
        if self.diff_search_matches.is_empty() {
            return;
        }
        self.diff_search_match_ix = Some(0);
        let first = self.diff_search_matches[0];
        self.diff_search_scroll_to_visible_ix(first);
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
                self.conflict_resolver_scroll_all_columns(visible_ix, gpui::ScrollStrategy::Center);
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

#[cfg(test)]
fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    match AsciiCaseInsensitiveNeedle::new(needle) {
        Some(needle) => needle.is_match(haystack),
        None => true,
    }
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

#[cfg(test)]
fn conflict_resolver_visible_match_indices(
    query: &str,
    ctx: &ConflictResolverSearchContext<'_>,
) -> Vec<usize> {
    let Some(query) = AsciiCaseInsensitiveNeedle::new(query) else {
        return Vec::new();
    };
    conflict_resolver_visible_match_indices_with_needle(query, ctx)
}

fn conflict_resolver_visible_match_indices_with_needle(
    query: AsciiCaseInsensitiveNeedle<'_>,
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
            let matching_rows = split_row_index
                .search_ascii_case_insensitive_matching_rows(ctx.marker_segments, query.bytes);
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
    query: AsciiCaseInsensitiveNeedle<'_>,
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
                    if query.is_match(base) || query.is_match(ours) || query.is_match(theirs) {
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
                if query.is_match(&summary) {
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
        AsciiCaseInsensitiveNeedle, ConflictResolverSearchContext,
        ConflictResolverSearchTwoWayRows, ConflictResolverSearchVisibleRows, DiffSearchQueryReuse,
        conflict_resolver_visible_match_indices, contains_ascii_case_insensitive,
        diff_search_query_reuse, diff_search_split_row_texts_match_query,
        empty_conflict_resolver_search_two_way_rows, three_way_visible_item_matches_query,
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
    fn diff_search_query_reuse_detects_same_semantics_and_refinements() {
        assert_eq!(
            diff_search_query_reuse("Render_Cache", " render_cache "),
            DiffSearchQueryReuse::SameSemantics
        );
        assert_eq!(
            diff_search_query_reuse("render_cache", "render_cache_hot_path"),
            DiffSearchQueryReuse::Refinement
        );
        assert_eq!(
            diff_search_query_reuse("", "render_cache"),
            DiffSearchQueryReuse::None
        );
        assert_eq!(
            diff_search_query_reuse("render_cache", "cache_render"),
            DiffSearchQueryReuse::None
        );
    }

    #[test]
    fn split_row_text_search_matches_rendered_tab_expansion() {
        let query = AsciiCaseInsensitiveNeedle::new("a    b").expect("query");
        let mut expanded_tabs = String::new();

        assert!(diff_search_split_row_texts_match_query(
            query,
            Some("a\tb"),
            None,
            &mut expanded_tabs,
        ));
        assert!(diff_search_split_row_texts_match_query(
            query,
            None,
            Some("a\tb"),
            &mut expanded_tabs,
        ));
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
        let visible_range = 0..1;
        let three_way_visible_projection = build_three_way_visible_projection(
            1,
            std::slice::from_ref(&visible_range),
            &marker_segments,
            false,
        );
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
        let visible_range = 0..1;
        let three_way_visible_projection = build_three_way_visible_projection(
            1,
            std::slice::from_ref(&visible_range),
            &marker_segments,
            true,
        );

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
        let conflict_ranges = 0..1;
        let three_way_visible_projection = build_three_way_visible_projection(
            1,
            std::slice::from_ref(&conflict_ranges),
            &marker_segments,
            false,
        );

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
        let conflict_ranges = 1..3; // lines 1..3 are the conflict block

        let base_text = "header\nbase_needle\nbase_plain\nfooter\n";
        let ours_text = "header\nours_plain\nours_needle\nfooter\n";
        let theirs_text = "header\ntheirs_plain\ntheirs_plain\nfooter\n";
        let base_line_starts = vec![0, 7, 19, 30];
        let ours_line_starts = vec![0, 7, 18, 30];
        let theirs_line_starts = vec![0, 7, 21, 35];

        let projection = build_three_way_visible_projection(
            three_way_len,
            std::slice::from_ref(&conflict_ranges),
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
        let matching_rows =
            index.search_ascii_case_insensitive_matching_rows(&marker_segments, query.as_bytes());
        let mut source_text_matches: Vec<usize> = matching_rows
            .into_iter()
            .filter_map(|r| proj.source_to_visible(r))
            .collect();
        source_text_matches.sort_unstable();

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
        let conflict_ranges = 0..1;
        let projection = build_three_way_visible_projection(
            1,
            std::slice::from_ref(&conflict_ranges),
            &marker_segments,
            true,
        );

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
