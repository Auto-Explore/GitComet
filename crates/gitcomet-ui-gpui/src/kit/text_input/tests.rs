use super::highlight::*;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;

#[test]
fn mask_text_preserves_length_and_newlines() {
    let input = "a\nb\r\nc";
    let masked = mask_text_for_display(input);
    assert_eq!(masked.len(), input.len());
    assert_eq!(masked, "*\n*\r\n*");
}

#[test]
fn mask_text_removes_original_characters() {
    let input = "secret-passphrase";
    let masked = mask_text_for_display(input);
    assert_ne!(masked, input);
    assert!(masked.chars().all(|ch| ch == '*'));
}

#[test]
fn truncate_line_for_shaping_respects_utf8_boundary_and_appends_suffix() {
    let input = "éééé";
    let (truncated, hash) = truncate_line_for_shaping(input, 5);
    assert_eq!(truncated.as_ref(), "é…");
    // hash_shaping_slice must be consistent with truncate_line_for_shaping
    let (hash2, _) = hash_shaping_slice(input, 5);
    assert_eq!(hash, hash2);
}

fn styled(range: Range<usize>) -> (Range<usize>, gpui::HighlightStyle) {
    (
        range,
        gpui::HighlightStyle {
            color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        },
    )
}

fn mapped_ranges(
    interpolation: &HighlightInterpolation,
    source: &[(Range<usize>, gpui::HighlightStyle)],
    clamp_len: usize,
) -> Vec<Range<usize>> {
    interpolation
        .map_highlights(source, clamp_len)
        .into_iter()
        .map(|(range, _)| range)
        .collect()
}

#[test]
fn highlight_interpolation_starts_exact() {
    let interpolation = HighlightInterpolation::default();
    assert!(interpolation.is_exact());
    assert_eq!(interpolation.to_source_offset(7), 7);
    assert_eq!(interpolation.debug_patch(), None);
}

#[test]
fn highlight_interpolation_shifts_highlights_after_an_insert() {
    let mut interpolation = HighlightInterpolation::default();
    // Two characters typed at offset 4.
    interpolation.record_edit(&(4..4), &(4..6));

    assert!(!interpolation.is_exact());
    // Before the caret: untouched. After it: shifted by the inserted length.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..3), styled(8..12)], 32),
        vec![0..3, 10..14]
    );
}

#[test]
fn highlight_interpolation_shifts_highlights_after_a_delete() {
    let mut interpolation = HighlightInterpolation::default();
    // Three characters deleted at offset 4.
    interpolation.record_edit(&(4..7), &(4..4));

    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..3), styled(10..14)], 32),
        vec![0..3, 7..11]
    );
    // A highlight entirely inside the deleted span has nothing left to describe.
    assert!(mapped_ranges(&interpolation, &[styled(5..6)], 32).is_empty());
}

#[test]
fn highlight_interpolation_shifts_highlights_after_a_replace() {
    let mut interpolation = HighlightInterpolation::default();
    // Two characters replaced by five at offset 4.
    interpolation.record_edit(&(4..6), &(4..9));

    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..4), styled(6..10)], 32),
        vec![0..4, 9..13]
    );
}

#[test]
fn highlight_interpolation_splits_a_highlight_straddling_the_edit() {
    let mut interpolation = HighlightInterpolation::default();
    // Source bytes 4..6 became live bytes 4..8.
    interpolation.record_edit(&(4..6), &(4..8));

    // The surviving halves keep their color; the freshly typed bytes 4..8 fall
    // back to the base color rather than taking the whole range down with them.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..10)], 32),
        vec![0..4, 8..12]
    );
}

#[test]
fn highlight_interpolation_sorts_split_pieces_across_overlapping_sources() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..6), &(4..8));

    // The first range's right piece (8..12) lands after the second range's left
    // piece (2..4), so source order is not output order — and `HighlightCursor`
    // binary-searches these.
    let mapped = mapped_ranges(&interpolation, &[styled(0..10), styled(2..3)], 32);
    assert_eq!(mapped, vec![0..4, 2..3, 8..12]);
    assert!(mapped.windows(2).all(|pair| pair[0].start <= pair[1].start));
}

#[test]
fn highlight_interpolation_clamps_to_the_live_text_length() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(0..6), &(0..2));

    // The source described a longer buffer; nothing may point past the end of
    // the one being rendered.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(6..20)], 8),
        vec![2..8]
    );
}

#[test]
fn highlight_interpolation_coalesces_a_run_of_single_character_inserts() {
    let mut interpolation = HighlightInterpolation::default();
    for offset in 0..10 {
        // A caret at 4 that advances one byte per keystroke.
        let caret = 4 + offset;
        interpolation.record_edit(&(caret..caret), &(caret..caret + 1));
    }

    assert_eq!(interpolation.generation(), 10);
    let patch = interpolation
        .debug_patch()
        .expect("typing should leave one patch");
    assert_eq!(patch.start, 4);
    assert_eq!(
        patch.old_len, 0,
        "an insert run replaces nothing, so the source span stays empty"
    );
    assert_eq!(patch.new_len, 10);
    // Highlights past the caret are shifted by the whole run at once.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(4..8)], 64),
        vec![14..18]
    );
}

#[test]
fn highlight_interpolation_collapses_when_typing_is_backspaced_away() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..4), &(4..5));
    interpolation.record_edit(&(4..5), &(4..4));

    assert!(
        interpolation.is_exact(),
        "an insert and the backspace undoing it leave the source describing the buffer verbatim"
    );
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(2..8)], 32),
        vec![2..8]
    );
}

#[test]
fn highlight_interpolation_widens_to_the_union_across_disjoint_edits() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..4), &(4..5));
    interpolation.record_edit(&(20..20), &(20..21));

    let patch = interpolation
        .debug_patch()
        .expect("two edits should leave one widened patch");
    assert_eq!(patch.start, 4);
    // The documented degradation: text between two disjoint edits is inside the
    // union, so it renders in the base color until the recompute lands, rather
    // than keeping highlights that would now sit on the wrong bytes.
    assert_eq!(patch.old_len, 15);
    assert_eq!(patch.new_len, 17);
    assert_eq!(
        mapped_ranges(
            &interpolation,
            &[styled(0..2), styled(8..12), styled(24..28)],
            64
        ),
        vec![0..2, 26..30]
    );
}

#[test]
fn highlight_interpolation_reset_restores_the_identity_map() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..4), &(4..6));
    let generation = interpolation.generation();

    interpolation.reset();

    assert!(interpolation.is_exact());
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(8..12)], 32),
        vec![8..12]
    );
    assert!(
        interpolation.generation() > generation,
        "a reset changes the coordinates callers cached against"
    );
}

#[test]
fn highlight_interpolation_source_offsets_are_monotone_and_locally_exact() {
    // Every shape of edit — insert, delete, replace, both directions — against
    // every offset in a small buffer.
    for old_len in 0..5usize {
        for new_len in 0..5usize {
            let mut interpolation = HighlightInterpolation::default();
            interpolation.record_edit(&(4..4 + old_len), &(4..4 + new_len));

            let mut previous = 0usize;
            for offset in 0..24usize {
                let source = interpolation.to_source_offset(offset);
                assert!(
                    source >= previous,
                    "to_source_offset must be monotone (old_len={old_len}, new_len={new_len}, offset={offset})"
                );
                previous = source;

                if offset <= 4 {
                    assert_eq!(
                        source, offset,
                        "offsets before the patch are unchanged (old_len={old_len}, new_len={new_len})"
                    );
                }
                // Strict: with `new_len == 0` the offset at the patch start is
                // both "before" and "past" the edit, and the map answers with
                // the lower of the two. Only monotonicity binds there.
                if offset > 4 + new_len {
                    assert_eq!(
                        source,
                        offset + old_len - new_len,
                        "offsets past the patch shift by the length delta (old_len={old_len}, new_len={new_len})"
                    );
                }
            }
        }
    }
}

#[test]
fn visible_plain_line_range_applies_guard_rows() {
    let range = visible_plain_line_range(100, px(20.0), px(200.0), px(260.0), 2);
    assert_eq!(range, 8..16);
}

#[test]
fn provider_prefetch_byte_range_extends_visible_window_with_guard_rows() {
    let text = std::iter::repeat_n("x", 100).collect::<Vec<_>>().join("\n");
    let line_starts = compute_line_starts(text.as_str());
    let range = provider_prefetch_byte_range_for_visible_window(
        line_starts.as_slice(),
        text.len(),
        100,
        px(20.0),
        px(600.0),
        px(660.0),
    );

    assert_eq!(range, 12..116);
}

