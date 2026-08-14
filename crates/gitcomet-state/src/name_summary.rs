//! Shortening a batch of ref names down to something one line can carry.
//!
//! A group delete can name several hundred branches, and every place that
//! reports on one — the confirm dialog, its command preview, the failure toast
//! that follows — has to stop at the same point and say the same thing about
//! what it left out. Private copies per call site is how the same batch ends up
//! summarised three different ways.

/// How many names are spelled out before the rest are summarised.
///
/// Sized for a 420px confirm dialog, which is the tightest of the callers: the
/// dialog gets no scroll wrapper, so a longer list would push its buttons off
/// screen.
pub const LISTED_NAMES: usize = 8;

/// `"branch"` or `"branches"` for `count`.
pub fn branch_noun(count: usize) -> &'static str {
    if count == 1 { "branch" } else { "branches" }
}

/// What to append when `total` names did not all fit, or `None` when they did.
///
/// Split out for the callers that lay their names out themselves — a stacked
/// list of rows cannot go through [`elide_names`] but still has to elide at the
/// same count and in the same words.
pub fn elision_suffix(total: usize) -> Option<String> {
    let rest = total.saturating_sub(LISTED_NAMES);
    (rest > 0).then(|| format!("…and {rest} more"))
}

/// The first [`LISTED_NAMES`] names joined by `separator`, with
/// [`elision_suffix`] appended when the list is longer.
pub fn elide_names<S: AsRef<str>>(names: &[S], separator: &str) -> String {
    let mut out = String::new();
    for (ix, name) in names.iter().take(LISTED_NAMES).enumerate() {
        if ix > 0 {
            out.push_str(separator);
        }
        out.push_str(name.as_ref());
    }
    if let Some(suffix) = elision_suffix(names.len()) {
        if !out.is_empty() {
            out.push_str(separator);
        }
        out.push_str(&suffix);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|ix| format!("feat/{ix}")).collect()
    }

    #[test]
    fn a_short_list_is_spelled_out_in_full() {
        assert_eq!(elide_names(&names(3), ", "), "feat/0, feat/1, feat/2");
        assert_eq!(elision_suffix(3), None);
    }

    /// The boundary itself is not elided: eight names is exactly what fits.
    #[test]
    fn the_cap_itself_is_not_elided() {
        let joined = elide_names(&names(LISTED_NAMES), " ");
        assert!(joined.ends_with("feat/7"), "got {joined}");
        assert_eq!(elision_suffix(LISTED_NAMES), None);
    }

    #[test]
    fn a_long_list_stops_at_the_cap_and_counts_the_rest() {
        let joined = elide_names(&names(12), ", ");

        assert!(joined.starts_with("feat/0, feat/1"));
        assert!(joined.contains("feat/7"), "got {joined}");
        assert!(!joined.contains("feat/8"), "got {joined}");
        assert!(joined.ends_with("…and 4 more"), "got {joined}");
    }

    /// The suffix has to join with the same separator, or the toast reads
    /// `feat/7…and 4 more`.
    #[test]
    fn the_suffix_is_joined_with_the_separator() {
        assert!(elide_names(&names(9), ", ").ends_with("feat/7, …and 1 more"));
        assert!(elide_names(&names(9), " ").ends_with("feat/7 …and 1 more"));
    }

    #[test]
    fn an_empty_list_produces_nothing() {
        assert_eq!(elide_names::<String>(&[], ", "), "");
    }

    #[test]
    fn branch_noun_is_singular_only_for_one() {
        assert_eq!(branch_noun(0), "branches");
        assert_eq!(branch_noun(1), "branch");
        assert_eq!(branch_noun(2), "branches");
    }
}
