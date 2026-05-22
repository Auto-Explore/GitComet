use super::*;
use gitcomet_core::domain::Diff;
use memchr::{memchr_iter, memchr2_iter};
use regex::{Regex, RegexBuilder};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::ops::Range;
use std::time::Duration;

const FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES: usize = 32 * 1024;
const FILE_PREVIEW_REGEX_SEARCH_WINDOW_BYTES: usize = 256 * 1024;
const FILE_PREVIEW_REGEX_SEARCH_KEEP_BYTES: usize = 64 * 1024;
const DIFF_SEARCH_QUERY_DEBOUNCE_MS: u64 = 150;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(in crate::view) struct DiffSearchOptions {
    pub(in crate::view) match_case: bool,
    pub(in crate::view) whole_word: bool,
    pub(in crate::view) regex: bool,
}

pub(in crate::view) fn normalize_diff_search_query(query: &str) -> Cow<'_, str> {
    if !query.contains('\r') {
        return Cow::Borrowed(query);
    }
    Cow::Owned(query.replace("\r\n", "\n").replace('\r', "\n"))
}

pub(in crate::view) struct DiffSearchMatcher {
    query: String,
    options: DiffSearchOptions,
    regex: Option<Regex>,
    regex_error: Option<String>,
}

impl DiffSearchMatcher {
    pub(in crate::view) fn new(query: &str, options: DiffSearchOptions) -> Self {
        let query = normalize_diff_search_query(query).into_owned();
        let (regex, regex_error) = if options.regex && !query.is_empty() {
            match RegexBuilder::new(&query)
                .case_insensitive(!options.match_case)
                .multi_line(true)
                .build()
            {
                Ok(regex) => (Some(regex), None),
                Err(err) => (None, Some(err.to_string())),
            }
        } else {
            (None, None)
        };

        Self {
            query,
            options,
            regex,
            regex_error,
        }
    }

    pub(in crate::view) fn query(&self) -> &str {
        self.query.as_str()
    }

    pub(in crate::view) fn regex_error(&self) -> Option<&str> {
        self.regex_error.as_deref()
    }

    pub(in crate::view) fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    pub(in crate::view) fn can_use_ascii_case_insensitive_fast_path(&self) -> bool {
        !self.options.match_case
            && !self.options.whole_word
            && !self.options.regex
            && !self.query.contains('\n')
    }

    #[cfg(test)]
    pub(in crate::view) fn is_match(&self, haystack: &str) -> bool {
        self.find_range_at_or_after(haystack, 0).is_some()
    }

    pub(in crate::view) fn find_ranges_into(
        &self,
        haystack: &str,
        out: &mut Vec<Range<usize>>,
        max_matches: usize,
    ) {
        out.clear();
        if max_matches == 0 || self.is_empty() || self.regex_error.is_some() {
            return;
        }

        let mut search_start = 0usize;
        while out.len() < max_matches {
            let Some(range) = self.find_range_at_or_after(haystack, search_start) else {
                break;
            };
            search_start = range.end;
            out.push(range);
        }
    }

    fn find_literal_case_sensitive_from(
        &self,
        haystack: &str,
        start_at: usize,
    ) -> Option<Range<usize>> {
        let needle = self.query.as_bytes();
        let haystack_bytes = haystack.as_bytes();
        let (&first, _) = needle.first().zip(needle.last())?;
        let last_start = haystack_bytes.len().checked_sub(needle.len())?;
        let start_at = start_at.min(haystack_bytes.len());
        if start_at > last_start {
            return None;
        }

        for offset in memchr_iter(first, &haystack_bytes[start_at..=last_start]) {
            let start = start_at + offset;
            let range = start..(start + needle.len());
            if haystack_bytes.get(range.clone()) == Some(needle)
                && self.range_has_requested_boundaries(haystack, range.clone())
            {
                return Some(range);
            }
        }
        None
    }

    fn find_literal_ascii_case_insensitive_from(
        &self,
        haystack: &str,
        start_at: usize,
    ) -> Option<Range<usize>> {
        let needle = self.query.as_bytes();
        let haystack_bytes = haystack.as_bytes();
        let (&first, &last) = needle.first().zip(needle.last())?;
        let last_start = haystack_bytes.len().checked_sub(needle.len())?;
        let start_at = start_at.min(haystack_bytes.len());
        if start_at > last_start {
            return None;
        }
        let first_lower = first.to_ascii_lowercase();
        let first_upper = first.to_ascii_uppercase();

        if needle.len() == 1 {
            for offset in memchr2_iter(first_lower, first_upper, &haystack_bytes[start_at..]) {
                let start = start_at + offset;
                let range = start..(start + 1);
                if self.range_has_requested_boundaries(haystack, range.clone()) {
                    return Some(range);
                }
            }
            return None;
        }

        let middle = &needle[1..needle.len() - 1];
        let last_lower = last.to_ascii_lowercase();
        let last_upper = last.to_ascii_uppercase();
        for offset in memchr2_iter(
            first_lower,
            first_upper,
            &haystack_bytes[start_at..=last_start],
        ) {
            let start = start_at + offset;
            let haystack_last = haystack_bytes[start + needle.len() - 1];
            if haystack_last != last_lower && haystack_last != last_upper {
                continue;
            }
            if !haystack_bytes[start + 1..start + needle.len() - 1].eq_ignore_ascii_case(middle) {
                continue;
            }
            let range = start..(start + needle.len());
            if self.range_has_requested_boundaries(haystack, range.clone()) {
                return Some(range);
            }
        }
        None
    }

    pub(in crate::view) fn find_row_overlay_ranges_into(
        &self,
        haystack: &str,
        out: &mut Vec<Range<usize>>,
        max_matches: usize,
    ) {
        self.find_ranges_into(haystack, out, max_matches);
    }

    fn find_range_at_or_after(&self, haystack: &str, start_at: usize) -> Option<Range<usize>> {
        if self.is_empty() || self.regex_error.is_some() {
            return None;
        }

        if let Some(regex) = self.regex.as_ref() {
            let mut search_start = start_at.min(haystack.len());
            loop {
                let m = regex.find_at(haystack, search_start)?;
                let range = m.start()..m.end();
                if !range.is_empty() && self.range_has_requested_boundaries(haystack, range.clone())
                {
                    return Some(range);
                }
                search_start = next_char_boundary_after(haystack, m.start())?;
            }
        }

        if self.options.match_case {
            self.find_literal_case_sensitive_from(haystack, start_at)
        } else {
            self.find_literal_ascii_case_insensitive_from(haystack, start_at)
        }
    }

    fn range_has_requested_boundaries(&self, haystack: &str, range: Range<usize>) -> bool {
        if !self.options.whole_word {
            return true;
        }

        let bytes = haystack.as_bytes();
        let before = range
            .start
            .checked_sub(1)
            .and_then(|ix| bytes.get(ix))
            .copied();
        let after = bytes.get(range.end).copied();
        !before.is_some_and(is_ascii_word_byte) && !after.is_some_and(is_ascii_word_byte)
    }
}