#[test]
fn provider_prefetch_byte_range_clamps_to_document_bounds() {
    let text = std::iter::repeat_n("x", 10).collect::<Vec<_>>().join("\n");
    let line_starts = compute_line_starts(text.as_str());
    let range = provider_prefetch_byte_range_for_visible_window(
        line_starts.as_slice(),
        text.len(),
        10,
        px(20.0),
        px(0.0),
        px(20.0),
    );

    assert_eq!(range, 0..text.len());
}

#[test]
fn wrapped_line_index_and_visible_range_use_row_counts() {
    let row_counts = vec![1, 3, 1, 2, 1];
    let y_offsets = vec![px(0.0), px(10.0), px(40.0), px(50.0), px(70.0)];
    let line_height = px(10.0);

    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(35.0)),
        1
    );
    let range =
        visible_wrapped_line_range(&y_offsets, &row_counts, line_height, px(42.0), px(58.0), 0);
    assert_eq!(range, 2..4);
}

#[test]
fn compute_line_starts_and_line_text_handle_trailing_newline() {
    let text = "alpha\nbeta\n";
    let starts = compute_line_starts(text);
    assert_eq!(starts, vec![0, 6, 11]);
    assert_eq!(line_text_for_index(text, starts.as_slice(), 0), "alpha");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 1), "beta");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 2), "");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 3), "");
}

#[test]
fn line_text_for_index_excludes_crlf_terminators() {
    let text = "alpha\r\nbeta\r\n";
    let starts = compute_line_starts(text);
    assert_eq!(starts, vec![0, 7, 13]);
    assert_eq!(line_text_for_index(text, starts.as_slice(), 0), "alpha");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 1), "beta");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 2), "");
}

#[gpui::test]
fn multiline_crlf_plain_layout_draws_without_painting_carriage_returns(
    cx: &mut gpui::TestAppContext,
) {
    let text = "[server]\r\nhost=local-api.internal\r\nport=8181\r\n";
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                soft_wrap: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| input.set_text(text, cx));
        let _ = window.draw(app);

        let input = input.read(app);
        assert_eq!(input.text(), text);
        let TextInputLayout::Plain(lines) = input
            .layout
            .last
            .as_ref()
            .expect("expected plain text input layout")
        else {
            panic!("expected plain text input layout");
        };
        assert_eq!(
            lines.get(0).expect("shaped line 0").text.as_ref(),
            "[server]"
        );
        assert_eq!(
            lines.get(1).expect("shaped line 1").text.as_ref(),
            "host=local-api.internal"
        );
    });
}

/// A plain multiline input inside a fixed-height scrolling viewport, the way
/// the merge tool hosts its resolved-output editor.
struct ScrolledPlainInputView {
    input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
}

impl ScrolledPlainInputView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll_handle = ScrollHandle::new();
        let input = cx.new({
            let scroll_handle = scroll_handle.clone();
            move |cx| {
                let mut input = TextInput::new(
                    TextInputOptions {
                        multiline: true,
                        soft_wrap: false,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                input.set_vertical_scroll_handle(Some(scroll_handle));
                input
            }
        });
        Self {
            input,
            scroll_handle,
        }
    }
}

impl Render for ScrolledPlainInputView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .id("plain_input_viewport")
                .w(px(600.0))
                .h(px(400.0))
                .overflow_y_scroll()
                .track_scroll(&self.scroll_handle)
                .child(self.input.clone()),
        )
    }
}

#[gpui::test]
fn plain_multiline_layout_shapes_only_the_viewport_of_a_large_document(
    cx: &mut gpui::TestAppContext,
) {
    // `ShapedLine` is ~3 KB, so a layout kept per document line costs megabytes
    // of zeroing on every frame of a large buffer — which is what made typing in
    // the merge tool's resolved output scale with the file instead of the
    // viewport.
    let line_count = 5_000;
    let text = (0..line_count)
        .map(|ix| format!("fn line_{ix:05}(value: usize) -> usize {{ value + {ix} }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (view, cx) = cx.add_window_view(ScrolledPlainInputView::new);
    let input = cx.update(|_window, app| view.read(app).input.clone());

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(text.clone(), cx);
            input.set_selected_range(0..0, false, cx);
        });
        let _ = window.draw(app);

        let input = input.read(app);
        let TextInputLayout::Plain(lines) = input
            .layout
            .last
            .as_ref()
            .expect("expected plain text input layout")
        else {
            panic!("expected plain text input layout");
        };
        assert_eq!(lines.line_count(), line_count);
        assert!(
            lines.shaped_line_count() < line_count / 4,
            "shaped {} of {line_count} lines — the layout should cover the viewport, not the document",
            lines.shaped_line_count(),
        );
        // The rows that are on screen still resolve, and off-screen rows report
        // no geometry rather than a bogus zero-width line.
        assert_eq!(
            lines.get(0).expect("shaped line 0").text.as_ref(),
            "fn line_00000(value: usize) -> usize { value + 0 }"
        );
        assert!(lines.get(line_count - 1).is_none());
    });
}

#[test]
fn wrapped_line_index_for_y_handles_row_boundaries() {
    let row_counts = vec![2, 1, 3];
    let y_offsets = vec![px(0.0), px(20.0), px(30.0)];
    let line_height = px(10.0);

    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(0.0)),
        0
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(19.0)),
        0
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(20.0)),
        1
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(30.0)),
        2
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(250.0)),
        2
    );
}

#[test]
fn line_display_columns_expands_tabs_to_tab_stops() {
    // No tabs: display columns equal the char/byte count.
    assert_eq!(line_display_columns(""), 0);
    assert_eq!(line_display_columns("abcd"), 4);

    // A leading tab advances to the first tab stop, not one column.
    assert_eq!(line_display_columns("\t"), TEXT_INPUT_WRAP_TAB_STOP_COLUMNS);
    // "ab\t" -> 2 columns, then advance to the next multiple of the tab stop.
    assert_eq!(
        line_display_columns("ab\t"),
        TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
    );
    // A tab landing exactly on a stop still advances a full tab width.
    assert_eq!(
        line_display_columns("abcd\t"),
        TEXT_INPUT_WRAP_TAB_STOP_COLUMNS * 2
    );
    // Several leading tabs (common source indentation).
    assert_eq!(
        line_display_columns("\t\tx"),
        TEXT_INPUT_WRAP_TAB_STOP_COLUMNS * 2 + 1
    );

    // ASCII fast path and the non-ASCII char scan agree on tab handling.
    for sample in ["", "\t", "a\tb", "ab\tcd\tef", "\t\t\t", "trailing-tab\t"] {
        let mut reference = 0usize;
        for ch in sample.chars() {
            if ch == '\t' {
                reference += TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
                    - (reference % TEXT_INPUT_WRAP_TAB_STOP_COLUMNS);
            } else {
                reference += 1;
            }
        }
        assert_eq!(line_display_columns(sample), reference, "sample={sample:?}");
    }
}

#[gpui::test]
fn content_width_cache_updates_only_edited_lines(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("short\nlongest-line\nmid", cx);
            input.set_content_width_layout(true);
            assert_eq!(input.content_width_max_units(), "longest-line\n".len());

            input.replace_utf8_range(6..18, "x", cx);
            assert_eq!(input.content_width_max_units(), "short\n".len());

            input.replace_utf8_range(0..5, "\t\twide", cx);
            assert_eq!(
                input.content_width_max_units(),
                TEXT_INPUT_WRAP_TAB_STOP_COLUMNS * 2 + "wide\n".len()
            );
        });
    });
}

#[gpui::test]
fn content_width_cache_tracks_line_splits_joins_and_undo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_content_width_layout(true);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                2
            );

            input.replace_utf8_range(2..2, "\n", cx);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                3
            );

            input.replace_utf8_range(2..3, "", cx);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                2
            );

            input.undo(&Undo, window, cx);
            assert_eq!(
                input.content_width_cache.as_ref().unwrap().line_units.len(),
                3
            );
        });
    });
}

#[test]
fn estimate_wrap_rows_for_line_handles_tabs_and_overflow() {
    assert_eq!(estimate_wrap_rows_for_line("abcd", 4), 1);
    assert_eq!(estimate_wrap_rows_for_line("abcde", 4), 2);
    assert_eq!(estimate_wrap_rows_for_line("a\tb", 4), 2);
}

