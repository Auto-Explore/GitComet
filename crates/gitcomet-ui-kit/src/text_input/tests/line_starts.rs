use super::layout::ScrolledPlainInputView;
use super::*;
use crate::text_model::TextModel;

/// One of the edit shapes an editor produces, placed at the start, middle or
/// end of a document of `len` bytes.
fn edit_for_step(step: usize, len: usize) -> (Range<usize>, &'static str) {
    let middle = len / 2 + step * 37;
    match step % 10 {
        0 => (middle..middle, "x"),
        1 => (middle..middle, "\n"),
        2 => (middle..middle + 5, ""),
        3 => (0..0, "// first\n"),
        4 => (len..len, "\nlast"),
        5 => (middle..middle, "é日本🙂"),
        // Longer than a row, so it joins two rows.
        6 => (middle..middle + 90, ""),
        7 => (middle..middle + 3, "pasted\r\nrows\nhere"),
        8 => (0..4, ""),
        _ => (len.saturating_sub(3)..len, "\n"),
    }
}

fn large_document(rows: usize) -> String {
    let mut text = String::new();
    for ix in 0..rows {
        text.push_str(&format!(
            "let value_{ix:06} = compute(alpha, beta, gamma, delta, epsilon) + offset;\n"
        ));
    }
    text
}

fn line_index_built(input: &Entity<TextInput>, app: &App) -> bool {
    input.read(app).content.snapshot().is_line_index_built()
}

/// A frame after an edit reads its rows through the rope.
///
/// An edit drops the model's line-start array, so any reader of it in layout,
/// prepaint, paint or hit-testing rebuilds one entry per row on every
/// keystroke. Each revision builds the array at most once, so counting the
/// frames that left it built counts the builds.
#[gpui::test]
fn frames_after_edits_do_not_build_a_whole_document_line_index(cx: &mut gpui::TestAppContext) {
    let text = large_document(60_000);
    assert!(text.len() > 4 * 1024 * 1024);
    let (view, cx) = cx.add_window_view(ScrolledPlainInputView::new);
    let input = cx.update(|_window, app| view.read(app).input.clone());

    let first_frame_builds = cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(text.clone(), cx);
            input.set_caret(0, cx);
        });
        let _ = window.draw(app);
        usize::from(line_index_built(&input, app))
    });

    let mut full_builds = 0usize;
    for step in 0..50 {
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                let (range, new_text) = edit_for_step(step, input.content.len());
                input.replace_utf8_range(range, new_text, cx);
            });
            // The caret frame, then the frame its autoscroll asked for.
            let _ = window.draw(app);
            let _ = window.draw(app);

            // Hit-testing, the IME rectangle and a painted selection read the
            // frame's line starts too.
            input.update(app, |input, cx| {
                let cursor = input.cursor_offset();
                let caret = input
                    .caret_point_for_hit_testing(cursor)
                    .expect("the caret's row is shaped");
                assert_eq!(input.index_for_position(caret), cursor);
                let caret_utf16 = input.offset_to_utf16(cursor);
                let bounds = input.layout.bounds.expect("the frame recorded its bounds");
                assert!(
                    input
                        .bounds_for_range(caret_utf16..caret_utf16, bounds, window, cx)
                        .is_some()
                );
                input.select_to(cursor.saturating_sub(200), window, cx);
            });
            let _ = window.draw(app);

            full_builds += usize::from(line_index_built(&input, app));
        });
    }

    assert_eq!(
        full_builds, 0,
        "frames after an edit must not build a whole-document line index"
    );
    assert_eq!(first_frame_builds, 0, "nor must the frame that opens it");
    assert!(!input.read_with(cx, |input, _| input.content.snapshot().is_materialized()));
}

/// xorshift64, seeded: a failure names a step that fails again.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound.max(1) as u64) as usize
    }
}

fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Every answer a frame takes from the rope equals the one the whole
/// line-start array gave it.
fn assert_rope_agrees_with_array(control: &str, model: &TextModel, edit_at: usize, rng: &mut Rng) {
    let snapshot = model.snapshot();
    let array = compute_line_starts(control);
    let array = array.as_slice();
    assert_eq!(snapshot.shared_line_starts().as_ref(), array);
    let rope = LineStarts::for_display(None, &snapshot);
    let len = control.len();
    let line_count = array.len();

    assert_eq!(rope.line_count(), line_count);
    assert_eq!(snapshot.line_count(), line_count);
    for line_ix in (0..line_count + 2).chain([usize::MAX]) {
        assert_eq!(
            rope.line_start(line_ix),
            array.line_start(line_ix),
            "start of row {line_ix}"
        );
    }

    let mut offsets = vec![0, len, len + 1, len + 1_000, usize::MAX];
    offsets.extend((0..5).map(|delta| (edit_at + delta).saturating_sub(2)));
    offsets.extend((0..64).map(|_| rng.below(len + 2)));
    for &start in array {
        offsets.extend([start.saturating_sub(1), start, start + 1]);
    }
    let unshaped = PlainLineLayouts::new(line_count, 0, 0);
    for offset in offsets {
        assert_eq!(
            rope.lines_starting_through(offset),
            array.lines_starting_through(offset),
            "rows through offset {offset}"
        );
        assert_eq!(
            line_index_for_offset(&rope, offset, line_count),
            line_index_for_offset(array, offset, line_count),
            "row of offset {offset}"
        );
        assert_eq!(
            line_for_offset(&rope, &unshaped, offset),
            line_for_offset(array, &unshaped, offset),
            "row and column of offset {offset}"
        );
    }

    let whole = LineTextSource::Whole {
        text: control,
        starts: array,
    };
    let flat = LineTextSource::whole(control, &rope);
    let window = LineTextSource::window(&snapshot, 0..line_count, None);
    for line_ix in 0..line_count + 1 {
        let expected = line_text_for_index(control, array, line_ix);
        assert_eq!(whole.line_text(line_ix), expected);
        assert_eq!(flat.line_text(line_ix), expected, "text of row {line_ix}");
        assert_eq!(window.line_text(line_ix), expected, "text of row {line_ix}");
    }

    for _ in 0..8 {
        let rows = rng.below(line_count + 2)..rng.below(line_count + 2);
        assert_eq!(
            byte_range_for_line_range(&rope, len, rows.clone()),
            byte_range_for_line_range(array, len, rows.clone()),
            "bytes of rows {rows:?}"
        );
    }
}

#[test]
fn rope_line_lookups_agree_with_the_whole_array_across_random_edits() {
    const INSERTS: &[&str] = &[
        "x",
        "\n",
        "\r\n",
        "\r",
        "é",
        "日本語",
        "🙂",
        "\n\n\n",
        "tab\there",
        "three\nrows\npasted\n",
        "",
    ];
    // Wider than a rope chunk, so lookups cross chunk boundaries.
    let block = "a row of some sixty bytes, so that forty of them span chunks\n".repeat(40);

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut control = String::new();
    let mut model = TextModel::new();
    assert_rope_agrees_with_array(&control, &model, 0, &mut rng);

    for step in 0..1_500 {
        let at = match rng.below(4) {
            0 => 0,
            1 => control.len(),
            _ => rng.below(control.len() + 1),
        };
        let at = floor_char_boundary(&control, at);
        let delete = match rng.below(8) {
            0..=3 => 0,
            4..=6 => rng.below(200),
            // Now and then the whole tail, down to an empty document.
            _ => rng.below(control.len() + 1),
        };
        let end = floor_char_boundary(&control, at + delete);
        let inserted = if control.len() < 2_048 && rng.below(3) == 0 {
            block.as_str()
        } else {
            INSERTS[rng.below(INSERTS.len())]
        };

        control.replace_range(at..end, inserted);
        model.replace_range(at..end, inserted);
        assert_eq!(model.len(), control.len(), "step {step}");
        assert_rope_agrees_with_array(&control, &model, at, &mut rng);
    }
}

/// `cargo test -p gitcomet-ui-kit --lib timing_post_edit_frame_100k_rows --
/// --ignored --nocapture`: the first frame after typing one character in the
/// middle of a 100k-row document without wrap, and a frame with no edit.
#[gpui::test]
#[ignore = "timing probe"]
fn timing_post_edit_frame_100k_rows(cx: &mut gpui::TestAppContext) {
    let text = large_document(100_000);
    let (view, cx) = cx.add_window_view(ScrolledPlainInputView::new);
    let input = cx.update(|_window, app| view.read(app).input.clone());
    let middle = text.len() / 2;

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(text.clone(), cx);
            input.set_caret(middle, cx);
        });
        let mut edit_frames = Vec::new();
        let mut idle_frames = Vec::new();
        // The first rounds scroll the caret into view and warm the caches.
        for round in 0..64 {
            input.update(app, |input, cx| {
                let at = input.cursor_offset();
                input.replace_utf8_range(at..at, "x", cx);
            });
            let started = std::time::Instant::now();
            let _ = window.draw(app);
            let edit_frame = started.elapsed();

            input.update(app, |_input, cx| cx.notify());
            let started = std::time::Instant::now();
            let _ = window.draw(app);
            let idle_frame = started.elapsed();

            if round >= 13 {
                edit_frames.push(edit_frame);
                idle_frames.push(idle_frame);
            }
        }
        edit_frames.sort();
        idle_frames.sort();
        println!(
            "timing post_edit_frame rows 100000 bytes {} edit frame p50 {:?} min {:?}; idle frame p50 {:?} min {:?}",
            text.len(),
            edit_frames[edit_frames.len() / 2],
            edit_frames[0],
            idle_frames[idle_frames.len() / 2],
            idle_frames[0],
        );
    });
}
