//! Matching for the history list's find bar.
//!
//! The text is matched with the shared quick search matcher
//! ([`TextSearchMatcher`]), so the find bar honours the same match case, whole
//! word and regex options as the diff and file search bars.

use crate::domain::Commit;
use crate::text_search::{TextSearchMatcher, TextSearchOptions};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

/// A compiled find-bar query. The text matches the summary or the author, each
/// on its own; a plain-text query that could be an abbreviated SHA also
/// matches the start of the commit id.
///
/// Two queries are equal when their text and options are, which is what
/// identifies a search: changing either starts a new one. Cloning shares the
/// compiled matcher, so a regex compiles once per query.
#[derive(Clone)]
pub struct HistoryFindQuery {
    /// As typed: the matcher sees surrounding spaces too.
    text: String,
    options: TextSearchOptions,
    matcher: Arc<TextSearchMatcher>,
    /// The lowercased, trimmed text when it can be a SHA prefix.
    sha_prefix: Option<String>,
}

/// The summary a history row shows for a stash tip, which find matches
/// instead of the commit's own: the stash list's message, or without one the
/// log summary after its "WIP on main:" / "On main:" prefix.
pub fn stash_row_summary<'a>(listed_message: Option<&'a str>, summary: &'a str) -> &'a str {
    listed_message
        .filter(|message| !message.trim().is_empty())
        .or_else(|| stash_summary_tail(summary))
        .unwrap_or(summary)
}

/// Whether parent count and summary have the shape of a stash tip.
pub fn is_probable_stash_summary(parent_count: usize, summary: &str) -> bool {
    (2..=3).contains(&parent_count)
        && (summary.starts_with("WIP on ") || summary.starts_with("On "))
        && summary.contains(": ")
}

/// A stash commit's log summary after its "WIP on main:" / "On main:" prefix.
pub fn stash_summary_tail(summary: &str) -> Option<&str> {
    let (_, tail) = summary.split_once(": ")?;
    Some(tail.trim()).filter(|tail| !tail.is_empty())
}

/// Shortest hex query treated as a SHA prefix. Shorter hex runs such as "add"
/// or "fix" are common words, and every commit id would match them.
const MIN_SHA_PREFIX_LEN: usize = 4;

impl HistoryFindQuery {
    /// `None` for a blank query, which matches nothing rather than everything.
    /// An invalid regex still makes a query; see [`Self::regex_error`].
    pub fn new(text: &str, options: TextSearchOptions) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Regex queries are patterns, not ids. The SHA prefix ignores match
        // case and whole word: ids are hex, and the prefix is a prefix.
        let sha_prefix = (!options.regex
            && trimmed.len() >= MIN_SHA_PREFIX_LEN
            && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| trimmed.to_ascii_lowercase());
        Some(Self {
            text: text.to_owned(),
            options,
            matcher: Arc::new(TextSearchMatcher::new(text, options)),
            sha_prefix,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn options(&self) -> TextSearchOptions {
        self.options
    }

    /// Why the query, in regex mode, is not a valid pattern. Such a query
    /// matches nothing and is not worth searching for.
    pub fn regex_error(&self) -> Option<&str> {
        self.matcher.regex_error()
    }

    pub fn matches(&self, commit: &Commit) -> bool {
        self.matches_fields(commit.id.as_ref(), &commit.summary, &commit.author)
    }

    /// Match cached search text without constructing a full commit object.
    pub fn matches_fields(&self, id: &str, summary: &str, author: &str) -> bool {
        self.sha_prefix_len(id).is_some()
            || self.matcher.is_match(summary)
            || self.matcher.is_match(author)
    }

    /// Replaces `out` with up to `max_matches` ranges of `text` (a summary or
    /// an author) the query matches, for highlighting.
    pub fn text_ranges_into(&self, text: &str, out: &mut Vec<Range<usize>>, max_matches: usize) {
        self.matcher.find_ranges_into(text, out, max_matches);
    }

    /// Length of the leading part of `id` the query matched as a SHA prefix.
    pub fn sha_prefix_len(&self, id: &str) -> Option<usize> {
        self.sha_prefix
            .as_deref()
            .filter(|prefix| starts_with_ignore_ascii_case(id, prefix))
            .map(str::len)
    }

    /// Whether this query can only remove matches from a previous query.
    /// Word boundaries, regex alternatives and newly enabled SHA matching
    /// can all add matches when text is appended, so they cannot narrow.
    pub fn is_refinement_of(&self, previous: &Self) -> bool {
        self.options == previous.options
            && !self.options.regex
            && !self.options.whole_word
            && self.matcher.query().starts_with(previous.matcher.query())
            && self.sha_prefix.as_ref().is_none_or(|prefix| {
                previous
                    .sha_prefix
                    .as_ref()
                    .is_some_and(|old| prefix.starts_with(old))
            })
    }
}

impl PartialEq for HistoryFindQuery {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.options == other.options
    }
}

impl Eq for HistoryFindQuery {}

impl Hash for HistoryFindQuery {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.options.hash(state);
    }
}