#[test]
fn estimate_wrap_rows_for_line_matches_reference_for_ascii_tabs() {
    fn reference_wrap_rows_for_line(line_text: &str, wrap_columns: usize) -> usize {
        if line_text.is_empty() {
            return 1;
        }
        let wrap_columns = wrap_columns.max(1);
        let mut rows = 1usize;
        let mut column = 0usize;
        for ch in line_text.chars() {
            let width = if ch == '\t' {
                let rem = column % TEXT_INPUT_WRAP_TAB_STOP_COLUMNS;
                if rem == 0 {
                    TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
                } else {
                    TEXT_INPUT_WRAP_TAB_STOP_COLUMNS - rem
                }
            } else {
                1
            };

            if width >= wrap_columns {
                if column > 0 {
                    rows += 1;
                }
                rows += width / wrap_columns;
                column = width % wrap_columns;
                if column == 0 {
                    column = wrap_columns;
                }
                continue;
            }

            if column + width > wrap_columns {
                rows += 1;
                column = width;
            } else {
                column += width;
            }
        }
        rows.max(1)
    }

    let samples = [
        "",
        "\t",
        "a\tb",
        "ab\tcd\tef",
        "\tsection_00000\tvalue = token\ttoken\ttoken\ttoken\t",
        "token\ttoken\ttoken\ttoken\ttoken\t",
        "abcd",
        "abcde",
        "\t\t\t",
        "trailing-tab\t",
    ];

    for wrap_columns in (TEXT_INPUT_WRAP_TAB_STOP_COLUMNS + 1)..=12 {
        for sample in samples {
            assert_eq!(
                estimate_wrap_rows_for_line(sample, wrap_columns),
                reference_wrap_rows_for_line(sample, wrap_columns),
                "sample={sample:?}, wrap_columns={wrap_columns}"
            );
        }
    }
}

#[test]
fn expanded_dirty_wrap_line_range_for_edit_keeps_tab_affected_line_dirty() {
    let text = "ax\tbb\nnext";
    let starts = compute_line_starts(text);
    let dirty = expanded_dirty_wrap_line_range_for_edit(text, starts.as_slice(), &(1..1), &(1..2));
    assert_eq!(dirty, 0..1);
}

#[test]
fn apply_interpolated_wrap_patch_delta_adjusts_rows_by_delta() {
    let mut rows = vec![6, 5, 4, 3];
    let patch = InterpolatedWrapPatch {
        width_key: 80,
        line_start: 1,
        old_rows: vec![3, 2],
        new_rows: vec![5, 1],
    };
    apply_interpolated_wrap_patch_delta(rows.as_mut_slice(), &patch);
    assert_eq!(rows, vec![6, 7, 3, 3]);
}

#[test]
fn reset_interpolated_wrap_patches_on_overflow_requests_full_recompute() {
    let patch = InterpolatedWrapPatch {
        width_key: 80,
        line_start: 12,
        old_rows: vec![1],
        new_rows: vec![2],
    };

    let mut below_limit =
        vec![patch.clone(); TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES.saturating_sub(1)];
    let mut recompute_requested = false;
    assert!(!reset_interpolated_wrap_patches_on_overflow(
        &mut below_limit,
        &mut recompute_requested
    ));
    assert_eq!(
        below_limit.len(),
        TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES.saturating_sub(1)
    );
    assert!(!recompute_requested);

    let mut saturated = vec![patch; TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES];
    assert!(reset_interpolated_wrap_patches_on_overflow(
        &mut saturated,
        &mut recompute_requested
    ));
    assert!(saturated.is_empty());
    assert!(recompute_requested);
}

#[test]
fn pending_wrap_job_accepts_interpolated_patch_respects_prepaint_launch_gate() {
    let job = PendingWrapJob {
        sequence: 5,
        width_key: 120,
        line_count: 64,
        wrap_columns: 80,
    };

    assert!(pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        120,
        64,
        true
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        120,
        64,
        false
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        121,
        64,
        true
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        120,
        63,
        true
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        None, 120, 64, true
    ));
}

fn runs_fingerprint(runs: &[TextRun]) -> Vec<String> {
    runs.iter().map(|run| format!("{run:?}")).collect()
}

fn run_color_at_offset(runs: &[TextRun], offset: usize) -> gpui::Hsla {
    let mut cursor = 0usize;
    for run in runs {
        let end = cursor.saturating_add(run.len);
        if offset < end {
            return run.color;
        }
        cursor = end;
    }
    panic!("offset {offset} is outside the run coverage");
}

#[test]
fn highlight_runs_skip_hidden_overlap_end_boundaries() {
    let text = "abcdefghijklmnop";
    let line_starts = compute_line_starts(text);
    let style_low = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_mid = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_high = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights = vec![(0..10, style_low), (2..8, style_mid), (4..12, style_high)];
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts.as_slice(),
        0..1,
        highlights.as_slice(),
    );
    let legacy_runs = runs_for_line(&base_font, base_color, 0, text, Some(highlights.as_slice()));

    assert_eq!(streamed.line(0).unwrap_or(&[]).len(), 4);
    assert_eq!(legacy_runs.len(), 4);
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 1),
        style_low.color.expect("style_low color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 3),
        style_mid.color.expect("style_mid color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 6),
        style_high.color.expect("style_high color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 14),
        base_color
    );
}

#[test]
fn streamed_highlight_runs_match_legacy_visible_window() {
    let mut text = String::new();
    for ix in 0..160usize {
        text.push_str(format!("line_{ix:03}_abcdefghijklmnopqrstuvwxyz0123456789\n").as_str());
    }
    let line_starts = compute_line_starts(text.as_str());

    let style_a = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_b = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_c = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)> = Vec::new();
    for line_ix in 0..line_starts.len() {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_len = line_text_for_index(text.as_str(), line_starts.as_slice(), line_ix).len();
        if line_len < 24 {
            continue;
        }
        if line_ix % 2 == 0 {
            highlights.push((line_start + 1..line_start + 14, style_a));
        }
        if line_ix % 3 == 0 {
            highlights.push((line_start + 6..line_start + line_len.min(24), style_b));
        }
    }
    let wide_start = line_starts.get(18).copied().unwrap_or(0).saturating_add(2);
    let wide_end = line_starts
        .get(140)
        .copied()
        .unwrap_or(text.len())
        .saturating_add(20)
        .min(text.len());
    highlights.push((wide_start..wide_end, style_c));
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let visible_range = 47..121;
    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text.as_str(),
        line_starts.as_slice(),
        visible_range.clone(),
        highlights.as_slice(),
    );
    assert_eq!(streamed.len(), visible_range.len());

    for local_ix in 0..streamed.len() {
        let line_ix = visible_range.start + local_ix;
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(text.as_str(), line_starts.as_slice(), line_ix);
        let (capped, _) = truncate_line_for_shaping(line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
        let legacy_runs = runs_for_line(
            &base_font,
            base_color,
            line_start,
            capped.as_ref(),
            Some(highlights.as_slice()),
        );
        assert_eq!(
            runs_fingerprint(streamed.line(local_ix).unwrap_or(&[])),
            runs_fingerprint(legacy_runs.as_slice())
        );
    }
}

#[test]
fn streamed_highlight_runs_preserve_latest_overlap_precedence() {
    let text = "abcdefghijklmnop";
    let line_starts = compute_line_starts(text);
    let style_low = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_high = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights = vec![(2..12, style_low), (4..10, style_high)];
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts.as_slice(),
        0..1,
        highlights.as_slice(),
    );
    let legacy_runs = runs_for_line(&base_font, base_color, 0, text, Some(highlights.as_slice()));
    assert_eq!(
        runs_fingerprint(streamed.line(0).unwrap_or(&[])),
        runs_fingerprint(legacy_runs.as_slice())
    );

    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 3),
        style_low.color.expect("style_low color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 6),
        style_high.color.expect("style_high color should exist")
    );
}

#[test]
fn highlight_runs_single_carry_in_highlight_matches_streamed() {
    let text = "prefix highlight continues here\nsuffix line";
    let line_starts = compute_line_starts(text);
    let style = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.12, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights = vec![(3..30, style)];
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts.as_slice(),
        0..2,
        highlights.as_slice(),
    );

    for line_ix in 0..2 {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(text, line_starts.as_slice(), line_ix);
        let legacy_runs = runs_for_line(
            &base_font,
            base_color,
            line_start,
            line_text,
            Some(highlights.as_slice()),
        );
        assert_eq!(
            runs_fingerprint(streamed.line(line_ix).unwrap_or(&[])),
            runs_fingerprint(legacy_runs.as_slice())
        );
    }
}

