use super::*;
use unicode_segmentation::UnicodeSegmentation as _;

/// Caret steps segment only the chunks around the caret, which must agree with
/// the whole document everywhere: CR LF, combining marks, ZWJ emoji, flag
/// pairs and Indic conjuncts, at row edges and split across rope chunk seams.
/// The dense text is shifted through one whole period, and the test checks that
/// a seam really lands inside each kind of cluster.
#[gpui::test]
fn caret_steps_agree_with_whole_document_graphemes(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let unit = "a\r\nb\u{301}\n👩\u{200d}👩\u{200d}👧\r\n🇫🇮🇸🇪\n\ne\u{301}\u{302}x\r\n";
    let dense = "👩\u{200d}👩\u{200d}👧🇫🇮🇸🇪e\u{301}\u{302}क्ष\r\n";
    let texts = std::iter::once(unit.to_owned())
        .chain((0..dense.len()).map(|pad| format!("{}{}", "x".repeat(pad), dense.repeat(60))));
    let mut split = std::collections::BTreeSet::new();
    for text in texts {
        let starts: Vec<usize> = text.grapheme_indices(true).map(|(ix, _)| ix).collect();
        cx.update(|_, app| {
            input.update(app, |input, cx| {
                input.set_text(text.as_str(), cx);
                let rope = input.content.snapshot().rope();
                let mut seam = 0;
                for chunk in rope.chunks() {
                    seam += chunk.len();
                    let next = starts.partition_point(|start| *start < seam);
                    if next > 0 && seam < text.len() && starts.get(next) != Some(&seam) {
                        let end = starts.get(next).copied().unwrap_or(text.len());
                        split.insert(cluster_kind(&text[starts[next - 1]..end]));
                    }
                }
                for offset in 0..=text.len() {
                    let before = starts.partition_point(|ix| *ix < offset);
                    let after = starts.partition_point(|ix| *ix <= offset);
                    assert_eq!(
                        input.previous_boundary(offset),
                        before.checked_sub(1).map_or(0, |ix| starts[ix]),
                        "before {offset}"
                    );
                    assert_eq!(
                        input.next_boundary(offset),
                        starts.get(after).copied().unwrap_or(text.len()),
                        "after {offset}"
                    );
                }
            });
        });
    }
    for kind in [
        "ZWJ family",
        "flag pair",
        "combining marks",
        "conjunct",
        "CR LF",
    ] {
        assert!(split.contains(kind), "no chunk seam fell inside a {kind}");
    }
}

/// Which kind of cluster the seam test split, by the codepoints that join it.
fn cluster_kind(cluster: &str) -> &'static str {
    if cluster.contains('\u{200d}') {
        "ZWJ family"
    } else if cluster.contains('\u{94d}') {
        "conjunct"
    } else if cluster.contains('\u{301}') {
        "combining marks"
    } else if cluster == "\r\n" {
        "CR LF"
    } else if cluster
        .chars()
        .all(|ch| ('\u{1f1e6}'..='\u{1f1ff}').contains(&ch))
    {
        "flag pair"
    } else {
        "other"
    }
}

/// Arrow keys, Backspace and Delete after an edit read the text around the
/// caret, not a flattened copy of the document; at 4.8 MB that copy plus a
/// scan from the far end took 23-29 ms per key.
#[gpui::test]
fn caret_steps_after_an_edit_do_not_flatten_the_document(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let text = "let value = compute(alpha, beta);\n".repeat(2_000);
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text.as_str(), cx);
            let middle = text.len() / 2;
            input.replace_utf8_range(middle..middle, "x", cx);

            input.previous_boundary(middle);
            input.next_boundary(middle);

            let content = input.content.snapshot();
            assert!(!content.is_materialized());
            assert!(!content.is_line_index_built());
        });
    });
}

/// Home and End stop at the row's own newline, CR included in the row.
#[gpui::test]
fn row_bounds_stop_at_the_rows_newlines(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let text = "first\r\n\nthird row\nlast";
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text, cx);
            for offset in 0..=text.len() {
                let start = text[..offset].rfind('\n').map_or(0, |ix| ix + 1);
                let end = text[offset..]
                    .find('\n')
                    .map_or(text.len(), |ix| offset + ix);
                assert_eq!(
                    input.logical_row_boundaries(offset),
                    (start, end),
                    "at {offset}"
                );
            }
        });
    });
}