#[inline]
fn is_ascii_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn next_char_boundary_after(s: &str, ix: usize) -> Option<usize> {
    if ix >= s.len() {
        return None;
    }

    Some(ix + s[ix..].chars().next()?.len_utf8())
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffSearchFinalizeMode {
    ScrollToFirst,
    PreserveCurrent {
        previous_match_ix: Option<usize>,
        previous_visible_ix: Option<usize>,
    },
}

impl DiffSearchFinalizeMode {
    fn preserve_current(
        previous_match_ix: Option<usize>,
        previous_visible_ix: Option<usize>,
    ) -> Self {
        Self::PreserveCurrent {
            previous_match_ix,
            previous_visible_ix,
        }
    }
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

fn expand_tabs_to_string(text: &str) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }

    let mut expanded = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => expanded.push_str("    "),
            _ => expanded.push(ch),
        }
    }
    expanded
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
    let previous_query = normalize_diff_search_query(previous_query);
    let next_query = normalize_diff_search_query(next_query);
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

fn diff_search_resume_match_ix(
    previous_visible_ix: Option<usize>,
    matches: &[usize],
) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }

    let Some(previous_visible_ix) = previous_visible_ix else {
        return Some(0);
    };

    match matches.binary_search(&previous_visible_ix) {
        Ok(ix) => Some(ix),
        Err(insert_ix) => Some(insert_ix.checked_sub(1).unwrap_or(matches.len() - 1)),
    }
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
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    query: AsciiCaseInsensitiveNeedle<'_>,
) -> bool {
    if raw_text.len() <= FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES {
        return query.is_match(raw_text.as_ref());
    }

    let overlap = query.as_bytes().len().saturating_sub(1);
    let mut chunk_start = 0usize;
    while chunk_start < raw_text.len() {
        let scan_start = chunk_start.saturating_sub(overlap);
        let scan_end = chunk_start
            .saturating_add(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES)
            .min(raw_text.len());
        let slice = raw_text
            .slice_text(scan_start..scan_end)
            .unwrap_or_default();
        if query.is_match(slice.as_ref()) {
            return true;
        }
        if scan_end >= raw_text.len() {
            break;
        }
        chunk_start = scan_end;
    }

    false
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

fn normalized_stream_row_text(row_text: &str) -> &str {
    row_text.strip_suffix('\r').unwrap_or(row_text)
}

fn normalized_file_diff_line_text_len(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
) -> usize {
    let len = raw_text.len();
    if len == 0 {
        return 0;
    }

    if raw_text
        .slice_bytes(len - 1..len)
        .is_some_and(|bytes| bytes.as_ref() == b"\r")
    {
        len - 1
    } else {
        len
    }
}

#[inline]
fn folded_search_byte(byte: u8, match_case: bool) -> u8 {
    if match_case {
        byte
    } else {
        byte.to_ascii_lowercase()
    }
}

fn build_literal_search_prefix_table(needle: &[u8]) -> Vec<usize> {
    let mut prefix = vec![0; needle.len()];
    let mut matched = 0usize;

    for ix in 1..needle.len() {
        while matched > 0 && needle[ix] != needle[matched] {
            matched = prefix[matched - 1];
        }
        if needle[ix] == needle[matched] {
            matched += 1;
            prefix[ix] = matched;
        }
    }

    prefix
}

fn visible_ix_for_stream_abs(row_starts: &[(usize, usize)], stream_abs: usize) -> Option<usize> {
    if row_starts.is_empty() {
        return None;
    }

    let row_ix = match row_starts.binary_search_by_key(&stream_abs, |(start, _)| *start) {
        Ok(ix) => ix,
        Err(ix) => ix.saturating_sub(1),
    };
    row_starts.get(row_ix).map(|(_, visible_ix)| *visible_ix)
}

fn next_row_start_after_stream_abs(
    row_starts: &[(usize, usize)],
    stream_abs: usize,
) -> Option<usize> {
    let ix = row_starts.partition_point(|(start, _)| *start <= stream_abs);
    row_starts.get(ix).map(|(start, _)| *start)
}

struct LiteralFileDiffLineTextStreamSearch {
    needle: Vec<u8>,
    prefix: Vec<usize>,
    match_case: bool,
    whole_word: bool,
    matched: usize,
    stream_abs: usize,
    recent_bytes: VecDeque<u8>,
    pending_whole_word_start: Option<usize>,
    row_starts: Vec<(usize, usize)>,
    last_reported_visible_ix: Option<usize>,
}

impl LiteralFileDiffLineTextStreamSearch {
    fn new(matcher: &DiffSearchMatcher) -> Option<Self> {
        if matcher.options.regex || matcher.query.is_empty() {
            return None;
        }

        let needle = matcher
            .query
            .as_bytes()
            .iter()
            .copied()
            .map(|byte| folded_search_byte(byte, matcher.options.match_case))
            .collect::<Vec<_>>();
        let prefix = build_literal_search_prefix_table(needle.as_slice());

        Some(Self {
            needle,
            prefix,
            match_case: matcher.options.match_case,
            whole_word: matcher.options.whole_word,
            matched: 0,
            stream_abs: 0,
            recent_bytes: VecDeque::with_capacity(matcher.query.len().saturating_add(1)),
            pending_whole_word_start: None,
            row_starts: Vec::new(),
            last_reported_visible_ix: None,
        })
    }

    fn has_rows(&self) -> bool {
        !self.row_starts.is_empty()
    }

    fn push_row_start(&mut self, visible_ix: usize) {
        self.row_starts.push((self.stream_abs, visible_ix));
    }

    fn row_reported(&self, visible_ix: usize) -> bool {
        self.last_reported_visible_ix == Some(visible_ix)
    }

    fn report_match_start(&mut self, match_start: usize, out: &mut Vec<usize>) {
        let Some(visible_ix) = visible_ix_for_stream_abs(self.row_starts.as_slice(), match_start)
        else {
            return;
        };
        if self.last_reported_visible_ix == Some(visible_ix) {
            return;
        }
        out.push(visible_ix);
        self.last_reported_visible_ix = Some(visible_ix);
    }

    fn finish_pending_whole_word(&mut self, after: Option<u8>, out: &mut Vec<usize>) {
        let Some(match_start) = self.pending_whole_word_start.take() else {
            return;
        };
        if !after.is_some_and(is_ascii_word_byte) {
            self.report_match_start(match_start, out);
        }
    }

    fn push_byte(&mut self, byte: u8, out: &mut Vec<usize>) {
        self.finish_pending_whole_word(Some(byte), out);

        let folded = folded_search_byte(byte, self.match_case);
        while self.matched > 0 && folded != self.needle[self.matched] {
            self.matched = self.prefix[self.matched - 1];
        }
        if folded == self.needle[self.matched] {
            self.matched += 1;
        }

        self.recent_bytes.push_back(byte);
        while self.recent_bytes.len() > self.needle.len().saturating_add(1) {
            self.recent_bytes.pop_front();
        }

        if self.matched == self.needle.len() {
            let match_end = self.stream_abs.saturating_add(1);
            let match_start = match_end.saturating_sub(self.needle.len());
            let before = if match_start == 0 {
                None
            } else {
                self.recent_bytes
                    .get(
                        self.recent_bytes
                            .len()
                            .saturating_sub(self.needle.len() + 1),
                    )
                    .copied()
            };

            if !self.whole_word || !before.is_some_and(is_ascii_word_byte) {
                if self.whole_word {
                    self.pending_whole_word_start = Some(match_start);
                } else {
                    self.report_match_start(match_start, out);
                }
            }

            self.matched = self.prefix[self.needle.len() - 1];
        }

        self.stream_abs = self.stream_abs.saturating_add(1);
    }

    fn push_bytes(&mut self, bytes: &[u8], out: &mut Vec<usize>) {
        for &byte in bytes {
            self.push_byte(byte, out);
        }
    }

    fn skip_bytes_until_next_row(&mut self, byte_count: usize) {
        self.stream_abs = self.stream_abs.saturating_add(byte_count);
        self.matched = 0;
        self.pending_whole_word_start = None;
        self.recent_bytes.clear();
    }

    fn finish(&mut self, out: &mut Vec<usize>) {
        self.finish_pending_whole_word(None, out);
    }
}

fn collect_file_diff_line_text_literal_stream_match_visible_rows(
    rows: impl IntoIterator<Item = (usize, gitcomet_core::file_diff::FileDiffLineText)>,
    matcher: &DiffSearchMatcher,
    out: &mut Vec<usize>,
) {
    let Some(mut search) = LiteralFileDiffLineTextStreamSearch::new(matcher) else {
        return;
    };

    for (visible_ix, raw_text) in rows {
        if search.has_rows() {
            search.push_byte(b'\n', out);
        }
        search.push_row_start(visible_ix);

        let row_len = normalized_file_diff_line_text_len(&raw_text);
        let mut chunk_start = 0usize;
        while chunk_start < row_len {
            let chunk_end = chunk_start
                .saturating_add(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES)
                .min(row_len);
            if let Some(bytes) = raw_text.slice_bytes(chunk_start..chunk_end) {
                search.push_bytes(bytes.as_ref(), out);
            } else {
                search.skip_bytes_until_next_row(chunk_end.saturating_sub(chunk_start));
            }
            chunk_start = chunk_end;

            if search.row_reported(visible_ix) {
                search.skip_bytes_until_next_row(row_len.saturating_sub(chunk_start));
                break;
            }
        }
    }

    search.finish(out);
}

struct RegexFileDiffLineTextStreamSearch<'a> {
    matcher: &'a DiffSearchMatcher,
    window: String,
    window_start_abs: usize,
    stream_abs: usize,
    row_starts: Vec<(usize, usize)>,
    last_reported_visible_ix: Option<usize>,
}