#[test]
fn resolve_provider_highlights_caches_by_epoch_and_range() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    // Simulate the cache behavior without needing a full GPUI context.
    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch: u64 = 1;

    let h1 = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert!(!h1.pending);
    assert_eq!(h1.highlights.len(), 1);
    assert_eq!(h1.highlights[0].0, 0..100);

    // Same range and epoch → cached, no new call.
    let h2 = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&h1.highlights, &h2.highlights));

    // Contained range → cached, no new call.
    let h3 = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&h1.highlights, &h3.highlights));

    // Wider range → new call.
    let _h4 = test_resolve_with_cache(&mut cache, epoch, 0, 120, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    // Different epoch → new call even for same range.
    let _h5 = test_resolve_with_cache(&mut cache, epoch + 1, 0, 120, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

#[test]
fn resolve_provider_highlights_reuses_multiple_cached_ranges() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;

    let first = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    let second = test_resolve_with_cache(&mut cache, epoch, 200, 300, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    let first_subrange = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    assert!(Arc::ptr_eq(&first.highlights, &first_subrange.highlights));

    let second_subrange = test_resolve_with_cache(&mut cache, epoch, 220, 260, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    assert!(Arc::ptr_eq(&second.highlights, &second_subrange.highlights));

    let cache = cache.expect("resolved ranges should populate the provider cache");
    assert_eq!(cache.highlight_epoch, epoch);
    assert_eq!(cache.entries.len(), 2);
}

#[test]
fn resolve_provider_highlights_prefers_smallest_containing_cached_range() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;

    let narrow = test_resolve_with_cache(&mut cache, epoch, 50, 150, &provider);
    let wide = test_resolve_with_cache(&mut cache, epoch, 0, 200, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    let resolved = test_resolve_with_cache(&mut cache, epoch, 60, 140, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    assert!(
        Arc::ptr_eq(&resolved.highlights, &narrow.highlights),
        "the smallest cached containing slice should win even if a wider slice is newer"
    );
    assert!(
        !Arc::ptr_eq(&resolved.highlights, &wide.highlights),
        "the wider containing slice should not be reused when a tighter one exists"
    );

    let cache = cache.expect("resolved ranges should populate the provider cache");
    assert_eq!(cached_provider_ranges(&cache), vec![0..200, 50..150]);
}

#[test]
fn resolve_provider_highlights_cache_is_bounded() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;
    for window in 0..TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT {
        let start = window * 100;
        let end = start + 100;
        let _ = test_resolve_with_cache(&mut cache, epoch, start, end, &provider);
    }
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
    );

    let _ = test_resolve_with_cache(
        &mut cache,
        epoch,
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT * 100,
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT * 100 + 100,
        &provider,
    );
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT + 1
    );

    let cache_ref = cache.as_ref().expect("cache should retain recent ranges");
    assert_eq!(
        cache_ref.entries.len(),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
    );

    let _ = test_resolve_with_cache(&mut cache, epoch, 0, 50, &provider);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT + 2,
        "the oldest cached slice should be evicted once the cache reaches its bound"
    );
}

#[test]
fn resolve_provider_highlights_cache_hit_promotes_entry_before_eviction() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;

    let first = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    let _second = test_resolve_with_cache(&mut cache, epoch, 100, 200, &provider);
    let _third = test_resolve_with_cache(&mut cache, epoch, 200, 300, &provider);
    let _fourth = test_resolve_with_cache(&mut cache, epoch, 300, 400, &provider);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
    );

    let promoted = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 4);
    assert!(Arc::ptr_eq(&promoted.highlights, &first.highlights));

    let _fifth = test_resolve_with_cache(&mut cache, epoch, 400, 500, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 5);

    let cache_ref = cache
        .as_ref()
        .expect("cache should retain recent ranges after a bounded insert");
    assert_eq!(
        cached_provider_ranges(cache_ref),
        vec![200..300, 300..400, 0..100, 400..500]
    );

    let reused = test_resolve_with_cache(&mut cache, epoch, 10, 50, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 5);
    assert!(
        Arc::ptr_eq(&reused.highlights, &first.highlights),
        "a cache hit should keep the promoted slice resident across the next eviction"
    );

    let _evicted = test_resolve_with_cache(&mut cache, epoch, 120, 180, &provider);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        6,
        "the cold slice should be evicted instead of the recently-used one"
    );
}

#[test]
fn highlight_provider_binding_key_reuses_existing_provider_when_unchanged() {
    assert!(!should_reset_highlight_provider_binding(
        true,
        Some(41),
        Some(41)
    ));
}

#[test]
fn highlight_provider_binding_key_rebinds_when_missing_changed_or_unkeyed() {
    assert!(should_reset_highlight_provider_binding(
        false,
        Some(41),
        Some(41)
    ));
    assert!(should_reset_highlight_provider_binding(
        true,
        Some(41),
        Some(42)
    ));
    assert!(should_reset_highlight_provider_binding(
        true,
        Some(41),
        None
    ));
}

fn test_resolve_with_cache(
    cache: &mut Option<ProviderHighlightCache>,
    epoch: u64,
    byte_start: usize,
    byte_end: usize,
    provider: &HighlightProvider,
) -> ResolvedProviderHighlights {
    let requested_range = byte_start..byte_end;
    if let Some(resolved) = cache
        .as_mut()
        .and_then(|c| c.resolve(epoch, &requested_range))
    {
        return resolved;
    }
    let mut result = provider.resolve(requested_range.clone());
    result
        .highlights
        .sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    let pending = result.pending;
    let highlights = Arc::new(result.highlights);
    cache
        .get_or_insert_with(|| ProviderHighlightCache::new(epoch))
        .insert(epoch, requested_range, pending, Arc::clone(&highlights));
    ResolvedProviderHighlights {
        pending,
        highlights,
    }
}

fn make_counting_provider() -> (Arc<std::sync::atomic::AtomicUsize>, HighlightProvider) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let call_count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&call_count);
    let provider = HighlightProvider::from_fn(move |range: Range<usize>| {
        counter.fetch_add(1, Ordering::SeqCst);
        vec![(
            range,
            gpui::HighlightStyle {
                color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
                ..gpui::HighlightStyle::default()
            },
        )]
    });

    (call_count, provider)
}

fn cached_provider_ranges(cache: &ProviderHighlightCache) -> Vec<Range<usize>> {
    cache
        .entries
        .iter()
        .map(|entry| entry.byte_start..entry.byte_end)
        .collect()
}

fn text_input_test_position(input: &TextInput) -> Point<Pixels> {
    let bounds = input.layout.bounds.expect("expected text input bounds");
    let line_height = if input.layout.line_height.is_zero() {
        px(16.0)
    } else {
        input.layout.line_height
    };
    point(bounds.left() + px(2.0), bounds.top() + line_height / 2.0)
}

fn text_input_hotspot_ranges() -> Vec<Range<usize>> {
    vec![0..7]
}

#[gpui::test]
fn truncated_read_only_select_all_returns_full_source_text(cx: &mut gpui::TestAppContext) {
    let text = "0123456789abcdef0123456789abcdef";
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                read_only: true,
                chromeless: true,
                soft_wrap: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text(text, cx);
            input.set_display_truncation(Some(TextTruncationProfile::Middle), cx);
            input.select_all_text(cx);

            assert_eq!(input.selected_text(), Some(text.to_string()));
        });
    });
}

#[gpui::test]
fn hotspot_hit_test_finds_range_at_pointer_position(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    let position = cx.update(|_window, app| text_input_test_position(input.read(app)));
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(position, &text_input_hotspot_ranges());
        assert_eq!(hotspot, Some(0));
    });
}

