use super::*;
use gitcomet_core::domain::{Commit, CommitId, CommitParentIds};

fn commit(author: &str) -> Commit {
    Commit {
        id: CommitId("deadbeefdeadbeef".into()),
        parent_ids: CommitParentIds::new(),
        summary: "msg".into(),
        author: author.into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

fn labels(rows: &AuthorRows) -> Vec<String> {
    rows.targets.iter().map(target_label).collect()
}

/// The rows as the dropdown actually renders them for `query` — narrowing
/// and match-position ordering both belong to `PickerPrompt`.
fn rendered_labels(rows: &AuthorRows, query: &str) -> Vec<String> {
    components::picker_prompt_layout(&rows.items, query)
        .item_indices
        .iter()
        .map(|&ix| target_label(&rows.targets[ix]))
        .collect()
}

fn target_label(target: &AuthorTarget) -> String {
    match target {
        AuthorTarget::All => ALL_AUTHORS_LABEL.to_owned(),
        AuthorTarget::Author(name) => name.to_string(),
    }
}

#[test]
fn author_suggestions_are_deduplicated_case_insensitively_and_sorted() {
    let authors = collect_author_suggestions(&[
        commit("Bob"),
        commit("Alice"),
        commit("alice"),
        commit("  Bob  "),
        commit(""),
        commit("   "),
    ]);

    assert_eq!(authors, vec!["Alice", "Bob"]);
}

#[test]
fn all_authors_is_first_and_marked_when_no_filter_is_active() {
    let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into()];
    let rows = rows(&authors, None);

    assert_eq!(labels(&rows), vec!["All authors", "Alice", "Bob"]);
    assert_eq!(rows.marked_index, Some(0));
    assert_eq!(rows.targets[0], AuthorTarget::All);
    assert_eq!(rows.targets[1], AuthorTarget::Author("Alice".into()));
}

/// Dedup, ordering and the active-filter mark all fold ASCII only — the rule
/// the backend's own matcher uses. A Unicode fold in any one of the three
/// would disagree with the other two: names that show as separate rows would
/// sort as if they were the same, and the row for the active filter would go
/// unmarked.
#[test]
fn non_ascii_names_are_folded_the_same_way_at_every_step() {
    let authors = collect_author_suggestions(&[
        commit("Zoe"),
        commit("Éric"),
        commit("éric"),
        commit("ÉRIC"),
    ]);

    // "Éric" and "ÉRIC" differ only in ASCII letters, so they collapse; the
    // two accents stay distinct, because nothing downstream — the backend
    // matcher least of all — would treat them as the same person.
    assert_eq!(authors, vec!["Zoe", "Éric", "éric"]);

    let rows = rows(&authors, Some("éric"));
    assert_eq!(
        rows.marked_index,
        Some(3),
        "the row for the active filter has to be the one that is marked"
    );
}

#[test]
fn active_filter_is_marked_case_insensitively() {
    let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into()];
    let rows = rows(&authors, Some("alice"));

    assert_eq!(rows.marked_index, Some(1));
}

#[test]
fn query_narrows_rendered_rows_case_insensitively() {
    let authors: Vec<SharedString> = vec!["Alice".into(), "Bob".into(), "boberta".into()];
    let rows = rows(&authors, None);

    assert_eq!(rendered_labels(&rows, "BO"), vec!["Bob", "boberta"]);
}

/// Every author is offered, however many there are — the list is
/// virtualized, so a long one costs no more to render than a short one.
#[test]
fn every_matching_author_gets_a_row() {
    let authors: Vec<SharedString> = (0..5_000)
        .map(|ix| SharedString::from(format!("author {ix:04}")))
        .collect();
    let rows = rows(&authors, None);

    // "All authors" rides along on top of the full list.
    assert_eq!(rows.items.len(), 5_001);
    assert_eq!(rows.targets.len(), 5_001);
    assert_eq!(
        rows.targets.last(),
        Some(&AuthorTarget::Author("author 4999".into()))
    );
}

/// Navigation walks the rendered order, which `PickerPrompt` re-sorts by
/// match position once a query is present.
#[test]
fn nav_order_follows_the_rendered_order() {
    let authors: Vec<SharedString> = vec!["Zoe Bar".into(), "Bar Zoe".into()];
    let rows = rows(&authors, None);

    // "Bar Zoe" matches at offset 0, so it renders above "Zoe Bar";
    // "All authors" does not match and drops out entirely.
    assert_eq!(rendered_labels(&rows, "bar"), vec!["Bar Zoe", "Zoe Bar"]);
}