impl<'a> RegexFileDiffLineTextStreamSearch<'a> {
    fn new(matcher: &'a DiffSearchMatcher) -> Self {
        Self {
            matcher,
            window: String::with_capacity(FILE_PREVIEW_REGEX_SEARCH_KEEP_BYTES),
            window_start_abs: 0,
            stream_abs: 0,
            row_starts: Vec::new(),
            last_reported_visible_ix: None,
        }
    }

    fn has_rows(&self) -> bool {
        !self.row_starts.is_empty()
    }

    fn push_row_start(&mut self, visible_ix: usize) {
        self.row_starts.push((self.stream_abs, visible_ix));
    }

    fn row_reported(&self, visible_ix: usize) -> bool {
        self.last_reported_visible_ix == Some(visible_ix)
    }

    fn report_match_start(&mut self, match_start: usize, out: &mut Vec<usize>) {
        let Some(visible_ix) = visible_ix_for_stream_abs(self.row_starts.as_slice(), match_start)
        else {
            return;
        };
        if self.last_reported_visible_ix == Some(visible_ix) {
            return;
        }
        out.push(visible_ix);
        self.last_reported_visible_ix = Some(visible_ix);
    }

    fn scan_window(&mut self, stream_finished: bool, out: &mut Vec<usize>) {
        let mut search_start = 0usize;
        while search_start < self.window.len() {
            let Some(range) = self
                .matcher
                .find_range_at_or_after(self.window.as_str(), search_start)
            else {
                break;
            };

            let has_real_before = range.start > 0 || self.window_start_abs == 0;
            let has_real_after = range.end < self.window.len() || stream_finished;
            let match_start_abs = self.window_start_abs.saturating_add(range.start);
            if has_real_before && has_real_after {
                self.report_match_start(match_start_abs, out);
            }

            let next_row_start =
                next_row_start_after_stream_abs(self.row_starts.as_slice(), match_start_abs)
                    .unwrap_or_else(|| self.window_start_abs.saturating_add(self.window.len()));
            let next_search_start = next_row_start
                .saturating_sub(self.window_start_abs)
                .min(self.window.len());
            if next_search_start > range.start {
                search_start = next_search_start;
            } else {
                search_start = next_char_boundary_after(self.window.as_str(), range.start)
                    .unwrap_or(self.window.len());
            }
        }
    }

    fn trim_window(&mut self) {
        if self.window.len() <= FILE_PREVIEW_REGEX_SEARCH_KEEP_BYTES {
            return;
        }

        let mut drop_len = self
            .window
            .len()
            .saturating_sub(FILE_PREVIEW_REGEX_SEARCH_KEEP_BYTES);
        while drop_len > 0 && !self.window.is_char_boundary(drop_len) {
            drop_len -= 1;
        }
        if drop_len == 0 {
            return;
        }

        self.window.drain(..drop_len);
        self.window_start_abs = self.window_start_abs.saturating_add(drop_len);
    }

    fn push_str(&mut self, text: &str, out: &mut Vec<usize>) {
        self.window.push_str(text);
        self.stream_abs = self.stream_abs.saturating_add(text.len());
        if self.window.len() >= FILE_PREVIEW_REGEX_SEARCH_WINDOW_BYTES {
            self.scan_window(false, out);
            self.trim_window();
        }
    }

    fn skip_bytes_until_next_row(&mut self, byte_count: usize) {
        self.stream_abs = self.stream_abs.saturating_add(byte_count);
        self.window.clear();
        self.window_start_abs = self.stream_abs;
    }

    fn finish(&mut self, out: &mut Vec<usize>) {
        self.scan_window(true, out);
    }
}

fn collect_file_diff_line_text_regex_stream_match_visible_rows(
    rows: impl IntoIterator<Item = (usize, gitcomet_core::file_diff::FileDiffLineText)>,
    matcher: &DiffSearchMatcher,
    out: &mut Vec<usize>,
) {
    let mut search = RegexFileDiffLineTextStreamSearch::new(matcher);

    for (visible_ix, raw_text) in rows {
        if search.has_rows() {
            search.push_str("\n", out);
        }
        search.push_row_start(visible_ix);

        let row_len = normalized_file_diff_line_text_len(&raw_text);
        let mut chunk_start = 0usize;
        while chunk_start < row_len {
            let requested_end = chunk_start
                .saturating_add(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES)
                .min(row_len);
            let Some((text, resolved_range)) =
                raw_text.slice_text_resolved(chunk_start..requested_end)
            else {
                break;
            };
            if !text.is_empty() {
                search.push_str(text.as_ref(), out);
            }

            if resolved_range.end > chunk_start {
                chunk_start = resolved_range.end;
            } else {
                chunk_start = requested_end;
            }

            if search.row_reported(visible_ix) {
                search.skip_bytes_until_next_row(row_len.saturating_sub(chunk_start));
                break;
            }
        }
    }

    search.finish(out);
}

fn collect_file_diff_line_text_stream_match_visible_rows(
    rows: impl IntoIterator<Item = (usize, gitcomet_core::file_diff::FileDiffLineText)>,
    matcher: &DiffSearchMatcher,
    out: &mut Vec<usize>,
) {
    if matcher.options.regex {
        collect_file_diff_line_text_regex_stream_match_visible_rows(rows, matcher, out);
    } else {
        collect_file_diff_line_text_literal_stream_match_visible_rows(rows, matcher, out);
    }
}