#[gpui::test]
fn hotspot_hit_test_includes_right_side_of_final_glyph(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    let position = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("expected text input bounds");
        let line_height = if input.layout.line_height.is_zero() {
            px(16.0)
        } else {
            input.layout.line_height
        };
        let TextInputLayout::Plain(lines) = input
            .layout
            .last
            .as_ref()
            .expect("expected text input layout")
        else {
            panic!("expected plain text input layout");
        };
        let line = lines.get(0).expect("expected first shaped line");
        let final_glyph_left = line.x_for_index(6);
        let final_glyph_right = line.x_for_index(7);
        let final_glyph_width = final_glyph_right - final_glyph_left;
        let position = point(
            bounds.left() + final_glyph_left + (final_glyph_width * 3.0) / 4.0,
            bounds.top() + line_height / 2.0,
        );
        position
    });
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(position, &text_input_hotspot_ranges());
        assert_eq!(hotspot, Some(0));
    });
}

#[gpui::test]
fn hotspot_hit_test_returns_none_outside_bounds(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    let outside = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("expected text input bounds");
        point(bounds.left() + px(2.0), bounds.top() - px(2.0))
    });
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(outside, &text_input_hotspot_ranges());
        assert_eq!(hotspot, None);
    });
}

#[gpui::test]
fn hotspot_hit_test_ignores_trailing_blank_space_after_link(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee", cx);
        });
        let _ = window.draw(app);
    });

    let position = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("expected text input bounds");
        let line_height = if input.layout.line_height.is_zero() {
            px(16.0)
        } else {
            input.layout.line_height
        };
        let position = point(bounds.right() - px(2.0), bounds.top() + line_height / 2.0);
        assert!(bounds.contains(&position));
        assert_eq!(input.offset_for_position(position), 7);
        position
    });
    cx.update(|_window, app| {
        let hotspot = input
            .read(app)
            .hotspot_range_index_at_position(position, &text_input_hotspot_ranges());
        assert_eq!(hotspot, None);
    });
}

#[gpui::test]
fn hotspot_bounds_match_range_extent(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                read_only: true,
                chromeless: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("deadbee target", cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input
            .hotspot_bounds(&(0..7))
            .expect("expected hotspot bounds");
        assert!(bounds.size.width > px(0.0));
        assert!(bounds.size.height > px(0.0));
        assert!(bounds.contains(&text_input_test_position(&input)));
    });
}

#[gpui::test]
fn truncated_line_hit_testing_snaps_ellipsis_to_hidden_range_boundaries(
    cx: &mut gpui::TestAppContext,
) {
    let text: SharedString = "0123456789abcdef0123456789abcdef".into();
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let line = shape_truncated_line_cached(
            window,
            app,
            &window.text_style(),
            &text,
            Some(px(80.0)),
            TextTruncationProfile::Middle,
            &[],
            None,
        );

        assert!(line.truncated, "expected the line to truncate");
        let (hidden_range, display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(text.len() / 2)
            .expect("expected a middle ellipsis segment");

        let start_x = line.shaped_line.x_for_index(display_range.start);
        let end_x = line.shaped_line.x_for_index(display_range.end);
        let span = end_x - start_x;
        let left_x = start_x + span / 4.0;
        let right_x = start_x + (span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_x),
            hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_x),
            hidden_range.end
        );
    });
}

#[gpui::test]
fn focused_truncated_line_hit_testing_snaps_both_ellipsis_segments_to_hidden_boundaries(
    cx: &mut gpui::TestAppContext,
) {
    let text: SharedString = "prefix-aaaaaaaaaa-suffix".into();
    let focus = 7..17;
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let runs_four = vec![style.clone().to_run("…aaaa…".len())];
        let runs_five = vec![style.clone().to_run("…aaaaa…".len())];
        let width_four = window
            .text_system()
            .shape_line("…aaaa…".into(), font_size, &runs_four, None)
            .width;
        let width_five = window
            .text_system()
            .shape_line("…aaaaa…".into(), font_size, &runs_five, None)
            .width;
        let max_width = width_four + (width_five - width_four) / 2.0;

        let line = shape_truncated_line_cached(
            window,
            app,
            &style,
            &text,
            Some(max_width),
            TextTruncationProfile::Middle,
            &[],
            Some(focus),
        );

        assert!(line.truncated, "expected the line to truncate");

        let (left_hidden_range, left_display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(0)
            .expect("expected a left ellipsis segment");
        let (right_hidden_range, right_display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(text.len())
            .expect("expected a right ellipsis segment");

        assert_ne!(left_display_range, right_display_range);

        let left_x0 = line.shaped_line.x_for_index(left_display_range.start);
        let left_x1 = line.shaped_line.x_for_index(left_display_range.end);
        let left_span = left_x1 - left_x0;
        let left_inside = left_x0 + left_span / 4.0;
        let left_outside = left_x0 + (left_span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_inside),
            left_hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_outside),
            left_hidden_range.end
        );

        let right_x0 = line.shaped_line.x_for_index(right_display_range.start);
        let right_x1 = line.shaped_line.x_for_index(right_display_range.end);
        let right_span = right_x1 - right_x0;
        let right_inside = right_x0 + right_span / 4.0;
        let right_outside = right_x0 + (right_span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_inside),
            right_hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_outside),
            right_hidden_range.end
        );
    });
}

const TOKEN_COLOR: gpui::Hsla = gpui::Hsla {
    h: 0.33,
    s: 1.0,
    l: 0.5,
    a: 1.0,
};

/// A provider shaped like a real syntax one: it answers in the coordinates of
/// the text it was built over, not of whatever the buffer holds now.
fn token_highlight_provider(source: &str, token: &'static str) -> HighlightProvider {
    let source = source.to_string();
    HighlightProvider::from_fn(move |range: Range<usize>| {
        source
            .match_indices(token)
            .map(|(start, matched)| start..start + matched.len())
            .filter(|found| found.start < range.end && found.end > range.start)
            .map(|found| {
                (
                    found,
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )
            })
            .collect()
    })
}

/// What the highlighted byte ranges actually cover in the buffer's live text —
/// the only assertion that catches a smear, since a wrong offset is still a
/// perfectly plausible-looking number.
fn highlighted_slices(input: &mut TextInput, window: Range<usize>) -> Vec<String> {
    let text = input.text().to_string();
    input
        .debug_effective_highlights_for_range(window)
        .into_iter()
        .map(|(range, _)| text[range].to_string())
        .collect()
}

struct DualProviders {
    first_calls: Arc<std::sync::atomic::AtomicUsize>,
    second_calls: Arc<std::sync::atomic::AtomicUsize>,
    first_color: gpui::Hsla,
    second_color: gpui::Hsla,
    first: HighlightProvider,
    second: HighlightProvider,
}

fn make_dual_providers() -> DualProviders {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let first_color = gpui::hsla(0.0, 1.0, 0.5, 1.0);
    let second_color = gpui::hsla(0.66, 1.0, 0.5, 1.0);
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let fc = Arc::clone(&first_calls);
    let sc = Arc::clone(&second_calls);
    DualProviders {
        first_calls,
        second_calls,
        first_color,
        second_color,
        first: HighlightProvider::from_fn(move |range: Range<usize>| {
            fc.fetch_add(1, Ordering::SeqCst);
            vec![(
                range,
                gpui::HighlightStyle {
                    color: Some(first_color),
                    ..gpui::HighlightStyle::default()
                },
            )]
        }),
        second: HighlightProvider::from_fn(move |range: Range<usize>| {
            sc.fetch_add(1, Ordering::SeqCst);
            vec![(
                range,
                gpui::HighlightStyle {
                    color: Some(second_color),
                    ..gpui::HighlightStyle::default()
                },
            )]
        }),
    }
}

#[gpui::test]
fn multiline_shift_enter_inserts_a_line_break(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let expected = format!("alpha{}", input.line_ending);

            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), expected);
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should insert a newline instead of flagging enter-pressed"
            );
        });
    });
}

#[gpui::test]
fn multiline_submit_on_enter_keeps_shift_enter_as_line_break(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_submit_on_enter(true);
            input.set_text("alpha", cx);

            input.enter(&Enter, window, cx);

            assert_eq!(input.text(), "alpha");
            assert!(
                input.take_enter_pressed(),
                "enter should submit multiline inputs when submit-on-enter is enabled"
            );

            let expected = format!("alpha{}", input.line_ending);
            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), expected);
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should still insert a newline"
            );
        });
    });
}

#[gpui::test]
fn single_line_shift_enter_is_a_noop(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);

            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), "alpha");
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should not submit or modify single-line inputs"
            );
        });
    });
}