/// Home, End and the protected-line check read one row or one byte, so after
/// an edit they leave the document unflattened.
#[gpui::test]
fn row_bounds_and_protected_checks_do_not_flatten_the_document(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let row = "let value = compute(alpha, beta);\n";
    let text = row.repeat(2_000);
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text.as_str(), cx);
            let protected = row.len() * 10..row.len() * 11;
            input.set_protected_ranges(Arc::from([protected.clone()]));
            input.replace_utf8_range(0..0, "x", cx);
            let protected = protected.start + 1..protected.end + 1;

            input.logical_row_boundaries(text.len() / 2);
            assert!(input.edit_alters_protected_range(&(protected.start - 3..protected.start), ""));

            let content = input.content.snapshot();
            assert!(!content.is_materialized());
            assert!(!content.is_line_index_built());
        });
    });
}

/// The word moves as they ran on a flattened copy of the text.
fn word_moves_on_flat_text(text: &str, offset: usize) -> (usize, usize) {
    let left = |mut offset: usize, keep: &dyn Fn(char) -> bool| {
        while let Some((ix, ch)) = text[..offset].char_indices().next_back() {
            if !keep(ch) {
                break;
            }
            offset = ix;
        }
        offset
    };
    let right = |mut offset: usize, keep: &dyn Fn(char) -> bool| {
        while let Some(ch) = text[offset..].chars().next() {
            if !keep(ch) {
                break;
            }
            offset += ch.len_utf8();
        }
        offset
    };
    let space = |ch: char| ch.is_whitespace();
    let word = crate::text_selection::is_word_char;
    let symbol = |ch: char| !ch.is_whitespace() && !crate::text_selection::is_word_char(ch);
    let start = left(left(left(left(offset, &space), &symbol), &space), &word);
    let end = match text[offset..].chars().next() {
        None => text.len(),
        Some(ch) if space(ch) => right(offset, &space),
        Some(ch) if word(ch) => right(offset, &word),
        Some(_) => right(offset, &symbol),
    };
    (start, end)
}

/// Word moves cross rows (a newline is whitespace) and rope chunk seams (a
/// word or a gap longer than a chunk), and must land where they did on the
/// flattened text.
#[gpui::test]
fn word_moves_cross_rows_like_the_flat_text(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let word = "w".repeat(700);
    let gap = " ".repeat(600);
    let text = format!("fn alpha(b: u8) {{\r\n\n   \n  šeta_2 = ö.ä;\n}}\n{word}{gap}.{word}\n");
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text.as_str(), cx);
            for offset in (0..=text.len()).filter(|ix| text.is_char_boundary(*ix)) {
                assert_eq!(
                    (
                        input.previous_word_start(offset),
                        input.next_word_end(offset)
                    ),
                    word_moves_on_flat_text(&text, offset),
                    "at {offset}"
                );
            }
        });
    });
}

/// An offset inside a character walks from that character's start, so a word
/// move lands on a boundary inside the text.
#[gpui::test]
fn word_moves_from_inside_a_character_start_at_it(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text("aé", cx);

            assert_eq!(input.previous_word_start(2), 0);
            assert_eq!(input.next_word_end(2), 3);
        });
    });
}

/// Word moves after an edit read the characters they pass, not a flattened
/// copy of the document.
#[gpui::test]
fn word_moves_after_an_edit_do_not_flatten_the_document(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let text = "let value = compute(alpha, beta);\n".repeat(2_000);
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text.as_str(), cx);
            let middle = text.len() / 2;
            input.replace_utf8_range(middle..middle, "x", cx);

            input.previous_word_start(middle);
            input.next_word_end(middle);

            let content = input.content.snapshot();
            assert!(!content.is_materialized());
            assert!(!content.is_line_index_built());
        });
    });
}

