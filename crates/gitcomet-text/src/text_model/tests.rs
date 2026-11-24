use super::*;

fn line_starts_for_text(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (ix, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(ix + 1);
        }
    }
    starts
}

fn clamp_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    offset
}

fn normalize_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = clamp_to_char_boundary(text, range.start.min(text.len()));
    let end = clamp_to_char_boundary(text, range.end.min(text.len()));
    if end < start { end..start } else { start..end }
}

fn replace_control(text: &mut String, range: Range<usize>, inserted: &str) -> Range<usize> {
    let normalized = normalize_range(text.as_str(), range);
    text.replace_range(normalized.clone(), inserted);
    normalized.start..normalized.start.saturating_add(inserted.len())
}

#[test]
fn replace_range_updates_text_and_line_index() {
    let mut model = TextModel::from_large_text("alpha\nbeta\ngamma");
    let inserted = model.replace_range(6..10, "BETA\nDELTA");
    assert_eq!(inserted, 6..16);
    assert_eq!(model.as_str(), "alpha\nBETA\nDELTA\ngamma");
    assert_eq!(model.line_starts(), &[0, 6, 11, 17]);
}

#[test]
fn replace_range_keeps_line_start_when_edit_ends_at_line_boundary() {
    let mut model = TextModel::from_large_text("ab\ncd");
    let inserted = model.replace_range(0..3, "");
    assert_eq!(inserted, 0..0);
    assert_eq!(model.as_str(), "cd");
    assert_eq!(model.line_starts(), &[0]);
}

#[test]
fn replace_range_dropping_newline_removes_stale_line_start() {
    let mut model = TextModel::from_large_text("a\nb\nc");
    let inserted = model.replace_range(1..2, "");
    assert_eq!(inserted, 1..1);
    assert_eq!(model.as_str(), "ab\nc");
    assert_eq!(model.line_starts(), &[0, 3]);
}

#[test]
fn snapshot_clone_is_cheap_and_immutable_after_mutation() {
    let mut model = TextModel::from_large_text("hello world");
    let snapshot_a = model.snapshot();
    let snapshot_b = snapshot_a.clone();
    let snapshot_revision = snapshot_a.revision();

    model.replace_range(0..5, "goodbye");

    assert_eq!(snapshot_a.as_str(), "hello world");
    assert_eq!(snapshot_b.as_str(), "hello world");
    assert_eq!(snapshot_a.revision(), snapshot_revision);
    assert_ne!(snapshot_a.revision(), model.revision());
}

#[test]
fn snapshot_shared_line_starts_remain_stable_after_edit() {
    let mut model = TextModel::from_large_text("alpha\nbeta\ngamma");
    let old_snapshot = model.snapshot();
    let old_starts = old_snapshot.shared_line_starts();

    model.replace_range(6..10, "BETA\nDELTA");

    let new_starts = model.snapshot().shared_line_starts();

    assert!(
        !Arc::ptr_eq(&old_starts, &new_starts),
        "editing should swap to a new line-start index"
    );
    assert_eq!(old_starts.as_ref(), &[0, 6, 11]);
    assert_eq!(new_starts.as_ref(), &[0, 6, 11, 17]);
}

#[test]
fn from_large_text_chunks_preserve_content() {
    let mut text = String::new();
    for ix in 0..2_048usize {
        text.push_str(format!("line_{ix:04}\n").as_str());
    }
    let model = TextModel::from_large_text(text.as_str());
    assert_eq!(model.len(), text.len());
    assert_eq!(model.as_str(), text);
    assert_eq!(model.line_starts().len(), 2_049);
}

#[test]
fn replace_range_clamps_unicode_boundaries() {
    let mut model = TextModel::from_large_text("🙂\nβeta");
    let inserted = model.replace_range(1..6, "é\n");
    assert_eq!(inserted, 0..3);
    assert_eq!(model.as_str(), "é\nβeta");
    assert_eq!(model.line_starts(), &[0, 3]);
}

#[test]
fn snapshot_slice_to_string_matches_full_text_across_piece_boundaries() {
    let mut model = TextModel::new();
    let _ = model.append_large("left-");
    let _ = model.append_large("🙂middle-");
    let _ = model.append_large("right");
    let snapshot = model.snapshot();
    let full = snapshot.as_str();
    let expected_range = normalize_range(full, 3..17);
    let expected = full[expected_range].to_string();
    assert_eq!(snapshot.slice_to_string(3..17), expected);
}

#[test]
#[allow(clippy::reversed_empty_ranges)]
fn replace_range_normalizes_reversed_and_out_of_bounds_ranges() {
    let mut model = TextModel::from_large_text("abcdef");
    let inserted = model.replace_range(128..2, "XY");
    assert_eq!(inserted, 2..4);
    assert_eq!(model.as_str(), "abXY");
    assert_eq!(model.line_starts(), &[0]);

    let inserted = model.replace_range(4..999, "!");
    assert_eq!(inserted, 4..5);
    assert_eq!(model.as_str(), "abXY!");
    assert_eq!(model.line_starts(), &[0]);
}