#[gpui::test]
fn escape_key_flags_escape_pressed(cx: &mut gpui::TestAppContext) {
    let (input, cx) =
        cx.add_window_view(|window, cx| TextInput::new(TextInputOptions::default(), window, cx));

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            assert!(
                !input.take_escape_pressed(),
                "escape should not be flagged before the key is pressed"
            );

            let keystroke = gpui::Keystroke::parse("escape")
                .expect("valid keystroke")
                .with_simulated_ime();
            let event = gpui::KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: false,
            };
            input.on_key_down(&event, _window, cx);

            assert!(
                input.take_escape_pressed(),
                "escape key should flag escape_pressed"
            );
            assert!(
                !input.take_escape_pressed(),
                "take_escape_pressed should consume the flag"
            );
        });
    });
}

#[gpui::test]
fn modified_escape_keystroke_does_not_flag_escape_pressed(cx: &mut gpui::TestAppContext) {
    let (input, cx) =
        cx.add_window_view(|window, cx| TextInput::new(TextInputOptions::default(), window, cx));

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let mut keystroke = gpui::Keystroke::parse("ctrl-escape")
                .expect("valid keystroke")
                .with_simulated_ime();
            keystroke.modifiers.control = true;
            let event = gpui::KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: false,
            };
            input.on_key_down(&event, _window, cx);

            assert!(
                !input.take_escape_pressed(),
                "modified escape should not flag escape_pressed"
            );
        });
    });
}

#[gpui::test]
fn single_line_enter_flags_enter_pressed(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            assert!(
                !input.take_enter_pressed(),
                "enter should not be flagged before the key is pressed"
            );

            input.enter(&Enter, window, cx);

            assert!(
                input.take_enter_pressed(),
                "enter key should flag enter_pressed in single-line inputs"
            );
            assert!(
                !input.take_enter_pressed(),
                "take_enter_pressed should consume the flag"
            );
        });
    });
}

#[gpui::test]
fn stable_highlight_provider_binding_key_preserves_existing_provider_and_cache(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let initial_resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(initial_resolved.highlights[0].1.color, Some(dp.first_color));

            let initial_cache = input
                .highlight
                .provider_cache
                .as_ref()
                .expect("initial resolve should populate the provider cache");
            assert_eq!(initial_cache.entries.len(), 1);
            let initial_entry = initial_cache
                .entries
                .last()
                .expect("initial cache should contain one provider slice");
            assert_eq!(initial_entry.byte_start, 0);
            assert_eq!(initial_entry.byte_end, 5);

            let initial_highlight_epoch = input.highlight.epoch;
            let initial_shape_epoch = input.layout.shape_style_epoch;
            let initial_cached_highlights = Arc::clone(&initial_entry.highlights);

            input.set_highlight_provider_with_key(41, dp.second.clone(), input.text().len(), cx);

            assert_eq!(
                input.highlight.epoch, initial_highlight_epoch,
                "reinstalling the same binding key should not invalidate provider highlights"
            );
            assert_eq!(
                input.layout.shape_style_epoch, initial_shape_epoch,
                "reinstalling the same binding key should not invalidate shaped rows"
            );

            let cache = input
                .highlight
                .provider_cache
                .as_ref()
                .expect("stable binding key should preserve the cached provider range");
            let cache_entry = cache
                .entries
                .last()
                .expect("stable binding key should keep the cached provider slice");
            assert!(
                Arc::ptr_eq(&cache_entry.highlights, &initial_cached_highlights),
                "stable binding key should preserve the existing cached highlight vector"
            );

            let resolved = input.resolve_provider_highlights(1, 4);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "stable binding key should keep using the original provider/cache"
            );
            assert_eq!(
                dp.second_calls.load(Ordering::SeqCst),
                0,
                "stable binding key should not bind a replacement provider"
            );
            assert!(Arc::ptr_eq(
                &resolved.highlights,
                &initial_cached_highlights
            ));
            assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
        });
    });
}

#[gpui::test]
fn replace_utf8_range_clears_shaped_row_caches(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                soft_wrap: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);

            input.layout.plain_line_cache.insert(
                ShapedRowCacheKey {
                    line_ix: 0,
                    font_size_key: 13,
                },
                ShapedLine::default(),
            );

            assert_eq!(input.layout.plain_line_cache.len(), 1);

            input.replace_utf8_range(0..5, "gamma", cx);

            assert!(
                input.layout.plain_line_cache.is_empty(),
                "text edits must invalidate cached plain shaped rows"
            );
        });
    });
}

#[gpui::test]
fn changed_highlight_provider_binding_key_rebinds_and_clears_cached_range(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);
            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            let previous_shape_epoch = input.layout.shape_style_epoch;

            input.set_highlight_provider_with_key(42, dp.second.clone(), input.text().len(), cx);

            assert!(
                input.highlight.provider_cache.is_none(),
                "changing the binding key should drop the cached provider range"
            );
            assert!(
                input.highlight.epoch > previous_highlight_epoch,
                "changing the binding key should invalidate provider highlight epochs"
            );
            assert!(
                input.layout.shape_style_epoch > previous_shape_epoch,
                "changing the binding key should invalidate shaped text caches"
            );

            let resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "rebinding should stop using the previous provider"
            );
            assert_eq!(
                dp.second_calls.load(Ordering::SeqCst),
                1,
                "rebinding should resolve highlights from the new provider"
            );
            assert_eq!(resolved.highlights[0].1.color, Some(dp.second_color));

            let cache = input
                .highlight
                .provider_cache
                .as_ref()
                .expect("resolving after a rebind should repopulate the provider cache");
            assert_eq!(cache.highlight_epoch, input.highlight.epoch);
            assert_eq!(cache.entries.len(), 1);
            let cache_entry = cache
                .entries
                .last()
                .expect("rebind resolve should cache the requested provider slice");
            assert_eq!(cache_entry.byte_start, 0);
            assert_eq!(cache_entry.byte_end, 5);
        });
    });
}

fn multiline_input(
    cx: &mut gpui::TestAppContext,
) -> (Entity<TextInput>, &mut gpui::VisualTestContext) {
    cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    })
}

#[gpui::test]
fn typing_keeps_provider_highlights_aligned(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                7,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );

            assert_eq!(
                highlighted_slices(input, 0..text.len()),
                vec!["incoming", "incoming"]
            );

            // Type inside the first line, ahead of both tokens, without letting
            // the owner's debounced provider rebuild run.
            input.replace_utf8_range(4..4, "X", cx);
            assert_eq!(
                input.text(),
                "let Xvalue = incoming;\n// keep incoming aligned\n"
            );

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"],
                "colors must stay pinned to their tokens between the edit and the recompute"
            );
        });
    });
}

#[gpui::test]
fn typing_a_newline_keeps_provider_highlights_aligned(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                7,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            let _ = highlighted_slices(input, 0..text.len());

            // The case that defeats reading the live text through the old
            // closure: the provider's line index no longer matches the buffer's,
            // so every line below would take its neighbour's tokens.
            input.replace_utf8_range(4..4, "\n", cx);
            assert_eq!(
                input.text(),
                "let \nvalue = incoming;\n// keep incoming aligned\n"
            );

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"],
                "an inserted newline must not smear the lines below it"
            );
        });
    });
}

#[gpui::test]
fn typing_multibyte_text_keeps_highlights_on_character_boundaries(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let café = incoming;\n// naïve incoming\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                7,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            let _ = highlighted_slices(input, 0..text.len());

            // Two multi-byte characters typed just before "café". Slicing the
            // result at all proves the mapped bounds are character boundaries;
            // a mid-character bound would panic here (and trips a debug assert
            // inside the mapping itself).
            input.replace_utf8_range(4..4, "üé", cx);
            assert_eq!(input.text(), "let üécafé = incoming;\n// naïve incoming\n");

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"]
            );
        });
    });
}

#[gpui::test]
fn typing_interpolates_statically_published_highlights(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("let value = incoming;", cx);
            let token = input.text().find("incoming").expect("token in fixture");
            input.set_highlights(
                vec![(
                    token..token + "incoming".len(),
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )],
                cx,
            );

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming"]
            );

            // Highlights published through `set_highlights` were never
            // invalidated on edit at all, so they smeared silently.
            input.replace_utf8_range(4..4, "X", cx);

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming"],
                "a static highlight vector rides along with edits like a provider's"
            );
        });
    });
}

