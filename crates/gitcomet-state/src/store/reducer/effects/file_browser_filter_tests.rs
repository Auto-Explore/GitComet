use super::file_browser_query_filters;

/// The same table the view asserts in
/// `file_browser_search_predicate_agrees_with_the_renderers_matchers`.
/// The predicate lives in both crates and cannot be shared, so the two
/// tables are what keep them from drifting: change one, change both.
///
/// Calls the real predicate rather than restating it: a copy here would
/// stay green through exactly the drift it exists to catch.
#[test]
fn filtered_predicate_matches_the_views_table() {
    for (query, expected) in [
        ("", false),
        (" ", false),
        ("\n", false),
        ("  \n \t ", false),
        ("a", true),
        (" a ", true),
        ("a\nb", true),
        ("\na", true),
        ("#comment", true),
    ] {
        assert_eq!(
            file_browser_query_filters(query),
            expected,
            "disagreement for {query:?}"
        );
    }
}