fn collect_stream_match_visible_rows<'a>(
    rows: impl IntoIterator<Item = (usize, Cow<'a, str>)>,
    matcher: &DiffSearchMatcher,
    out: &mut Vec<usize>,
) {
    let mut text = String::new();
    let mut line_starts = Vec::new();
    let mut visible_indices = Vec::new();

    for (visible_ix, row_text) in rows {
        if !visible_indices.is_empty() {
            text.push('\n');
        }
        line_starts.push(text.len());
        visible_indices.push(visible_ix);
        text.push_str(normalized_stream_row_text(row_text.as_ref()));
    }

    if visible_indices.is_empty() {
        return;
    }

    let mut search_start = 0usize;
    while let Some(range) = matcher.find_range_at_or_after(&text, search_start) {
        let start = range.start.min(text.len());
        let line_ix = match line_starts.binary_search(&start) {
            Ok(ix) => ix,
            Err(ix) => ix.saturating_sub(1),
        };
        if let Some(visible_ix) = visible_indices.get(line_ix).copied() {
            out.push(visible_ix);
        }

        search_start = line_starts.get(line_ix + 1).copied().unwrap_or(text.len());
        if search_start <= start {
            break;
        }
    }
}

fn collect_split_stream_match_visible_rows<'a>(
    rows: impl IntoIterator<Item = (usize, Option<Cow<'a, str>>, Option<Cow<'a, str>>)>,
    matcher: &DiffSearchMatcher,
    out: &mut Vec<usize>,
) {
    let mut left_rows = Vec::new();
    let mut right_rows = Vec::new();

    for (visible_ix, left, right) in rows {
        left_rows.push((visible_ix, left.unwrap_or(Cow::Borrowed(""))));
        right_rows.push((visible_ix, right.unwrap_or(Cow::Borrowed(""))));
    }

    collect_stream_match_visible_rows(left_rows, matcher, out);
    collect_stream_match_visible_rows(right_rows, matcher, out);
}

impl MainPaneView {
    pub(in crate::view) fn active_conflict_target(
        &self,
    ) -> Option<(
        std::path::PathBuf,
        Option<gitcomet_core::domain::FileConflictKind>,
    )> {
        if self.is_inline_submodule_diff_active() {
            return None;
        }
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()? else {
            return None;
        };
        if *area != DiffArea::Unstaged {
            return None;
        }
        let conflict = repo
            .status_entry_for_path(DiffArea::Unstaged, path.as_path())
            .filter(|entry| entry.kind == FileStatusKind::Conflicted)?;

        Some((path.clone(), conflict.conflict))
    }

    pub(in crate::view) fn diff_search_options_or_default(&self) -> DiffSearchOptions {
        if self.diff_search_active {
            self.diff_search_options
        } else {
            DiffSearchOptions::default()
        }
    }

    pub(in crate::view) fn diff_search_has_query(&self) -> bool {
        self.diff_search_active && !self.diff_search_query.as_ref().is_empty()
    }

    fn diff_search_current_matcher(&mut self) -> DiffSearchMatcher {
        let matcher = DiffSearchMatcher::new(
            self.diff_search_query.as_ref(),
            self.diff_search_options_or_default(),
        );
        self.diff_search_regex_error = matcher.regex_error().map(|err| err.to_string().into());
        matcher
    }