#[gpui::test]
fn rebinding_a_provider_resets_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);
            input.replace_utf8_range(0..1, "AA", cx);
            assert!(!input.highlight.interpolation.is_exact());

            // A different key means a different closure, built over the text as
            // it stands now.
            input.set_highlight_provider_with_key(42, dp.second.clone(), input.text().len(), cx);
            assert!(
                input.highlight.interpolation.is_exact(),
                "a fresh provider describes the buffer verbatim"
            );

            input.replace_utf8_range(0..1, "BB", cx);
            assert!(!input.highlight.interpolation.is_exact());

            // The same key means the same closure over the same text, so its
            // anchor has to survive.
            input.set_highlight_provider_with_key(42, dp.second.clone(), input.text().len(), cx);
            assert!(
                !input.highlight.interpolation.is_exact(),
                "reapplying an unchanged binding must not move the anchor"
            );
        });
    });
}

#[gpui::test]
fn set_highlights_resets_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlights(
                vec![(
                    0..5,
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )],
                cx,
            );
            input.replace_utf8_range(0..1, "AA", cx);
            assert!(!input.highlight.interpolation.is_exact());

            input.set_highlights(
                vec![(
                    0..6,
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )],
                cx,
            );

            assert!(input.highlight.interpolation.is_exact());
            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["AAlpha"]
            );
        });
    });
}

#[gpui::test]
fn rebinding_under_a_new_key_resets_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
    // This is the contract the live syntax engine depends on. It re-syncs its
    // tree on the keystroke and rebinds under the document's new version, so
    // its ranges are already correct for the edited text. If interpolation
    // survived the rebind, those correct ranges would be mapped through the
    // edit a second time and land in the wrong place.
    let (input, cx) = multiline_input(cx);
    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(1, dp.first.clone(), input.text().len(), cx);
            let _ = input.effective_highlights_for_window(0..5);

            input.replace_utf8_range(0..1, "AA", cx);
            assert!(
                !input.highlight.interpolation.is_exact(),
                "an edit against a still-installed provider must be interpolated"
            );

            // What the live engine does next: rebind under a fresh key.
            input.set_highlight_provider_with_key(2, dp.second.clone(), input.text().len(), cx);

            assert!(
                input.highlight.interpolation.is_exact(),
                "a new binding key declares the provider current for the edited text"
            );
            assert_eq!(
                highlighted_slices(input, 0..6),
                vec!["AAlpha"],
                "ranges from the rebound provider must pass through unmapped"
            );
        });
    });
}

#[gpui::test]
fn a_never_pending_provider_does_not_accumulate_superseded_sources(cx: &mut gpui::TestAppContext) {
    // The live engine rebinds on every keystroke. Each rebind sets the outgoing
    // source aside to cover for a replacement that cannot answer yet; a
    // provider that always answers must clear that reserve instead of piling up
    // an Arc per keystroke.
    let (input, cx) = multiline_input(cx);
    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            for key in 1..=8u64 {
                let provider = if key % 2 == 0 {
                    dp.second.clone()
                } else {
                    dp.first.clone()
                };
                input.set_highlight_provider_with_key(key, provider, input.text().len(), cx);
                let resolved = input.effective_highlights_for_window(0..5);
                assert!(
                    !resolved.pending,
                    "a live-engine provider is always exact and never reports pending"
                );
                assert!(
                    input.highlight.superseded.is_none(),
                    "a settled source must release the one it replaced (key {key})"
                );
            }
        });
    });
}

#[gpui::test]
fn landing_background_syntax_chunks_preserves_the_highlight_interpolation(
    cx: &mut gpui::TestAppContext,
) {
    let (input, cx) = multiline_input(cx);

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);
            let _ = input.effective_highlights_for_window(0..5);
            input.replace_utf8_range(0..1, "AA", cx);

            let generation = input.highlight.interpolation.generation();
            let previous_epoch = input.highlight.epoch;

            // Chunks landing improve the provider's tokens; the text it was
            // built over is unchanged, so its anchor must not move.
            input.note_provider_highlights_changed();

            assert!(!input.highlight.interpolation.is_exact());
            assert_eq!(input.highlight.interpolation.generation(), generation);
            assert!(
                input.highlight.epoch > previous_epoch,
                "better tokens are a reason to refetch"
            );
            assert!(input.highlight.interpolated_cache.is_none());

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, _)| range.clone())
                    .collect::<Vec<_>>(),
                vec![2..5],
                "the refetched tokens arrive shifted by the same +1 the edit caused"
            );
        });
    });
}

/// A provider whose tokens are built in the background: it answers `pending`
/// with nothing until `ready` is flipped, the way a freshly prepared syntax
/// document does.
fn deferred_token_provider(
    source: &str,
    token: &'static str,
    ready: Arc<std::sync::atomic::AtomicBool>,
) -> HighlightProvider {
    use std::sync::atomic::Ordering;

    let source = source.to_string();
    let resolve_ready = Arc::clone(&ready);
    let has_pending_ready = Arc::clone(&ready);
    HighlightProvider::with_pending(
        move |range: Range<usize>| {
            if !resolve_ready.load(Ordering::SeqCst) {
                return HighlightProviderResult {
                    highlights: Vec::new(),
                    pending: true,
                };
            }
            HighlightProviderResult {
                highlights: source
                    .match_indices(token)
                    .map(|(start, matched)| start..start + matched.len())
                    .filter(|found| found.start < range.end && found.end > range.start)
                    .map(|found| {
                        (
                            found,
                            gpui::HighlightStyle {
                                color: Some(TOKEN_COLOR),
                                ..gpui::HighlightStyle::default()
                            },
                        )
                    })
                    .collect(),
                pending: false,
            }
        },
        || 0,
        move || !has_pending_ready.load(Ordering::SeqCst),
    )
}

#[gpui::test]
fn rebinding_to_a_provider_that_cannot_answer_yet_keeps_the_previous_colors(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                1,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            assert_eq!(
                highlighted_slices(input, 0..text.len()),
                vec!["incoming", "incoming"]
            );

            // The user types; the owner's debounce then recomputes and rebinds
            // to a provider over a freshly prepared document, whose token
            // chunks are still being built in the background.
            input.replace_utf8_range(4..4, "X", cx);
            let edited = input.text().to_string();
            let ready = Arc::new(AtomicBool::new(false));
            input.set_highlight_provider_with_key(
                2,
                deferred_token_provider(&edited, "incoming", Arc::clone(&ready)),
                edited.len(),
                cx,
            );

            // Without the handoff this window renders in the base color — the
            // whole output flashing white for a frame or two.
            assert_eq!(
                highlighted_slices(input, 0..edited.len()),
                vec!["incoming", "incoming"],
                "the outgoing source must cover for its replacement until it can answer"
            );

            // Chunks land; the new provider takes over and the fallback is let go.
            ready.store(true, Ordering::SeqCst);
            input.note_provider_highlights_changed();

            assert_eq!(
                highlighted_slices(input, 0..edited.len()),
                vec!["incoming", "incoming"]
            );
            assert!(
                input.highlight.superseded.is_none(),
                "a settled source needs no stand-in"
            );
        });
    });
}

#[gpui::test]
fn a_provider_that_never_answered_does_not_become_the_fallback(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::AtomicBool;

    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                1,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            let _ = highlighted_slices(input, 0..text.len());

            // Two rebinds in a row, neither replacement ready — the second must
            // not push the good source out in favour of the first, which never
            // managed to say anything.
            for key in [2u64, 3] {
                input.replace_utf8_range(4..4, "X", cx);
                let edited = input.text().to_string();
                input.set_highlight_provider_with_key(
                    key,
                    deferred_token_provider(&edited, "incoming", Arc::new(AtomicBool::new(false))),
                    edited.len(),
                    cx,
                );
            }

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"],
                "the last source that actually answered stays the fallback"
            );
        });
    });
}

#[gpui::test]
fn replace_utf8_range_keeps_cached_provider_highlights_and_interpolates(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            // One byte becomes two, so everything the provider described from
            // byte 1 onward now sits one byte further along.
            let inserted = input.replace_utf8_range(0..1, "AA", cx);
            assert_eq!(inserted, 0..2);
            assert_eq!(input.text(), "AAlpha\nbeta");
            assert!(
                input.highlight.provider_cache.is_some(),
                "an edit is interpolated over, so the cached provider range survives it"
            );
            assert_eq!(
                input.highlight.epoch, previous_highlight_epoch,
                "an edit does not change what the provider describes"
            );

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "the same window maps back into the range already fetched"
            );
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(2..6, Some(dp.first_color))],
                "the provider's 0..5 span keeps its tail, shifted past the insert"
            );
        });
    });
}