/// On one long row, a minified file, a word move copies nothing: reading the
/// row as slices cost a copy of it up to the caret per skip.
#[gpui::test]
fn word_moves_on_a_long_row_copy_nothing(cx: &mut gpui::TestAppContext) {
    use crate::text_model::OWNED_SLICES;
    let (input, cx) = multiline_input(cx);
    let text = "alpha beta.gamma ".repeat(2_000);
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text.as_str(), cx);
            let middle = text.len() / 2;
            input.replace_utf8_range(middle..middle, "x", cx);
            OWNED_SLICES.with(|count| count.set(0));

            input.previous_word_start(middle);
            input.next_word_end(middle);

            assert_eq!(OWNED_SLICES.with(|count| count.get()), 0);
        });
    });
}

/// On one long row, a minified file, a caret step copies nothing: reading the
/// caret's row as a slice copied the whole row on every key, 39.6 ms at 4.8 MB.
#[gpui::test]
fn caret_steps_on_a_long_row_copy_nothing(cx: &mut gpui::TestAppContext) {
    use crate::text_model::OWNED_SLICES;
    let (input, cx) = multiline_input(cx);
    let text = "alpha beta.gamma ".repeat(2_000);
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text.as_str(), cx);
            let middle = text.len() / 2;
            input.replace_utf8_range(middle..middle, "x", cx);
            OWNED_SLICES.with(|count| count.set(0));

            input.previous_boundary(middle);
            input.next_boundary(middle);

            assert_eq!(OWNED_SLICES.with(|count| count.get()), 0);
        });
    });
}

// The identical-replacement shortcut compared by copying the replaced range
// first, so select-all and typing copied the whole buffer on every keystroke.
#[gpui::test]
fn replacing_a_large_selection_does_not_copy_it(cx: &mut gpui::TestAppContext) {
    use crate::text_model::OWNED_SLICES;
    let (input, cx) = multiline_input(cx);
    let text = "let value = compute(alpha, beta);\n".repeat(20_000);
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(text.as_str(), cx);
            OWNED_SLICES.with(|count| count.set(0));
            input.replace_utf8_range(0..text.len(), "x", cx);
            assert_eq!(input.text(), "x");
            assert_eq!(OWNED_SLICES.with(|count| count.get()), 0);
        })
    });
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

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(text, cx);
            input.set_display_truncation(Some(TextTruncationProfile::Middle), cx);
            input.select_all_text(window, cx);

            assert_eq!(input.selected_text(), Some(text.to_string()));
        });
    });
}

#[gpui::test]
fn read_only_appends_preserve_selection_direction_and_drag(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_read_only(true, cx);
            input.set_text("héllo\nworld", cx);
            input.set_selected_range(1..6, false, window, cx);
            input.selection.reversed = true;
            input.interaction.is_selecting = true;
            input.interaction.mouse_selection_anchor = Some(6);
            input.layout.scroll_x = px(12.0);
            input.set_text_preserving_selection_on_append("héllo\nworld\nnext", cx);
            assert_eq!(input.selected_text().as_deref(), Some("éllo"));
            assert!(input.selection.reversed);
            assert!(input.interaction.is_selecting);
            assert_eq!(input.interaction.mouse_selection_anchor, Some(6));
            assert_eq!(input.layout.scroll_x, px(12.0));
            assert!(!input.selection_owner.is_stale(cx));

            input.set_text_preserving_selection_on_append("world\nnext", cx);
            assert!(input.selected_range().is_empty());
            assert!(!input.interaction.is_selecting);
            assert_eq!(input.interaction.mouse_selection_anchor, None);

            input.set_selected_range(0..5, false, window, cx);
            input.set_text("world\nnext\nordinary setter", cx);
            assert!(
                input.selected_range().is_empty(),
                "ordinary setters retain their behavior"
            );
        });
    });
}

#[gpui::test]
fn platform_typing_after_undo_discards_redo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            input.replace_text_in_range(Some(0..5), "beta", window, cx);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");

            input.replace_text_in_range(Some(0..5), "gamma", window, cx);
            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "gamma");
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
        });
    });
}

#[gpui::test]
fn platform_composition_after_undo_discards_redo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            input.replace_text_in_range(Some(0..5), "beta", window, cx);
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");

            input.replace_and_mark_text_in_range(Some(0..5), "猫", None, window, cx);
            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "猫");
            assert_eq!(input.marked_text_range(window, cx), Some(0..1));
            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
        });
    });
}

