//! Matching for the history list's find bar.

use crate::domain::Commit;

/// A normalized find-bar query. Text matches the summary or author without
/// regard to case; a query that could be an abbreviated SHA also matches the
/// start of the commit id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryFindQuery {
    text: String,
    sha_prefix: bool,
}

/// Shortest hex query treated as a SHA prefix. Shorter hex runs such as "add"
/// or "fix" are common words, and every commit id would match them.
const MIN_SHA_PREFIX_LEN: usize = 4;

impl HistoryFindQuery {
    /// `None` for a blank query, which matches nothing rather than everything.
    pub fn new(query: &str) -> Option<Self> {
        let text = query.trim().to_lowercase();
        if text.is_empty() {
            return None;
        }
        let sha_prefix =
            text.len() >= MIN_SHA_PREFIX_LEN && text.bytes().all(|byte| byte.is_ascii_hexdigit());
        Some(Self { text, sha_prefix })
    }

    pub fn matches(&self, commit: &Commit) -> bool {
        (self.sha_prefix && starts_with_ignore_ascii_case(commit.id.as_ref(), &self.text))
            || contains_lowercase(&commit.summary, &self.text)
            || contains_lowercase(&commit.author, &self.text)
    }
}

fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// `needle` is already lowercase. ASCII haystacks, the common case, avoid the
/// allocation that a full Unicode lowercase conversion needs.
fn contains_lowercase(haystack: &str, needle: &str) -> bool {
    if haystack.is_ascii() {
        let needle = needle.as_bytes();
        return needle.len() <= haystack.len()
            && haystack
                .as_bytes()
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle));
    }
    haystack.to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CommitId, CommitParentIds};
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

    #[test]
    fn blank_query_matches_nothing() {
        assert_eq!(HistoryFindQuery::new(""), None);
        assert_eq!(HistoryFindQuery::new("   "), None);
    }

    #[test]
    fn summary_matches_ignore_case() {
        let query = HistoryFindQuery::new("UPGRA").unwrap();
        assert!(query.matches(&commit("d5bb3ab2", "upgrade gix to 0.88", "Havunen")));
        assert!(!query.matches(&commit("c94afbcf", "fix markdown preview", "Havunen")));
    }

    #[test]
    fn author_matches() {
        let query = HistoryFindQuery::new("havu").unwrap();
        assert!(query.matches(&commit("d5bb3ab2", "upgrade gix", "Havunen")));
    }

    #[test]
    fn hex_query_matches_the_start_of_the_sha_only() {
        let query = HistoryFindQuery::new("D5BB3").unwrap();
        assert!(query.matches(&commit("d5bb3ab2", "upgrade gix", "Havunen")));
        let middle = HistoryFindQuery::new("3ab2").unwrap();
        assert!(!middle.matches(&commit("d5bb3ab2", "upgrade gix", "Havunen")));
    }

    #[test]
    fn short_hex_words_match_text_not_every_sha() {
        let query = HistoryFindQuery::new("add").unwrap();
        assert!(!query.matches(&commit("add12345", "fix typo", "Havunen")));
        assert!(query.matches(&commit("d5bb3ab2", "add copy commit sha", "Havunen")));
    }

    #[test]
    fn non_ascii_summaries_match_ignore_case() {
        let query = HistoryFindQuery::new("ÜBER").unwrap();
        assert!(query.matches(&commit("d5bb3ab2", "Fix über-long lines", "Jörg")));
        assert!(
            HistoryFindQuery::new("jö")
                .unwrap()
                .matches(&commit("d5bb3ab2", "summary", "Jörg"))
        );
    }
}
