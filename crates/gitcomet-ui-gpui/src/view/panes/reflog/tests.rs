/// The table's cells hold text, so a larger UI font has to widen them or the
/// selector, sha and date columns run into each other.
#[test]
fn reflog_columns_widen_with_the_ui_font() {
    let scale = |ui_font_size_px| {
        UiScale::from_percent(100).with_appearance(crate::appearance::Appearance {
            ui_font_size_px,
            ..crate::appearance::Appearance::default()
        })
    };

    for column in [
        REFLOG_SELECTOR_COLUMN_PX,
        REFLOG_SHA_COLUMN_PX,
        REFLOG_DATE_COLUMN_PX,
        REFLOG_FILTER_WIDTH_PX,
    ] {
        assert_eq!(reflog_column(scale(14), column), px(column));
        assert!(reflog_column(scale(24), column) > reflog_column(scale(14), column));
    }

    assert_eq!(
        reflog_column(UiScale::from_percent(200), REFLOG_SHA_COLUMN_PX),
        px(REFLOG_SHA_COLUMN_PX) * 2.0,
        "and still follow the UI zoom"
    );
}
use super::*;
use std::sync::Arc as StdArc;

fn entry(index: usize, selector: &str, sha: &str, author: &str, message: &str) -> ReflogEntry {
    ReflogEntry {
        index,
        new_id: CommitId(StdArc::from(sha)),
        message: StdArc::from(message),
        time: None,
        selector: StdArc::from(selector),
        author: StdArc::from(author),
    }
}

#[test]
fn empty_query_matches_every_entry() {
    let e = entry(0, "HEAD@{0}", "deadbeefcafe", "Jane Doe", "commit: initial");
    assert!(reflog_entry_matches(&e, ""));
}

#[test]
fn query_matches_selector_sha_message_and_author() {
    let e = entry(
        1,
        "HEAD@{1}",
        "deadbeefcafe",
        "Jane Doe",
        "checkout: moving to feature",
    );

    assert!(reflog_entry_matches(&e, "head@{1}"));
    assert!(reflog_entry_matches(&e, "deadbeef"));
    assert!(reflog_entry_matches(&e, "checkout"));
    assert!(reflog_entry_matches(&e, "jane"));
    assert!(!reflog_entry_matches(&e, "nonexistent"));
}

/// `reflog_entry_matches` takes an already-lowercased query: the filter
/// observer lowercases the search box's text once, when it changes, rather
/// than re-lowercasing it per entry. This exercises that real path — a
/// mixed-case query as the user types it, lowercased exactly once.
#[test]
fn a_mixed_case_query_matches_once_lowercased_by_the_caller() {
    let e = entry(
        1,
        "HEAD@{1}",
        "deadbeefcafe",
        "Jane Doe",
        "checkout: moving to feature",
    );
    let query = "CHECKOUT".to_lowercase();
    assert!(reflog_entry_matches(&e, &query));
}

#[test]
fn query_does_not_match_beyond_the_short_sha_prefix() {
    let e = entry(
        2,
        "HEAD@{2}",
        "deadbeefcafe0000",
        "Jane Doe",
        "reset: moving to HEAD@{1}",
    );
    // Only the first 8 chars are shown/searched as the "sha" field, mirroring
    // what the table displays.
    assert!(!reflog_entry_matches(&e, "cafe0000"));
}

#[test]
fn a_query_change_changes_the_row_cache_key() {
    assert_ne!(hash_query(""), hash_query("checkout"));
    assert_eq!(hash_query("checkout"), hash_query("checkout"));
}
