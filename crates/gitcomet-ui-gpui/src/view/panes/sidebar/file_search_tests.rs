use super::*;

fn options(match_case: bool, whole_word: bool, regex: bool) -> TextSearchOptions {
    TextSearchOptions {
        match_case,
        whole_word,
        regex,
    }
}

#[test]
fn default_search_is_case_insensitive_substring() {
    let matchers = file_search_matchers("Read", options(false, false, false));
    assert!(file_search_matches(&matchers, "src/reader.rs"));
    assert!(file_search_matches(&matchers, "README.md"));
    assert!(!file_search_matches(&matchers, "src/writer.rs"));
}

#[test]
fn match_case_narrows_matches() {
    let matchers = file_search_matchers("READ", options(true, false, false));
    assert!(file_search_matches(&matchers, "README.md"));
    assert!(!file_search_matches(&matchers, "src/reader.rs"));
}

#[test]
fn whole_word_requires_boundaries() {
    let matchers = file_search_matchers("read", options(false, true, false));
    assert!(file_search_matches(&matchers, "src/read.rs"));
    assert!(!file_search_matches(&matchers, "src/reader.rs"));
}

#[test]
fn regex_mode_matches_patterns_and_reports_errors() {
    let matchers = file_search_matchers(r"re.d\.rs$", options(false, false, true));
    assert!(file_search_matches(&matchers, "src/read.rs"));
    assert!(!file_search_matches(&matchers, "src/read.rs.bak"));

    let broken = file_search_matchers("re(", options(false, false, true));
    assert!(broken[0].regex_error().is_some());
    assert!(!file_search_matches(&broken, "src/re(.rs"));
}

#[test]
fn each_query_line_is_an_alternative() {
    let matchers = file_search_matchers("reader\nwriter\n\n", options(false, false, false));
    assert_eq!(matchers.len(), 2);
    assert!(file_search_matches(&matchers, "src/reader.rs"));
    assert!(file_search_matches(&matchers, "src/writer.rs"));
    assert!(!file_search_matches(&matchers, "src/printer.rs"));
}

#[test]
fn highlight_ranges_are_sorted_and_merged() {
    let matchers = file_search_matchers("read\neader", options(false, false, false));
    let ranges = file_search_highlight_ranges(&matchers, "reader.rs");
    assert_eq!(ranges, vec![0..6]);

    let matchers = file_search_matchers("r", options(false, false, false));
    let ranges = file_search_highlight_ranges(&matchers, "reader.rs");
    assert_eq!(ranges, vec![0..1, 5..6, 7..8]);
}
