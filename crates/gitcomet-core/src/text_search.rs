//! Query matching for the quick search bars (diff/file search, file filters).
//!
//! A [`TextSearchMatcher`] compiles a query once for a set of
//! [`TextSearchOptions`] (match case, whole word, regex) and then finds match
//! ranges in arbitrary haystacks. Literal queries use `memchr`-driven scans;
//! the case-insensitive literal path folds ASCII only. Regex queries are
//! compiled with `multi_line` so `^`/`$` anchor to individual lines.

use crate::services::CancellationToken;
use memchr::{memchr_iter, memchr2_iter};
use regex::{Regex, RegexBuilder};
use std::borrow::Cow;
use std::ops::Range;

/// The toggles offered next to a quick search input.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextSearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// Normalizes `\r\n` and lone `\r` in a query to `\n`, borrowing when the
/// query has no carriage returns.
pub fn normalize_text_search_query(query: &str) -> Cow<'_, str> {
    if !query.contains('\r') {
        return Cow::Borrowed(query);
    }
    Cow::Owned(query.replace("\r\n", "\n").replace('\r', "\n"))
}

/// A compiled search query.
pub struct TextSearchMatcher {
    query: String,
    options: TextSearchOptions,
    regex: Option<Regex>,
    regex_error: Option<String>,
    cancellation: Option<CancellationToken>,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    // Matchers built since the last take; a regex query compiles on each.
    static SEARCH_MATCHERS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Search matchers built on this thread since the last call, which resets the
/// count.
#[cfg(any(test, feature = "test-support"))]
pub fn take_text_search_matchers_built_for_tests() -> usize {
    SEARCH_MATCHERS_BUILT.with(|built| built.replace(0))
}

impl TextSearchMatcher {
    pub fn new(query: &str, options: TextSearchOptions) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        SEARCH_MATCHERS_BUILT.with(|built| built.set(built.get() + 1));
        let query = normalize_text_search_query(query).into_owned();
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
            cancellation: None,
        }
    }

    /// The normalized query (see [`normalize_text_search_query`]).
    pub fn query(&self) -> &str {
        self.query.as_str()
    }

    pub fn options(&self) -> TextSearchOptions {
        self.options
    }

    /// Makes every subsequent search stop early (reporting no further
    /// matches) once `token` is cancelled.
    pub fn set_cancellation(&mut self, token: CancellationToken) {
        self.cancellation = Some(token);
    }

    pub fn regex_error(&self) -> Option<&str> {
        self.regex_error.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
    }

    /// Whether the query is a plain single-line, case-insensitive literal, so
    /// callers may use their own ASCII case-folding scans and caches.
    pub fn can_use_ascii_case_insensitive_fast_path(&self) -> bool {
        !self.options.match_case
            && !self.options.whole_word
            && !self.options.regex
            && !self.query.contains('\n')
    }

    /// Whether the query is a single-line literal, so a match never spans
    /// more than one row of text.
    pub fn can_use_single_row_literal_path(&self) -> bool {
        !self.options.regex && !self.query.contains('\n')
    }

    pub fn is_match(&self, haystack: &str) -> bool {
        self.find_range_at_or_after(haystack, 0).is_some()
    }

    /// Replaces `out` with up to `max_matches` non-overlapping match ranges.
    pub fn find_ranges_into(
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

    /// Match ranges to highlight within a single row of text. A multi-line
    /// query never highlights fragments of itself.
    pub fn find_row_overlay_ranges_into(
        &self,
        haystack: &str,
        out: &mut Vec<Range<usize>>,
        max_matches: usize,
    ) {
        self.find_ranges_into(haystack, out, max_matches);
    }

    /// The first match starting at or after byte offset `start_at`.
    pub fn find_range_at_or_after(&self, haystack: &str, start_at: usize) -> Option<Range<usize>> {
        if self.is_empty() || self.regex_error.is_some() || self.is_cancelled() {
            return None;
        }

        if let Some(regex) = self.regex.as_ref() {
            let mut search_start = start_at.min(haystack.len());
            loop {
                if self.is_cancelled() {
                    return None;
                }
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

        !haystack[..range.start]
            .chars()
            .next_back()
            .is_some_and(is_word_char)
            && !haystack[range.end..]
                .chars()
                .next()
                .is_some_and(is_word_char)
    }
}

/// Characters that make up a word for the whole-word option.
#[inline]
pub fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The byte offset just past the character starting at `ix`, or `None` at the
/// end of `s`.
pub fn next_char_boundary_after(s: &str, ix: usize) -> Option<usize> {
    if ix >= s.len() {
        return None;
    }

    Some(ix + s[ix..].chars().next()?.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::{TextSearchMatcher, TextSearchOptions};
    use crate::services::CancellationToken;

    #[test]
    fn text_search_matcher_honors_case_sensitivity() {
        let default_matcher = TextSearchMatcher::new("render", TextSearchOptions::default());
        assert!(default_matcher.is_match("Render path"));

        let case_sensitive = TextSearchMatcher::new(
            "render",
            TextSearchOptions {
                match_case: true,
                ..TextSearchOptions::default()
            },
        );
        assert!(!case_sensitive.is_match("Render path"));
        assert!(case_sensitive.is_match("render path"));
    }

    #[test]
    fn text_search_matcher_honors_whole_word_boundaries() {
        let matcher = TextSearchMatcher::new(
            "render",
            TextSearchOptions {
                whole_word: true,
                ..TextSearchOptions::default()
            },
        );

        assert!(matcher.is_match("render cache"));
        assert!(!matcher.is_match("prerender cache"));
        assert!(!matcher.is_match("render_cache"));
    }

    #[test]
    fn text_search_matcher_whole_word_uses_unicode_boundaries() {
        let literal = TextSearchMatcher::new(
            "β",
            TextSearchOptions {
                whole_word: true,
                ..TextSearchOptions::default()
            },
        );
        assert!(!literal.is_match("αβγ"));
        assert!(literal.is_match("β value"));

        let regex = TextSearchMatcher::new(
            "β",
            TextSearchOptions {
                whole_word: true,
                regex: true,
                ..TextSearchOptions::default()
            },
        );
        assert!(!regex.is_match("αβγ"));
        assert!(regex.is_match("β value"));
    }

    #[test]
    fn text_search_matcher_handles_regex_and_invalid_regex() {
        let regex = TextSearchMatcher::new(
            r"render\d+",
            TextSearchOptions {
                regex: true,
                ..TextSearchOptions::default()
            },
        );
        assert!(regex.regex_error().is_none());
        assert!(regex.is_match("RENDER42"));

        let invalid = TextSearchMatcher::new(
            "(",
            TextSearchOptions {
                regex: true,
                ..TextSearchOptions::default()
            },
        );
        assert!(invalid.regex_error().is_some());
        assert!(!invalid.is_match("("));
    }

    #[test]
    fn text_search_matcher_row_overlay_does_not_highlight_multiline_fragments() {
        let matcher = TextSearchMatcher::new("foo\nbar", TextSearchOptions::default());
        let mut ranges = Vec::new();

        matcher.find_row_overlay_ranges_into("foo", &mut ranges, 64);
        assert!(ranges.is_empty());

        matcher.find_row_overlay_ranges_into("bar", &mut ranges, 64);
        assert!(ranges.is_empty());
    }

    #[test]
    fn text_search_matcher_normalizes_carriage_returns_in_the_query() {
        let matcher = TextSearchMatcher::new("a\r\nb\rc", TextSearchOptions::default());
        assert_eq!(matcher.query(), "a\nb\nc");
        assert!(matcher.is_match("a\nb\nc"));
    }

    #[test]
    fn text_search_matcher_stops_once_cancelled() {
        let mut matcher = TextSearchMatcher::new("needle", TextSearchOptions::default());
        let token = CancellationToken::new();
        matcher.set_cancellation(token.clone());
        assert!(matcher.is_match("a needle"));
        token.cancel();
        assert!(matcher.is_cancelled());
        assert!(!matcher.is_match("a needle"));
    }

    #[test]
    fn text_search_matcher_counts_constructions_for_tests() {
        super::take_text_search_matchers_built_for_tests();
        let _ = TextSearchMatcher::new("a", TextSearchOptions::default());
        let _ = TextSearchMatcher::new("b", TextSearchOptions::default());
        assert_eq!(super::take_text_search_matchers_built_for_tests(), 2);
        assert_eq!(super::take_text_search_matchers_built_for_tests(), 0);
    }
}
