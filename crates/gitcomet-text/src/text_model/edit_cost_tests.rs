use super::*;

#[test]
fn editing_a_large_document_does_not_materialize_it() {
    let mut model = TextModel::from_large_text(&"fn value() -> u32 { 7 }\n".repeat(20_000));
    assert!(!model.snapshot().is_materialized());

    // Mid-document insert, delete, and replace — the three shapes an editor
    // produces. None of them may need the document as one string.
    let middle = model.len() / 2;
    model.replace_range(middle..middle, "// inserted\n");
    model.replace_range(middle..middle + 6, "");
    model.replace_range(middle..middle + 4, "abcd");

    assert!(
        !model.snapshot().is_materialized(),
        "editing must not flatten the buffer"
    );
}

/// A snapshot taken before an edit keeps observing the old document, and
/// getting there costs an atomic increment rather than a copy.
#[test]
fn snapshots_are_cheap_and_isolated_from_later_edits() {
    let text = "line of text\n".repeat(20_000);
    let mut model = TextModel::from_large_text(&text);
    let before = model.snapshot();

    model.replace_range(0..0, "prefix\n");

    assert_eq!(before.len(), text.len());
    assert_eq!(before.line_text(0).as_ref(), "line of text");
    assert_eq!(model.snapshot().line_text(0).as_ref(), "prefix");
    assert_ne!(before, model.snapshot());
}

/// History keeps these across many edits: the text and its identity, none
/// of the caches the live model warmed.
#[test]
fn snapshots_without_caches_keep_only_the_text_and_identity() {
    let model = TextModel::from_large_text("alpha\nbeta");
    model.as_str();
    model.line_starts();

    let history = model.snapshot_without_caches();

    assert_eq!(history, model.snapshot());
    assert_eq!(history.line_text(1).as_ref(), "beta");
    assert!(!history.is_materialized());
    assert!(!history.is_line_index_built());
    assert!(model.snapshot().is_materialized());
}

/// `longest_row` backs the horizontal scroll bound, and it must stay a
/// summary read rather than a scan.
#[test]
fn the_widest_row_is_known_without_scanning() {
    let mut model = TextModel::from_large_text(&"short\n".repeat(10_000));
    let wide = "w".repeat(4096);
    model.replace_range(0..0, &format!("{wide}\n"));

    let snapshot = model.snapshot();
    assert_eq!(snapshot.line_text(0).as_ref().len(), wide.len());
    assert!(!snapshot.is_materialized());
}