#[test]
fn replace_range_handles_empty_model_insert_and_delete() {
    let mut model = TextModel::new();
    let inserted = model.replace_range(0..16, "");
    assert_eq!(inserted, 0..0);
    assert_eq!(model.as_str(), "");
    assert_eq!(model.line_starts(), &[0]);

    let inserted = model.replace_range(0..0, "hello\n");
    assert_eq!(inserted, 0..6);
    assert_eq!(model.as_str(), "hello\n");
    assert_eq!(model.line_starts(), &[0, 6]);

    let inserted = model.replace_range(0..usize::MAX, "");
    assert_eq!(inserted, 0..0);
    assert_eq!(model.as_str(), "");
    assert_eq!(model.line_starts(), &[0]);
}

#[test]
fn replace_range_updates_consecutive_newline_line_starts() {
    let mut model = TextModel::from_large_text("a\n\n\nb");
    let inserted = model.replace_range(1..4, "\n\n");
    assert_eq!(inserted, 1..3);
    assert_eq!(model.as_str(), "a\n\nb");
    assert_eq!(model.line_starts(), &[0, 2, 3]);
}

#[test]
fn apply_edit_at_line_boundaries_stays_monotonic() {
    // Exercises boundary conditions around the monotonic-output guarantee:
    // edits exactly at newline offsets, multi-newline inserts replacing
    // multi-newline ranges, and empty-range inserts at every line start.
    let cases: &[(&str, Range<usize>, &str)] = &[
        // Delete a newline exactly between two line starts.
        ("a\nb\nc", 1..2, ""),
        // Replace across multiple newlines with multiple newlines.
        ("a\nb\nc\nd", 2..5, "X\nY\nZ"),
        // Insert newlines at position 0.
        ("abc", 0..0, "\n\n"),
        // Insert at end after trailing newline.
        ("a\n", 2..2, "b\nc"),
        // Replace entire content.
        ("old\ntext", 0..8, "new\n\nlines\n"),
        // Delete range that spans from before a newline to after it.
        ("ab\ncd\nef", 2..5, ""),
        // Insert at every line start in a multi-line doc.
        ("a\nb\nc\n", 0..0, "X"),
        ("a\nb\nc\n", 2..2, "X"),
        ("a\nb\nc\n", 4..4, "X"),
        // Replace newline with newlines.
        ("a\nb", 1..2, "\n\n"),
    ];
    for (text, range, inserted) in cases {
        let mut model = TextModel::from_large_text(text);
        model.replace_range(range.clone(), inserted);
        let mut control = text.to_string();
        replace_control(&mut control, range.clone(), inserted);
        assert_eq!(model.as_str(), control, "text mismatch for edit {text:?}");
        let expected_starts = line_starts_for_text(&control);
        assert_eq!(
            model.line_starts(),
            expected_starts.as_slice(),
            "line starts mismatch for edit on {text:?} [{range:?} -> {inserted:?}]"
        );
    }
}

#[test]
fn sequential_edits_match_string_control() {
    let mut model = TextModel::from_large_text("😀alpha\nβeta\n\ngamma");
    let mut control = model.as_str().to_string();
    let edits = [
        (1usize, 6usize, "X"),
        (12usize, 4usize, "Q\n"),
        (999usize, 999usize, "\ntail"),
        (3usize, 1_000usize, ""),
        (0usize, 0usize, "prefix\n"),
        (2usize, 2usize, "🙂"),
        (5usize, 8usize, ""),
        (usize::MAX - 1, 1usize, "Ω"),
    ];

    for (start, end, inserted_text) in edits {
        let range = start..end;
        let expected_inserted = replace_control(&mut control, range.clone(), inserted_text);
        let actual_inserted = model.replace_range(range, inserted_text);
        assert_eq!(actual_inserted, expected_inserted);
        assert_eq!(model.as_str(), control);
        let expected_starts = line_starts_for_text(control.as_str());
        assert_eq!(model.line_starts(), expected_starts.as_slice());
    }
}

/// A caret offset inside a character must not hand the character to both
/// walks: a word move subtracts what the backward walk passed.
#[test]
fn char_walks_split_a_mid_character_offset_at_its_start() {
    let snapshot = TextModel::from_large_text("aé").snapshot();

    assert_eq!(snapshot.reversed_chars_at(2).collect::<String>(), "a");
    assert_eq!(snapshot.chars_at(2).collect::<String>(), "é");
    assert_eq!(snapshot.reversed_chars_at(3).collect::<String>(), "éa");
}
