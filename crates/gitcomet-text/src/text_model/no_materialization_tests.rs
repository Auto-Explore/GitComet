use super::*;

/// An edited model whose materialization cache is cold: any accessor that
/// reaches for `as_str()` will flatten the whole document to answer.
fn edited_model() -> TextModel {
    let mut model = TextModel::from_large_text(&"pub fn value() -> u32 { 7 }\n".repeat(4000));
    // Edit mid-document so the piece table is genuinely fragmented and the
    // "unedited original" fast path cannot apply.
    let offset = model.len() / 2;
    model.replace_range(offset..offset, "// touched\n");
    assert!(
        !model.snapshot().is_materialized(),
        "an edit must invalidate the materialization cache"
    );
    model
}

#[test]
fn utf16_conversions_do_not_materialize_the_document() {
    let model = edited_model();
    let snapshot = model.snapshot();

    snapshot.offset_to_utf16(snapshot.len() / 2);
    snapshot.offset_from_utf16(snapshot.len() / 4);

    assert!(
        !snapshot.is_materialized(),
        "caret queries run on every keystroke; they must not flatten the buffer"
    );
}

#[test]
fn reading_one_row_does_not_materialize_the_document() {
    let model = edited_model();
    let snapshot = model.snapshot();

    let row = snapshot.line_count() / 2;
    let text = snapshot.line_text(row);

    assert!(!text.is_empty(), "fixture row should have content");
    assert!(
        !snapshot.is_materialized(),
        "rendering a row must cost the row, not the document"
    );
}

#[test]
fn line_geometry_does_not_materialize_the_document() {
    let model = edited_model();
    let snapshot = model.snapshot();

    let _ = snapshot.line_count();
    let _ = snapshot.line_range(snapshot.line_count() - 1);
    let _ = snapshot.shared_line_starts();

    assert!(!snapshot.is_materialized());
}