#[gpui::test]
fn platform_composition_selection_uses_replacement_local_utf16(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            for (before, replaced, inserted, selected, after, bytes, marked) in [
                ("world", 0..5, "猫", 0..1, "猫", 0..3, 0..1),
                ("😀old!", 2..5, "猫犬", 1..2, "😀猫犬!", 7..10, 2..4),
                ("aold!", 1..4, "😀猫", 2..3, "a😀猫!", 5..8, 1..4),
                ("猫old!", 1..4, "😀", 2..2, "猫😀!", 7..7, 1..3),
            ] {
                input.set_text(before, cx);
                input.replace_and_mark_text_in_range(
                    Some(replaced),
                    inserted,
                    Some(selected),
                    window,
                    cx,
                );
                assert_eq!(input.text(), after);
                assert_eq!(input.selected_range(), bytes);
                assert_eq!(input.marked_text_range(window, cx), Some(marked));
                assert!(input.text().is_char_boundary(input.selected_range().start));
                assert!(input.text().is_char_boundary(input.selected_range().end));
            }
        });
    });
}

/// A programmatic highlight (the file editor's search reveal) must own the
/// window's selection, or the next neutral gesture reaps it.
#[gpui::test]
fn a_programmatic_selection_survives_a_preserved_press(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello world", cx);
            input.set_selected_range(0..5, false, window, cx);
        });
    });
    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        Some("hello".to_string()),
        "precondition: the range was installed"
    );

    // A scrollbar drag: writes the global, must not disturb any selection.
    cx.update(|_window, app| crate::text_selection_owner::preserve(app));
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        Some("hello".to_string()),
        "a selection-neutral gesture must not collapse a programmatic selection"
    );
}

/// Undo restores a previously selected range; that restored highlight must own
/// the window's selection too.
#[gpui::test]
fn an_undone_selection_survives_a_preserved_press(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello world", cx);
            input.select_all_text(window, cx);
            input.replace_text_in_range(None, "x", window, cx);
        });
    });
    // Ownership moves elsewhere first -- click into another surface -- which is
    // what leaves the restored range with a stale token.
    cx.update(|window, app| {
        let mut elsewhere = crate::text_selection_owner::SelectionOwnerToken::default();
        elsewhere.adopt(window, app);
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        input.update(app, |input, cx| input.undo(&Undo, window, cx));
    });
    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        Some("hello world".to_string()),
        "precondition: undo restored the text and its selection"
    );

    cx.update(|_window, app| crate::text_selection_owner::preserve(app));
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        Some("hello world".to_string()),
        "a selection-neutral gesture must not collapse an undone selection"
    );
}

/// Backspace paints no highlight, so it must not take the window's selection.
#[gpui::test]
fn deleting_a_character_does_not_steal_another_surfaces_selection(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello world", cx);
            input.set_caret(5, cx);
        });
    });

    let elsewhere = cx.update(|window, app| {
        let mut owner = crate::text_selection_owner::SelectionOwnerToken::default();
        owner.adopt(window, app);
        owner
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| input.backspace(&Backspace, window, cx));
    });
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_window, app| input.read(app).text().to_string()),
        "hell world",
        "precondition: the backspace actually deleted a character"
    );
    assert!(
        cx.update(|_window, app| !elsewhere.is_stale(app)),
        "a delete keystroke must leave the other surface's selection owned"
    );
}

/// The observer skips a composing input, so the check re-runs when it ends.
#[gpui::test]
fn ending_an_ime_composition_clears_a_selection_that_went_stale(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello world", cx);
            input.select_all_text(window, cx);
            input.selection.marked_range = Some(0..1);
        });
    });

    cx.update(|window, app| {
        let mut elsewhere = crate::text_selection_owner::SelectionOwnerToken::default();
        elsewhere.adopt(window, app);
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        Some("hello world".to_string()),
        "a composing input keeps its highlight while the IME owns it"
    );

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            gpui::EntityInputHandler::unmark_text(input, window, cx)
        });
    });

    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        None,
        "once composition ends the stale highlight must go"
    );
}