#[gpui::test]
fn set_text_records_an_edit_delta_for_highlights(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            // A rewrite that appends one byte at offset 5, which `set_text`
            // diffs out of the two texts rather than invalidating blindly.
            input.set_text("alphaX\nbeta", cx);

            assert!(
                !input.highlight.interpolation.is_exact(),
                "set_text must record what it changed, not silently move the text out from under the highlights"
            );
            assert!(
                input.highlight.provider_cache.is_some(),
                "the cached provider range still describes the text it was fetched for"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(0..5, Some(dp.first_color))],
                "highlights entirely ahead of the edit are untouched"
            );
        });
    });
}

#[gpui::test]
fn undo_composes_into_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;

            let inserted = input.replace_utf8_range(0..1, "AA", cx);
            assert_eq!(inserted, 0..2);
            assert!(!input.highlight.interpolation.is_exact());

            input.undo(&Undo, window, cx);

            assert_eq!(input.text(), "alpha\nbeta");
            // The undo composes into the patch like any other edit. It restores
            // the provider's own text, but the interpolation tracks which bytes
            // were touched, not what they now say, so byte 0 stays marked.
            assert!(!input.highlight.interpolation.is_exact());
            assert!(
                input.highlight.provider_cache.is_some(),
                "the restored text is the text the cached range was fetched for"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "returning to the provider's own text should not re-query it"
            );
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(1..5, Some(dp.first_color))],
                "offsets are back where they started; only the touched byte waits \
                 for the debounced recompute"
            );
        });
    });
}

#[gpui::test]
fn redo_restores_text_after_undo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);
            assert_eq!(input.text(), "beta");

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "beta");
            assert!(input.selection.redo_stack.is_empty());
        });
    });
}

#[gpui::test]
fn edit_delta_queue_retains_multiple_edits_and_undo_redo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("abcd", cx);
            input.replace_utf8_range(1..2, "XX", cx);
            input.replace_utf8_range(3..4, "", cx);
            assert_eq!(input.text(), "aXXd");
            assert_eq!(
                input.drain_recent_utf8_edit_deltas(),
                vec![(1..2, 1..3), (3..4, 3..3)],
            );

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "aXXcd");
            assert_eq!(input.drain_recent_utf8_edit_deltas(), vec![(3..3, 3..4)],);

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "aXXd");
            assert_eq!(input.drain_recent_utf8_edit_deltas(), vec![(3..4, 3..3)],);
        });
    });
}

#[gpui::test]
fn redo_is_cleared_by_a_new_edit_after_undo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);

            let inserted = input.replace_utf8_range(0..5, "gamma", cx);
            assert_eq!(inserted, 0..5);
            assert_eq!(input.text(), "gamma");
            assert!(input.selection.redo_stack.is_empty());

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "gamma");
            assert!(input.selection.redo_stack.is_empty());
        });
    });
}

#[gpui::test]
fn redo_is_noop_when_input_is_read_only(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);

            input.set_read_only(true, cx);
            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);
        });
    });
}

#[gpui::test]
fn replace_text_in_range_keeps_cached_provider_highlights_and_interpolates(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            input.replace_text_in_range(Some(0..1), "AA", window, cx);

            assert_eq!(input.text(), "AAlpha\nbeta");
            assert!(
                input.highlight.provider_cache.is_some(),
                "IME replace_text_in_range is interpolated over like any other edit"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(2..6, Some(dp.first_color))],
            );
        });
    });
}

#[gpui::test]
fn replace_and_mark_text_in_range_keeps_cached_provider_highlights_and_interpolates(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            input.replace_and_mark_text_in_range(Some(0..1), "AA", None, window, cx);

            assert_eq!(input.text(), "AAlpha\nbeta");
            assert_eq!(input.selection.marked_range, Some(0..2));
            assert!(
                input.highlight.provider_cache.is_some(),
                "an IME composition is interpolated over like any other edit"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(2..6, Some(dp.first_color))],
            );
        });
    });
}

#[test]
fn highlight_provider_with_pending_uses_custom_callbacks() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let pending = Arc::new(AtomicBool::new(true));
    let pending_for_resolve = Arc::clone(&pending);
    let pending_for_check = Arc::clone(&pending);
    let drain_calls = Arc::new(AtomicUsize::new(0));
    let drain_calls_for_provider = Arc::clone(&drain_calls);
    let provider = HighlightProvider::with_pending(
        move |range: Range<usize>| HighlightProviderResult {
            highlights: vec![(
                range,
                gpui::HighlightStyle {
                    color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
                    ..gpui::HighlightStyle::default()
                },
            )],
            pending: pending_for_resolve.load(Ordering::SeqCst),
        },
        move || {
            drain_calls_for_provider.fetch_add(1, Ordering::SeqCst);
            pending.store(false, Ordering::SeqCst);
            1
        },
        move || pending_for_check.load(Ordering::SeqCst),
    );

    let first = provider.resolve(4..12);
    assert!(first.pending);
    assert_eq!(first.highlights[0].0, 4..12);
    assert!(provider.has_pending());
    assert_eq!(provider.drain_pending(), 1);
    assert_eq!(drain_calls.load(Ordering::SeqCst), 1);
    assert!(!provider.has_pending());

    let second = provider.resolve(4..12);
    assert!(!second.pending);
}

const PROTECTED_SAMPLE_TEXT: &str = "head\n<Merge Conflict>\ntail\n";

fn protected_sample_span() -> Range<usize> {
    let start = PROTECTED_SAMPLE_TEXT
        .find("<Merge Conflict>")
        .expect("placeholder line");
    start..start + "<Merge Conflict>\n".len()
}

fn protected_sample_input(
    cx: &mut gpui::TestAppContext,
) -> (Entity<TextInput>, &mut gpui::VisualTestContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });
    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text(PROTECTED_SAMPLE_TEXT, cx);
            input.set_protected_ranges(Arc::from([protected_sample_span()]));
        });
    });
    (input, cx)
}

#[gpui::test]
fn protected_ranges_reject_typed_edits_that_would_alter_them(cx: &mut gpui::TestAppContext) {
    let span = protected_sample_span();
    let (input, cx) = protected_sample_input(cx);

    for (range, inserted) in [
        // Typing inside the line, over it, and at its first offset.
        (span.start + 2..span.start + 2, "x"),
        (span.start..span.end, "picked\n"),
        (span.start..span.start, "x"),
        // Backspace at the line start, which would join the line above into it.
        (span.start - 1..span.start, ""),
        // Replacing the line above with text that no longer ends the line.
        (0..span.start, "head"),
    ] {
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.replace_text_in_range(Some(range.clone()), inserted, window, cx);
                assert_eq!(
                    input.text(),
                    PROTECTED_SAMPLE_TEXT,
                    "replacing {range:?} with {inserted:?} must be refused"
                );
                assert_eq!(input.protected_ranges(), std::slice::from_ref(&span));
            });
        });
    }
}

#[gpui::test]
fn protected_ranges_ride_along_with_edits_around_them(cx: &mut gpui::TestAppContext) {
    let span = protected_sample_span();
    let (input, cx) = protected_sample_input(cx);

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            // Editing the line after the span leaves it where it was.
            input.replace_text_in_range(Some(span.end..span.end), "new\n", window, cx);
            assert_eq!(input.text(), "head\n<Merge Conflict>\nnew\ntail\n");
            assert_eq!(input.protected_ranges(), std::slice::from_ref(&span));

            // Editing before it moves it by the length the edit added.
            input.replace_text_in_range(Some(0..0), "top\n", window, cx);
            assert_eq!(input.text(), "top\nhead\n<Merge Conflict>\nnew\ntail\n");
            let moved = span.start + "top\n".len()..span.end + "top\n".len();
            assert_eq!(input.protected_ranges(), std::slice::from_ref(&moved));

            // Deleting the whole line above keeps the placeholder a line of its
            // own, so that edit goes through.
            let span = input.protected_ranges()[0].clone();
            input.replace_text_in_range(
                Some(span.start - "head\n".len()..span.start),
                "",
                window,
                cx,
            );
            assert_eq!(input.text(), "top\n<Merge Conflict>\nnew\ntail\n");
            let moved = span.start - "head\n".len()..span.end - "head\n".len();
            assert_eq!(input.protected_ranges(), std::slice::from_ref(&moved));
        });
    });
}