    pub(in super::super::super) fn diff_search_recompute_matches(&mut self) {
        self.diff_search_cancel_pending_query_recompute();
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

    pub(in super::super::super) fn diff_search_recompute_matches_and_scroll_to_first(&mut self) {
        self.diff_search_cancel_pending_query_recompute();
        self.diff_search_recompute_matches_with_finalize(DiffSearchFinalizeMode::ScrollToFirst);
    }

    fn diff_search_recompute_matches_with_finalize(&mut self, finalize: DiffSearchFinalizeMode) {
        if !self.diff_search_active {
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        if !self.is_file_preview_active() && self.active_conflict_target().is_none() {
            self.ensure_diff_visible_indices();
        }

        self.diff_search_recompute_matches_for_current_view_with_finalize(finalize);
    }

    pub(in super::super::super) fn diff_search_recompute_matches_preserving_current(&mut self) {
        self.diff_search_cancel_pending_query_recompute();
        if !self.diff_search_active {
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        if !self.is_file_preview_active() && self.active_conflict_target().is_none() {
            self.ensure_diff_visible_indices();
        }

        self.diff_search_recompute_matches_for_current_view_preserving_current();
    }

    pub(super) fn diff_search_recompute_matches_for_query_change(&mut self, previous_query: &str) {
        if !self.diff_search_active {
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        self.diff_search_match_ix = None;
        let matcher = self.diff_search_current_matcher();

        if matcher.is_empty() || matcher.regex_error().is_some() {
            self.diff_search_matches.clear();
            self.diff_search_finalize_matches(DiffSearchFinalizeMode::ScrollToFirst);
            return;
        }

        let can_refine = matcher.can_use_ascii_case_insensitive_fast_path()
            && self.diff_search_can_refine_current_matches();

        match diff_search_query_reuse(previous_query, matcher.query()) {
            DiffSearchQueryReuse::SameSemantics
                if matcher.can_use_ascii_case_insensitive_fast_path() => {}
            DiffSearchQueryReuse::Refinement if can_refine => {
                let Some(query) = AsciiCaseInsensitiveNeedle::new(matcher.query()) else {
                    self.diff_search_matches.clear();
                    self.diff_search_finalize_matches(DiffSearchFinalizeMode::ScrollToFirst);
                    return;
                };
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
            DiffSearchQueryReuse::SameSemantics
            | DiffSearchQueryReuse::None
            | DiffSearchQueryReuse::Refinement => {
                self.diff_search_scan_current_view_with_matcher(&matcher);
            }
        }

        self.diff_search_finalize_matches(DiffSearchFinalizeMode::ScrollToFirst);
    }

    pub(in super::super::super) fn diff_search_cancel_pending_query_recompute(&mut self) {
        self.diff_search_debounce_seq = self.diff_search_debounce_seq.wrapping_add(1);
        self.diff_search_pending_previous_query = None;
    }

    pub(super) fn diff_search_schedule_query_recompute(
        &mut self,
        previous_query: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.diff_search_active {
            self.diff_search_cancel_pending_query_recompute();
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        if self.diff_search_pending_previous_query.is_none() {
            self.diff_search_pending_previous_query = Some(previous_query);
        }
        self.diff_search_debounce_seq = self.diff_search_debounce_seq.wrapping_add(1);
        let seq = self.diff_search_debounce_seq;

        cx.spawn(
            async move |view: gpui::WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(DIFF_SEARCH_QUERY_DEBOUNCE_MS))
                    .await;
                let _ = view.update(cx, |this, cx| {
                    if this.diff_search_debounce_seq != seq {
                        return;
                    }
                    if this.diff_search_flush_pending_query_recompute() {
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    pub(super) fn diff_search_flush_pending_query_recompute(&mut self) -> bool {
        let Some(previous_query) = self.diff_search_pending_previous_query.take() else {
            return false;
        };

        self.diff_search_debounce_seq = self.diff_search_debounce_seq.wrapping_add(1);
        self.diff_search_recompute_matches_for_query_change(previous_query.as_ref());
        true
    }

    pub(super) fn diff_search_recompute_matches_for_current_view(&mut self) {
        let previous_match_ix = self.diff_search_match_ix;
        let previous_visible_ix =
            previous_match_ix.and_then(|ix| self.diff_search_matches.get(ix).copied());
        self.diff_search_recompute_matches_for_current_view_with_finalize(
            DiffSearchFinalizeMode::preserve_current(previous_match_ix, previous_visible_ix),
        );
    }

    pub(super) fn diff_search_recompute_matches_for_current_view_preserving_current(&mut self) {
        let previous_match_ix = self.diff_search_match_ix;
        let previous_visible_ix = self.diff_search_current_match_visible_ix();
        self.diff_search_recompute_matches_for_current_view_with_finalize(
            DiffSearchFinalizeMode::preserve_current(previous_match_ix, previous_visible_ix),
        );
    }

    fn diff_search_recompute_matches_for_current_view_with_finalize(
        &mut self,
        finalize: DiffSearchFinalizeMode,
    ) {
        let matcher = self.diff_search_current_matcher();

        if matcher.is_empty() || matcher.regex_error().is_some() {
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        self.diff_search_scan_current_view_with_matcher(&matcher);
        self.diff_search_finalize_matches(finalize);
    }

    fn diff_search_scan_current_view_with_matcher(&mut self, matcher: &DiffSearchMatcher) {
        if matcher.can_use_ascii_case_insensitive_fast_path()
            && let Some(query) = AsciiCaseInsensitiveNeedle::new(matcher.query())
        {
            self.diff_search_scan_current_view_with_needle(query);
            return;
        }

        self.diff_search_scan_current_view_general(matcher);
    }

    fn diff_search_scan_current_view_with_needle(&mut self, query: AsciiCaseInsensitiveNeedle<'_>) {
        self.diff_search_matches.clear();

        if self.is_file_preview_active() {
            let Some(line_count) = self.worktree_preview_line_count() else {
                return;
            };
            if let Some(index) = self.worktree_preview_search_trigram_index.as_ref() {
                match index.candidates(query.as_bytes()) {
                    DiffSearchVisibleCandidates::None => {}
                    DiffSearchVisibleCandidates::All => {
                        for ix in 0..line_count {
                            if self.worktree_preview_line_raw_text(ix).is_some_and(|line| {
                                resolved_output_line_ix_matches_query(&line, query)
                            }) {
                                self.diff_search_matches.push(ix);
                            }
                        }
                    }
                    DiffSearchVisibleCandidates::Indexed(candidate_rows) => {
                        for &ix in candidate_rows {
                            let ix = ix as usize;
                            if self.worktree_preview_line_raw_text(ix).is_some_and(|line| {
                                resolved_output_line_ix_matches_query(&line, query)
                            }) {
                                self.diff_search_matches.push(ix);
                            }
                        }
                    }
                }
            } else {
                for ix in 0..line_count {
                    if self
                        .worktree_preview_line_raw_text(ix)
                        .is_some_and(|line| resolved_output_line_ix_matches_query(&line, query))
                    {
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
                && !self.is_collapsed_diff_projection_active()
                && self.diff_search_scan_inline_patch_diff_with_needle(query)
            {
                return;
            }

            let total = self.diff_visible_len();
            if self.diff_view == DiffViewMode::Inline && self.is_file_diff_view_active() {
                for visible_ix in 0..total {
                    if self
                        .diff_search_file_diff_inline_visible_row_matches_query(query, visible_ix)
                    {
                        self.diff_search_matches.push(visible_ix);
                    }
                }
                return;
            }
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

    fn diff_search_scan_current_view_general(&mut self, matcher: &DiffSearchMatcher) {
        self.diff_search_matches.clear();

        if self.is_file_preview_active() {
            let Some(line_count) = self.worktree_preview_line_count() else {
                return;
            };
            if self.worktree_preview_source_len > 0 && self.worktree_preview_text.is_empty() {
                let mut matches = Vec::new();
                collect_file_diff_line_text_stream_match_visible_rows(
                    (0..line_count).filter_map(|ix| {
                        self.worktree_preview_line_raw_text(ix)
                            .map(|line| (ix, line))
                    }),
                    matcher,
                    &mut matches,
                );
                self.diff_search_matches = matches;
            } else {
                let rows: Vec<_> = (0..line_count)
                    .filter_map(|ix| {
                        self.worktree_preview_line_raw_text(ix)
                            .map(|line| (ix, Cow::Owned(line.as_ref().to_string())))
                    })
                    .collect();
                collect_stream_match_visible_rows(rows, matcher, &mut self.diff_search_matches);
            }
            self.diff_search_matches.sort_unstable();
            self.diff_search_matches.dedup();
            return;
        }

        if let Some((_path, conflict_kind)) = self.active_conflict_target() {
            if conflict_kind.is_some() || self.conflict_resolver.path.is_some() {
                let ctx =
                    ConflictResolverSearchContext::from_conflict_resolver(&self.conflict_resolver);
                self.diff_search_matches =
                    conflict_resolver_visible_match_indices_with_matcher(matcher, &ctx);
            }
            return;
        }

        if self.diff_view == DiffViewMode::Inline
            && !self.is_file_diff_view_active()
            && !self.is_collapsed_diff_projection_active()
            && self.diff_search_scan_inline_patch_diff_general(matcher)
        {
            return;
        }

        let total = self.diff_visible_len();
        if self.diff_view == DiffViewMode::Inline && self.is_file_diff_view_active() {
            let rows: Vec<_> = (0..total)
                .filter_map(|visible_ix| {
                    self.diff_mapped_ix_for_visible_ix(visible_ix)
                        .and_then(|mapped_ix| self.file_diff_inline_render_data(mapped_ix))
                        .map(|row| (visible_ix, Cow::Owned(row.text.as_ref().to_string())))
                })
                .collect();
            collect_stream_match_visible_rows(rows, matcher, &mut self.diff_search_matches);
            self.diff_search_matches.sort_unstable();
            self.diff_search_matches.dedup();
            return;
        }

        if self.diff_view == DiffViewMode::Split && self.is_file_diff_view_active() {
            let Some(provider) = self.file_diff_row_provider.as_ref() else {
                return;
            };
            let rows: Vec<_> = (0..total)
                .filter_map(|visible_ix| {
                    let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return None;
                    };
                    let Some((left, right)) = provider.split_row_texts(mapped_ix) else {
                        return None;
                    };
                    Some((
                        visible_ix,
                        left.map(|left| Cow::Owned(expand_tabs_to_string(left.as_ref()))),
                        right.map(|right| Cow::Owned(expand_tabs_to_string(right.as_ref()))),
                    ))
                })
                .collect();
            collect_split_stream_match_visible_rows(rows, matcher, &mut self.diff_search_matches);
            self.diff_search_matches.sort_unstable();
            self.diff_search_matches.dedup();
            return;
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                let rows: Vec<_> = (0..total)
                    .map(|visible_ix| {
                        let text =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                        (visible_ix, Cow::Owned(text.as_ref().to_string()))
                    })
                    .collect();
                collect_stream_match_visible_rows(rows, matcher, &mut self.diff_search_matches);
            }
            DiffViewMode::Split => {
                let left_rows: Vec<_> = (0..total)
                    .map(|visible_ix| {
                        let text =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitLeft);
                        (visible_ix, Cow::Owned(text.as_ref().to_string()))
                    })
                    .collect();
                collect_stream_match_visible_rows(
                    left_rows,
                    matcher,
                    &mut self.diff_search_matches,
                );

                let right_rows: Vec<_> = (0..total)
                    .map(|visible_ix| {
                        let text =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitRight);
                        (visible_ix, Cow::Owned(text.as_ref().to_string()))
                    })
                    .collect();
                collect_stream_match_visible_rows(
                    right_rows,
                    matcher,
                    &mut self.diff_search_matches,
                );
            }
        }
        self.diff_search_matches.sort_unstable();
        self.diff_search_matches.dedup();
    }

    fn diff_search_scan_inline_patch_diff_with_needle(
        &mut self,
        query: AsciiCaseInsensitiveNeedle<'_>,
    ) -> bool {
        let diff = match self.rendered_patch_diff_loadable() {
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

    fn diff_search_scan_inline_patch_diff_general(&mut self, matcher: &DiffSearchMatcher) -> bool {
        let diff = match self.rendered_patch_diff_loadable() {
            Some(Loadable::Ready(diff)) => Arc::clone(diff),
            _ => return false,
        };

        if let Some(map) = self.diff_visible_inline_map.as_ref() {
            let mut rows = Vec::new();
            map.for_each_visible_src_ix(|visible_ix, src_ix| {
                if let Some(text) = inline_patch_diff_search_text(
                    diff.as_ref(),
                    &self.diff_click_kinds,
                    &self.diff_header_display_cache,
                    src_ix,
                ) {
                    rows.push((visible_ix, Cow::Owned(text.as_ref().to_string())));
                }
            });
            collect_stream_match_visible_rows(rows, matcher, &mut self.diff_search_matches);
        } else {
            let rows: Vec<_> = self
                .diff_visible_indices
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(visible_ix, src_ix)| {
                    inline_patch_diff_search_text(
                        diff.as_ref(),
                        &self.diff_click_kinds,
                        &self.diff_header_display_cache,
                        src_ix,
                    )
                    .map(|text| (visible_ix, Cow::Owned(text.as_ref().to_string())))
                })
                .collect();
            collect_stream_match_visible_rows(rows, matcher, &mut self.diff_search_matches);
        }

        self.diff_search_matches.sort_unstable();
        self.diff_search_matches.dedup();
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
        diff_search_split_row_texts_match_query(
            query,
            left.as_ref().map(|text| text.as_ref()),
            right.as_ref().map(|text| text.as_ref()),
            expanded_tabs,
        )
    }

    fn diff_search_file_diff_inline_visible_row_matches_query(
        &self,
        query: AsciiCaseInsensitiveNeedle<'_>,
        visible_ix: usize,
    ) -> bool {
        if !self.is_file_diff_view_active() || self.diff_view != DiffViewMode::Inline {
            return false;
        }
        let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
            return false;
        };
        self.file_diff_inline_render_data(mapped_ix)
            .is_some_and(|row| query.is_match(row.text.as_ref()))
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
            || self.is_collapsed_diff_projection_active()
        {
            return false;
        }

        let Some(diff) = self.rendered_patch_diff_loadable() else {
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

        retain_refined_visible_matches(
            previous_matches,
            index.candidates(query.as_bytes()),
            |line_ix| {
                self.worktree_preview_line_raw_text(line_ix)
                    .is_some_and(|line| resolved_output_line_ix_matches_query(&line, query))
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
                .worktree_preview_line_raw_text(visible_ix)
                .is_some_and(|line| resolved_output_line_ix_matches_query(&line, query));
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                if self.is_file_diff_view_active() {
                    return self
                        .diff_search_file_diff_inline_visible_row_matches_query(query, visible_ix);
                }
                query.is_match(
                    self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline)
                        .as_ref(),
                )
            }
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

    fn diff_search_current_match_visible_ix(&self) -> Option<usize> {
        let len = self.diff_search_matches.len();
        if len == 0 {
            return None;
        }
        self.diff_search_match_ix
            .map(|ix| self.diff_search_matches[ix.min(len - 1)])
    }

    fn diff_search_finalize_matches(&mut self, mode: DiffSearchFinalizeMode) {
        if self.diff_search_matches.is_empty() {
            self.diff_search_match_ix = None;
            return;
        }

        match mode {
            DiffSearchFinalizeMode::ScrollToFirst => {
                self.diff_search_match_ix = Some(0);
                let first = self.diff_search_matches[0];
                self.diff_search_scroll_to_visible_ix(first);
            }
            DiffSearchFinalizeMode::PreserveCurrent {
                previous_match_ix,
                previous_visible_ix,
            } => {
                let had_previous_match =
                    previous_match_ix.is_some() || previous_visible_ix.is_some();
                let next_ix = previous_visible_ix
                    .and_then(|visible_ix| {
                        diff_search_resume_match_ix(Some(visible_ix), &self.diff_search_matches)
                    })
                    .or_else(|| {
                        previous_match_ix
                            .map(|ix| ix.min(self.diff_search_matches.len().saturating_sub(1)))
                    })
                    .unwrap_or(0);
                self.diff_search_match_ix = Some(next_ix);
                if !had_previous_match {
                    let first = self.diff_search_matches[next_ix];
                    self.diff_search_scroll_to_visible_ix(first);
                }
            }
        }
    }

    pub(in super::super::super) fn diff_search_prev_match(&mut self) {
        if !self.diff_search_active {
            return;
        }

        self.diff_search_flush_pending_query_recompute();
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

        self.diff_search_flush_pending_query_recompute();
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
        self.clear_diff_text_selection();
        self.diff_selection_range = None;

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

fn conflict_resolver_visible_match_indices_with_matcher(
    matcher: &DiffSearchMatcher,
    ctx: &ConflictResolverSearchContext<'_>,
) -> Vec<usize> {
    let mut out = Vec::new();
    match ctx.view_mode {
        ConflictResolverViewMode::ThreeWay => {
            let ConflictResolverSearchVisibleRows::Projection(projection) = ctx.three_way_visible;
            search_three_way_via_spans_with_matcher(projection, ctx, matcher, &mut out);
        }
        ConflictResolverViewMode::TwoWayDiff => {
            let ConflictResolverSearchTwoWayRows::Streamed {
                split_row_index,
                two_way_split_projection,
            } = ctx.two_way_rows;
            let rows = (0..two_way_split_projection.visible_len()).filter_map(|visible_ix| {
                let Some((source_row, _conflict_ix)) = two_way_split_projection.get(visible_ix)
                else {
                    return None;
                };
                let Some(row) = split_row_index.row_at(ctx.marker_segments, source_row) else {
                    return None;
                };
                Some((
                    visible_ix,
                    row.old
                        .as_ref()
                        .map(|text| Cow::Owned(text.as_ref().to_string())),
                    row.new
                        .as_ref()
                        .map(|text| Cow::Owned(text.as_ref().to_string())),
                ))
            });
            collect_split_stream_match_visible_rows(rows, matcher, &mut out);
        }
    }
    out.sort_unstable();
    out.dedup();
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

fn search_three_way_via_spans_with_matcher(
    projection: &conflict_resolver::ThreeWayVisibleProjection,
    ctx: &ConflictResolverSearchContext<'_>,
    matcher: &DiffSearchMatcher,
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

    let mut base_rows = Vec::new();
    let mut ours_rows = Vec::new();
    let mut theirs_rows = Vec::new();
    let mut summary_rows = Vec::new();

    for span in projection.spans() {
        match *span {
            conflict_resolver::ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } => {
                for i in 0..len {
                    let visible_ix = visible_start + i;
                    let line_ix = source_line_start + i;
                    base_rows.push((
                        visible_ix,
                        Cow::Borrowed(line_text(
                            ctx.three_way_base_text,
                            ctx.three_way_base_line_starts,
                            line_ix,
                        )),
                    ));
                    ours_rows.push((
                        visible_ix,
                        Cow::Borrowed(line_text(
                            ctx.three_way_ours_text,
                            ctx.three_way_ours_line_starts,
                            line_ix,
                        )),
                    ));
                    theirs_rows.push((
                        visible_ix,
                        Cow::Borrowed(line_text(
                            ctx.three_way_theirs_text,
                            ctx.three_way_theirs_line_starts,
                            line_ix,
                        )),
                    ));
                }
            }
            conflict_resolver::ThreeWayVisibleSpan::CollapsedResolvedBlock {
                visible_index,
                conflict_ix,
            } => {
                let choice_label = conflict_choice_for_index(ctx.marker_segments, conflict_ix)
                    .map(conflict_choice_label)
                    .unwrap_or("?");
                summary_rows.push((
                    visible_index,
                    Cow::Owned(format!("Resolved: picked {choice_label}")),
                ));
            }
        }
    }

    collect_stream_match_visible_rows(base_rows, matcher, out);
    collect_stream_match_visible_rows(ours_rows, matcher, out);
    collect_stream_match_visible_rows(theirs_rows, matcher, out);
    collect_stream_match_visible_rows(summary_rows, matcher, out);
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
        ConflictResolverSearchTwoWayRows, ConflictResolverSearchVisibleRows, DiffSearchMatcher,
        DiffSearchOptions, DiffSearchQueryReuse, FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES,
        collect_file_diff_line_text_stream_match_visible_rows,
        collect_split_stream_match_visible_rows, collect_stream_match_visible_rows,
        conflict_resolver_visible_match_indices,
        conflict_resolver_visible_match_indices_with_matcher, contains_ascii_case_insensitive,
        diff_search_query_reuse, diff_search_resume_match_ix,
        diff_search_split_row_texts_match_query, empty_conflict_resolver_search_two_way_rows,
        three_way_visible_item_matches_query,
    };
    use crate::perf_alloc::measure_allocations;
    use crate::view::conflict_resolver;
    use crate::view::conflict_resolver::{
        ConflictBlock, ConflictChoice, ConflictResolverViewMode, ConflictSegment,
        ConflictSplitRowIndex, TwoWaySplitProjection, build_three_way_visible_projection,
    };
    use crate::view::{
        ConflictModeState, ConflictResolverUiState, StreamedConflictState, ThreeWaySides,
    };
    use std::borrow::Cow;
    use std::io::Write;
    use std::sync::Arc;

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
            diff_search_query_reuse("Render_Cache", "render_cache"),
            DiffSearchQueryReuse::SameSemantics
        );
        assert_eq!(
            diff_search_query_reuse("Render_Cache", " render_cache "),
            DiffSearchQueryReuse::None
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
    fn diff_search_matcher_honors_case_sensitivity() {
        let default_matcher = DiffSearchMatcher::new("render", DiffSearchOptions::default());
        assert!(default_matcher.is_match("Render path"));

        let case_sensitive = DiffSearchMatcher::new(
            "render",
            DiffSearchOptions {
                match_case: true,
                ..DiffSearchOptions::default()
            },
        );
        assert!(!case_sensitive.is_match("Render path"));
        assert!(case_sensitive.is_match("render path"));
    }

    #[test]
    fn diff_search_matcher_honors_whole_word_boundaries() {
        let matcher = DiffSearchMatcher::new(
            "render",
            DiffSearchOptions {
                whole_word: true,
                ..DiffSearchOptions::default()
            },
        );

        assert!(matcher.is_match("render cache"));
        assert!(!matcher.is_match("prerender cache"));
        assert!(!matcher.is_match("render_cache"));
    }

    #[test]
    fn diff_search_matcher_handles_regex_and_invalid_regex() {
        let regex = DiffSearchMatcher::new(
            r"render\d+",
            DiffSearchOptions {
                regex: true,
                ..DiffSearchOptions::default()
            },
        );
        assert!(regex.regex_error().is_none());
        assert!(regex.is_match("RENDER42"));

        let invalid = DiffSearchMatcher::new(
            "(",
            DiffSearchOptions {
                regex: true,
                ..DiffSearchOptions::default()
            },
        );
        assert!(invalid.regex_error().is_some());
        assert!(!invalid.is_match("("));
    }

    #[test]
    fn diff_search_matcher_regex_anchors_match_visible_rows() {
        let start_anchor = DiffSearchMatcher::new(
            r"^use",
            DiffSearchOptions {
                regex: true,
                ..DiffSearchOptions::default()
            },
        );
        let mut matches = Vec::new();
        collect_stream_match_visible_rows(
            [
                (10, Cow::Borrowed("mod app;")),
                (11, Cow::Borrowed("use crate::app;")),
                (12, Cow::Borrowed("  use crate::other;")),
            ],
            &start_anchor,
            &mut matches,
        );
        assert_eq!(matches, vec![11]);

        let end_anchor = DiffSearchMatcher::new(
            r";$",
            DiffSearchOptions {
                regex: true,
                ..DiffSearchOptions::default()
            },
        );
        matches.clear();
        collect_stream_match_visible_rows(
            [
                (20, Cow::Borrowed("let x = 1")),
                (21, Cow::Borrowed("let y = 2;")),
                (22, Cow::Borrowed("let z = 3")),
            ],
            &end_anchor,
            &mut matches,
        );
        assert_eq!(matches, vec![21]);
    }

    #[test]
    fn diff_search_matcher_matches_across_adjacent_visible_rows() {
        let matcher = DiffSearchMatcher::new("alpha\nbeta", DiffSearchOptions::default());
        let mut matches = Vec::new();
        collect_stream_match_visible_rows(
            [
                (10, Cow::Borrowed("alpha")),
                (11, Cow::Borrowed("beta")),
                (12, Cow::Borrowed("gamma")),
            ],
            &matcher,
            &mut matches,
        );

        assert_eq!(matches, vec![10]);
    }

    #[test]
    fn diff_search_stream_collector_normalizes_crlf_row_endings() {
        let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());
        let mut matches = Vec::new();
        collect_stream_match_visible_rows(
            [
                (10, Cow::Borrowed("foo\r")),
                (11, Cow::Borrowed("bar\r")),
                (12, Cow::Borrowed("baz\r")),
            ],
            &matcher,
            &mut matches,
        );

        assert_eq!(matches, vec![10]);
    }

    #[test]
    fn diff_search_file_slice_stream_collector_normalizes_crlf_row_endings() {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        file.write_all(b"foo\r\nbar\r\nbaz\r\n")
            .expect("write temp file");
        let path = Arc::new(file.path().to_path_buf());
        let rows = [
            (
                10,
                gitcomet_core::file_diff::FileDiffLineText::file_slice(
                    Arc::clone(&path),
                    0..4,
                    true,
                    false,
                ),
            ),
            (
                11,
                gitcomet_core::file_diff::FileDiffLineText::file_slice(
                    Arc::clone(&path),
                    5..9,
                    true,
                    false,
                ),
            ),
            (
                12,
                gitcomet_core::file_diff::FileDiffLineText::file_slice(
                    Arc::clone(&path),
                    10..14,
                    true,
                    false,
                ),
            ),
        ];

        let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());
        let mut matches = Vec::new();
        collect_file_diff_line_text_stream_match_visible_rows(rows, &matcher, &mut matches);

        assert_eq!(matches, vec![10]);
    }

    #[test]
    fn diff_search_file_slice_general_collector_does_not_materialize_large_rows() {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        let payload_bytes = FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES * 128;
        let mut source = String::with_capacity(payload_bytes);
        source.push_str("needle ");
        source.push_str(&"x".repeat(payload_bytes));
        file.write_all(source.as_bytes()).expect("write temp file");
        let path = Arc::new(file.path().to_path_buf());

        let materialized = gitcomet_core::file_diff::FileDiffLineText::file_slice(
            Arc::clone(&path),
            0..source.len(),
            true,
            false,
        );
        let (_, materialized_metrics) = measure_allocations(|| {
            let copy = materialized.as_ref().to_string();
            let _ = std::hint::black_box(copy.as_str()).len();
        });

        let streamed = gitcomet_core::file_diff::FileDiffLineText::file_slice(
            Arc::clone(&path),
            0..source.len(),
            true,
            false,
        );
        let matcher = DiffSearchMatcher::new(
            "needle",
            DiffSearchOptions {
                whole_word: true,
                ..DiffSearchOptions::default()
            },
        );
        let mut matches = Vec::new();
        let (_, streamed_metrics) = measure_allocations(|| {
            collect_file_diff_line_text_stream_match_visible_rows(
                [(77, streamed)],
                &matcher,
                &mut matches,
            );
        });

        assert_eq!(matches, vec![77]);
        assert!(
            streamed_metrics.alloc_bytes.saturating_mul(8) < materialized_metrics.alloc_bytes,
            "streamed search should stay far below materializing the row: streamed={streamed_metrics:?} materialized={materialized_metrics:?}"
        );

        let regex_streamed = gitcomet_core::file_diff::FileDiffLineText::file_slice(
            Arc::clone(&path),
            0..source.len(),
            true,
            false,
        );
        let regex_matcher = DiffSearchMatcher::new(
            r"^needle",
            DiffSearchOptions {
                regex: true,
                ..DiffSearchOptions::default()
            },
        );
        let mut regex_matches = Vec::new();
        let (_, regex_streamed_metrics) = measure_allocations(|| {
            collect_file_diff_line_text_stream_match_visible_rows(
                [(78, regex_streamed)],
                &regex_matcher,
                &mut regex_matches,
            );
        });

        assert_eq!(regex_matches, vec![78]);
        assert!(
            regex_streamed_metrics.alloc_bytes.saturating_mul(2) < materialized_metrics.alloc_bytes,
            "regex streamed search should stay far below materializing the row: streamed={regex_streamed_metrics:?} materialized={materialized_metrics:?}"
        );
    }

    #[test]
    fn diff_search_matcher_does_not_match_multiline_fragments_on_non_adjacent_rows() {
        let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());
        let mut matches = Vec::new();
        collect_stream_match_visible_rows(
            [
                (10, Cow::Borrowed("foo")),
                (11, Cow::Borrowed("not the middle of the query")),
                (12, Cow::Borrowed("bar")),
            ],
            &matcher,
            &mut matches,
        );

        assert!(matches.is_empty());
    }

    #[test]
    fn diff_search_row_overlay_does_not_highlight_multiline_fragments() {
        let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());
        let mut ranges = Vec::new();

        matcher.find_row_overlay_ranges_into("foo", &mut ranges, 64);
        assert!(ranges.is_empty());

        matcher.find_row_overlay_ranges_into("bar", &mut ranges, 64);
        assert!(ranges.is_empty());
    }

    #[test]
    fn diff_search_stream_collector_reports_each_start_row_once() {
        let matcher = DiffSearchMatcher::new(
            "a",
            DiffSearchOptions {
                match_case: true,
                ..DiffSearchOptions::default()
            },
        );
        let mut matches = Vec::new();
        collect_stream_match_visible_rows(
            [
                (10, Cow::Borrowed("aaaa")),
                (11, Cow::Borrowed("bbbb")),
                (12, Cow::Borrowed("caca")),
            ],
            &matcher,
            &mut matches,
        );

        assert_eq!(matches, vec![10, 12]);
    }

    #[test]
    fn diff_search_stream_collector_keeps_empty_row_separators() {
        let matcher = DiffSearchMatcher::new("\nbar", DiffSearchOptions::default());
        let mut matches = Vec::new();
        collect_stream_match_visible_rows(
            [
                (10, Cow::Borrowed("")),
                (11, Cow::Borrowed("bar")),
                (12, Cow::Borrowed("baz")),
            ],
            &matcher,
            &mut matches,
        );

        assert_eq!(matches, vec![10]);
    }

    #[test]
    fn diff_search_split_stream_collector_preserves_missing_side_rows() {
        let mut matches = Vec::new();
        collect_split_stream_match_visible_rows(
            [
                (10, Some(Cow::Borrowed("foo")), Some(Cow::Borrowed("alpha"))),
                (11, None, Some(Cow::Borrowed("beta"))),
                (12, Some(Cow::Borrowed("bar")), Some(Cow::Borrowed("gamma"))),
            ],
            &DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default()),
            &mut matches,
        );
        assert!(
            matches.is_empty(),
            "a missing split side cell must not collapse out of the search stream"
        );

        collect_split_stream_match_visible_rows(
            [
                (10, Some(Cow::Borrowed("foo")), Some(Cow::Borrowed("alpha"))),
                (11, None, Some(Cow::Borrowed("beta"))),
                (12, Some(Cow::Borrowed("bar")), Some(Cow::Borrowed("gamma"))),
            ],
            &DiffSearchMatcher::new("foo\n\nbar", DiffSearchOptions::default()),
            &mut matches,
        );
        assert_eq!(matches, vec![10]);
    }

    #[test]
    fn diff_search_resume_keeps_exact_visible_match() {
        assert_eq!(diff_search_resume_match_ix(Some(20), &[4, 20, 30]), Some(1));
    }

    #[test]
    fn diff_search_resume_positions_before_next_later_match_when_previous_disappears() {
        let matches = [4, 30, 50];
        let ix = diff_search_resume_match_ix(Some(20), &matches).expect("resume ix");
        assert_eq!(ix, 0);
        assert_eq!(matches[(ix + 1) % matches.len()], 30);
    }

    #[test]
    fn diff_search_resume_wraps_when_no_later_match_remains() {
        let matches = [4, 12];
        let ix = diff_search_resume_match_ix(Some(20), &matches).expect("resume ix");
        assert_eq!(ix, 1);
        assert_eq!(matches[(ix + 1) % matches.len()], 4);
    }

    #[test]
    fn diff_search_resume_starts_at_first_without_previous_match() {
        assert_eq!(diff_search_resume_match_ix(None, &[4, 20, 30]), Some(0));
        assert_eq!(diff_search_resume_match_ix(Some(20), &[]), None);
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
    fn conflict_search_two_way_general_matches_across_adjacent_rows() {
        let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "foo\nbar\n".into(),
            theirs: "remote\nside\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let index = ConflictSplitRowIndex::new(&marker_segments, 1);
        let projection = TwoWaySplitProjection::new(&index, &marker_segments, false);
        let three_way_visible_projection =
            build_three_way_visible_projection(0, &[], &marker_segments, false);
        let ctx = ConflictResolverSearchContext {
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            marker_segments: &marker_segments,
            three_way_visible: ConflictResolverSearchVisibleRows::Projection(
                &three_way_visible_projection,
            ),
            three_way_base_text: "",
            three_way_base_line_starts: &[],
            three_way_ours_text: "",
            three_way_ours_line_starts: &[],
            three_way_theirs_text: "",
            three_way_theirs_line_starts: &[],
            two_way_rows: ConflictResolverSearchTwoWayRows::Streamed {
                split_row_index: &index,
                two_way_split_projection: &projection,
            },
        };
        let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());

        assert_eq!(
            conflict_resolver_visible_match_indices_with_matcher(&matcher, &ctx),
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