/// `select_mouse_to_index` writes a range without adopting; it is safe only
/// because the mouse-down that started the drag adopted first.
#[gpui::test]
fn a_dragged_selection_survives_a_preserved_press(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello world", cx);
        });
        let _ = window.draw(app);
    });
    let (anchor, target) = cx.update(|_window, app| {
        let input = input.read(app);
        let bounds = input.layout.bounds.expect("text input bounds");
        let y = bounds.center().y;
        let position_for_offset = |wanted| {
            (0..f32::from(bounds.size.width).ceil() as usize)
                .map(|x| point(bounds.left() + px(x as f32), y))
                .find(|position| input.index_for_mouse_position(*position) == wanted)
                .unwrap_or_else(|| panic!("a hit position for offset {wanted}"))
        };
        (position_for_offset(0), position_for_offset(5))
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.on_mouse_down(
                &MouseDownEvent {
                    position: anchor,
                    modifiers: gpui::Modifiers::default(),
                    button: MouseButton::Left,
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
        });
    });
    cx.simulate_mouse_move(target, Some(MouseButton::Left), gpui::Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, gpui::Modifiers::default());
    let dragged = cx.update(|_window, app| input.read(app).selected_text());
    assert!(
        dragged.is_some(),
        "precondition: the drag installed a selection, got {dragged:?}"
    );

    cx.update(|_window, app| crate::text_selection_owner::preserve(app));
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        dragged,
        "a drag-installed selection must own the window like any other"
    );
}

/// Same invariant for `replace_utf8_range_preserving_view` (conflict resolver).
#[gpui::test]
fn a_view_preserving_replacement_keeps_its_selection(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello world", cx);
            input.set_selected_range(6..11, false, window, cx);
        });
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.replace_utf8_range_preserving_view(0..5, "HELLO", cx);
        });
    });
    let after = cx.update(|_window, app| input.read(app).selected_text());
    assert_eq!(
        after,
        Some("world".to_string()),
        "precondition: the replacement preserved the selection"
    );

    cx.update(|_window, app| crate::text_selection_owner::preserve(app));
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_window, app| input.read(app).selected_text()),
        after,
        "a view-preserving replacement must leave the selection owned"
    );
}

/// Select-all takes the window's selection only when it highlights something.
/// This is why Ctrl+F is asymmetric by design: a populated search box paints a
/// highlight and must win, an empty one paints nothing and must not.
#[gpui::test]
fn select_all_takes_ownership_only_when_it_highlights_something(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    let elsewhere = cx.update(|window, app| {
        let mut owner = crate::text_selection_owner::SelectionOwnerToken::default();
        owner.adopt(window, app);
        owner
    });

    // Empty buffer: select-all highlights nothing, so ownership must not move.
    cx.update(|window, app| {
        input.update(app, |input, cx| input.select_all_text(window, cx));
    });
    cx.run_until_parked();
    assert!(
        cx.update(|_window, app| !elsewhere.is_stale(app)),
        "select-all on an empty buffer paints nothing and must not take the selection"
    );

    // With content, it is a real highlight and must take over.
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello", cx);
            input.select_all_text(window, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        cx.update(|_window, app| elsewhere.is_stale(app)),
        "select-all over real text is a highlight and must take the selection"
    );
}

/// Left at the start and Right at the end still reset the vertical goal
/// column and scroll the caret into view; the edge flag only rides along.
#[gpui::test]
fn arrow_keys_at_the_text_edges_keep_caret_bookkeeping(cx: &mut gpui::TestAppContext) {
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
            input.set_text("first\nsecond", cx);
            for (offset, press_left) in [(0, true), (input.text().len(), false)] {
                input.move_to(offset, cx);
                input.interaction.vertical_motion_x = Some(px(40.0));
                input.interaction.pending_cursor_autoscroll = false;
                if press_left {
                    input.left(&Left, window, cx);
                    assert!(input.take_arrow_left_at_start_pressed());
                } else {
                    input.right(&Right, window, cx);
                    assert!(input.take_arrow_right_at_end_pressed());
                }
                assert_eq!(input.cursor_offset(), offset);
                assert_eq!(input.interaction.vertical_motion_x, None);
                assert!(input.interaction.pending_cursor_autoscroll);
            }
        });
    });
}