impl fmt::Debug for HistoryFindQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryFindQuery")
            .field("text", &self.text)
            .field("options", &self.options)
            .field("sha_prefix", &self.sha_prefix)
            .field("regex_error", &self.regex_error())
            .finish()
    }
}

fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CommitId, CommitParentIds};
    use std::collections::hash_map::DefaultHasher;
    use std::time::SystemTime;

    fn commit(id: &str, summary: &str, author: &str) -> Commit {
        Commit {
            id: CommitId(id.into()),
            parent_ids: CommitParentIds::new(),
            summary: summary.into(),
            author: author.into(),
            time: SystemTime::UNIX_EPOCH,
        }
    }

    const PLAIN: TextSearchOptions = TextSearchOptions {
        match_case: false,
        whole_word: false,
        regex: false,
    };
    const MATCH_CASE: TextSearchOptions = TextSearchOptions {
        match_case: true,
        ..PLAIN
    };
    const WHOLE_WORD: TextSearchOptions = TextSearchOptions {
        whole_word: true,
        ..PLAIN
    };
    const REGEX: TextSearchOptions = TextSearchOptions {
        regex: true,
        ..PLAIN
    };

    fn query(text: &str, options: TextSearchOptions) -> HistoryFindQuery {
        HistoryFindQuery::new(text, options).expect("a non-blank query")
    }

    #[test]
    fn stash_rows_show_their_listed_message_or_the_summary_after_its_prefix() {
        let summary = "On main: savepoint";
        assert_eq!(stash_row_summary(Some("listed"), summary), "listed");
        assert_eq!(stash_row_summary(Some("  "), summary), "savepoint");
        assert_eq!(
            stash_row_summary(None, "WIP on main: keep this"),
            "keep this"
        );
        assert_eq!(stash_row_summary(None, "no delimiter"), "no delimiter");
        assert_eq!(stash_row_summary(None, "On main:  "), "On main:  ");
    }

    fn hash(query: &HistoryFindQuery) -> u64 {
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn extending_a_query_can_gain_sha_or_whole_word_matches() {
        let sha_only = commit("abcd1234", "unrelated", "Ann");
        assert!(!query("abc", PLAIN).matches(&sha_only));
        assert!(query("abcd", PLAIN).matches(&sha_only));
        let longer_word = commit("00000000", "fixes", "Ann");
        assert!(!query("fix", WHOLE_WORD).matches(&longer_word));
        assert!(query("fixes", WHOLE_WORD).matches(&longer_word));
        let alternation = commit("00000000", "feature", "Ann");
        assert!(!query("fix", REGEX).matches(&alternation));
        assert!(query("fix|feature", REGEX).matches(&alternation));
    }

    #[test]
    fn blank_query_matches_nothing() {
        assert_eq!(HistoryFindQuery::new("", PLAIN), None);
        assert_eq!(HistoryFindQuery::new("   ", REGEX), None);
    }

    #[test]
    fn text_matches_the_summary_or_the_author() {
        let upgrade = commit("d5bb3ab2", "upgrade gix to 0.88", "Havunen");
        let fix = commit("c94afbcf", "fix markdown preview", "Havunen");
        assert!(query("UPGRA", PLAIN).matches(&upgrade));
        assert!(!query("UPGRA", PLAIN).matches(&fix));
        assert!(query("havu", PLAIN).matches(&fix));
    }

    /// Each option narrows (or, for regex, reinterprets) the text match.
    #[test]
    fn options_change_what_the_text_matches() {
        let render = commit("11111111", "Render the prerender cache", "Ann");
        let prerender = commit("22222222", "prerender only", "Ann");
        let digits = commit("33333333", "bump to v42", "Ann");
        let cases: [(&str, TextSearchOptions, &Commit, bool); 10] = [
            ("render", PLAIN, &render, true),
            ("render", MATCH_CASE, &render, true),
            ("RENDER", PLAIN, &render, true),
            ("RENDER", MATCH_CASE, &render, false),
            ("render", WHOLE_WORD, &prerender, false),
            ("render", PLAIN, &prerender, true),
            (r"v\d+", PLAIN, &digits, false),
            (r"v\d+", REGEX, &digits, true),
            (r"V\d+", REGEX, &digits, true),
            (
                r"V\d+",
                TextSearchOptions {
                    match_case: true,
                    ..REGEX
                },
                &digits,
                false,
            ),
        ];
        for (text, options, commit, expected) in cases {
            assert_eq!(
                query(text, options).matches(commit),
                expected,
                "{text:?} with {options:?} against {:?}",
                commit.summary
            );
        }
    }

    #[test]
    fn hex_query_matches_the_start_of_the_sha_only() {
        let upgrade = commit("d5bb3ab2", "upgrade gix", "Havunen");
        assert!(query("D5BB3", PLAIN).matches(&upgrade));
        assert!(
            query(" d5bb3 ", PLAIN).matches(&upgrade),
            "trimmed for the SHA"
        );
        assert!(!query("3ab2", PLAIN).matches(&upgrade));
    }

    #[test]
    fn short_hex_words_match_text_not_every_sha() {
        assert!(!query("add", PLAIN).matches(&commit("add12345", "fix typo", "Havunen")));
        assert!(query("add", PLAIN).matches(&commit("d5bb3ab2", "add copy sha", "Havunen")));
    }

    #[test]
    fn sha_prefix_ignores_match_case_and_whole_word() {
        let upgrade = commit("d5bb3ab2", "upgrade gix", "Havunen");
        assert!(query("D5BB3", MATCH_CASE).matches(&upgrade));
        assert!(query("d5bb", WHOLE_WORD).matches(&upgrade));
    }

    #[test]
    fn regex_queries_do_not_match_the_sha() {
        let upgrade = commit("d5bb3ab2", "upgrade gix", "Havunen");
        assert!(!query("d5bb3", REGEX).matches(&upgrade));
        assert!(query("d5bb3", REGEX).matches(&commit("00000000", "revert d5bb3ab2", "A")));
    }

    /// The highlight ranges are what `matches` matched, field by field.
    #[test]
    fn match_ranges_follow_each_field() {
        let mut ranges = Vec::new();
        query("fix", PLAIN).text_ranges_into("Fix a prefix", &mut ranges, 16);
        assert_eq!(ranges, [0..3, 9..12]);
        query("fix", WHOLE_WORD).text_ranges_into("Fix a prefix", &mut ranges, 16);
        assert_eq!(ranges, [0..3]);
        query("^A", REGEX).text_ranges_into("Alice", &mut ranges, 16);
        assert_eq!(ranges, [0..1]);

        assert_eq!(query(" D5BB3 ", PLAIN).sha_prefix_len("d5bb3ab2"), Some(5));
        assert_eq!(query("3ab2", PLAIN).sha_prefix_len("d5bb3ab2"), None);
        assert_eq!(query("d5bb3", REGEX).sha_prefix_len("d5bb3ab2"), None);
    }

    /// The summary and the author are matched one at a time, so a match
    /// never spans the two.
    #[test]
    fn summary_and_author_are_not_concatenated() {
        let fix = commit("c94afbcf", "fix typo", "Alice");
        assert!(!query("typo alice", PLAIN).matches(&fix));
        assert!(!query("typoalice", PLAIN).matches(&fix));
        assert!(query("^Alice$", REGEX).matches(&fix));
        assert!(query("^fix typo$", REGEX).matches(&fix));
        assert!(!query("typo.Alice", REGEX).matches(&fix));
        assert!(query("alice", WHOLE_WORD).matches(&fix));
    }

    #[test]
    fn an_invalid_regex_reports_its_error_and_matches_nothing() {
        let invalid = query("fix(", REGEX);
        assert!(invalid.regex_error().is_some());
        assert!(!invalid.matches(&commit("c94afbcf", "fix( typo", "Alice")));
        assert_eq!(query("fix(", PLAIN).regex_error(), None);
        assert!(query("fix(", PLAIN).matches(&commit("c94afbcf", "fix( typo", "Alice")));
    }

    /// Plain-text matching folds ASCII case only, like the diff and file
    /// search bars; regex mode folds Unicode case.
    #[test]
    fn plain_text_folds_ascii_case_and_regex_folds_unicode() {
        let uber = commit("d5bb3ab2", "Fix über-long lines", "Jörg");
        assert!(!query("ÜBER", PLAIN).matches(&uber));
        assert!(query("über", PLAIN).matches(&uber));
        assert!(query("jö", PLAIN).matches(&uber));
        assert!(query("ÜBER", REGEX).matches(&uber));
        assert!(query("JÖRG", REGEX).matches(&uber));
    }

    #[test]
    fn identity_is_the_text_and_the_options() {
        let fix = query("fix", PLAIN);
        assert_eq!(fix, query("fix", PLAIN));
        assert_eq!(hash(&fix), hash(&query("fix", PLAIN)));
        assert_eq!(fix, fix.clone());
        assert_ne!(fix, query("FIX", PLAIN), "the text is kept as typed");
        assert_ne!(fix, query("fix ", PLAIN), "surrounding spaces count");
        for options in [MATCH_CASE, WHOLE_WORD, REGEX] {
            assert_ne!(fix, query("fix", options), "{options:?}");
        }
        assert_eq!(fix.text(), "fix");
        assert_eq!(query("fix", REGEX).options(), REGEX);
    }
}
