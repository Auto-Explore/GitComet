use super::*;
use gitcomet_ui_kit::theme::AppTheme;
use gpui::{Bounds, Pixels, point, px, size};

/// Program output must never reach the system clipboard, nor pull it into
/// the PTY: OSC 52 stays disabled and the emulator's clipboard events map
/// to nothing.
#[test]
fn terminal_config_disables_osc52() {
    let config = terminal_config(TERMINAL_SCROLLBACK_ROWS);
    assert!(matches!(config.osc52, Osc52::Disabled));
}

#[test]
fn clipboard_events_from_the_emulator_are_inert() {
    use alacritty_terminal::term::ClipboardType;

    let store = TerminalBackendEvent::from(AlacEvent::ClipboardStore(
        ClipboardType::Clipboard,
        "exfiltrated".to_string(),
    ));
    assert!(matches!(store, TerminalBackendEvent::Wakeup));

    let load = TerminalBackendEvent::from(AlacEvent::ClipboardLoad(
        ClipboardType::Clipboard,
        std::sync::Arc::new(|text: &str| text.to_string()),
    ));
    assert!(matches!(load, TerminalBackendEvent::Wakeup));
}

/// Only what the program printed counts as output; a repaint the emulator
/// asks for on its own does not.
#[test]
fn output_is_only_what_the_program_printed() {
    assert!(matches!(
        TerminalBackendEvent::from(AlacEvent::Wakeup),
        TerminalBackendEvent::Output
    ));
    assert!(matches!(
        TerminalBackendEvent::from(AlacEvent::MouseCursorDirty),
        TerminalBackendEvent::Wakeup
    ));
}

#[cfg(unix)]
#[test]
fn alacritty_child_exit_preserves_the_exit_code() {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    for (status, code) in [(7 << 8, Some(7)), (15, None)] {
        let event = AlacEvent::ChildExit(ExitStatus::from_raw(status));
        assert!(
            matches!(TerminalBackendEvent::from(event), TerminalBackendEvent::ChildExit(actual) if actual == code)
        );
    }
}
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::cell::Cell as AlacCell;
use gpui::{FontStyle, FontWeight, WhiteSpace};

fn default_text_style() -> gpui::TextStyle {
    gpui::TextStyle {
        color: gpui::Hsla::default(),
        font_family: Default::default(),
        font_features: Default::default(),
        font_fallbacks: None,
        font_size: px(14.0).into(),
        font_style: FontStyle::Normal,
        font_weight: FontWeight::NORMAL,
        font_width: Default::default(),
        line_height: px(20.0).into(),
        background_color: Some(gpui::Hsla::default()),
        white_space: WhiteSpace::Normal,
        underline: None,
        strikethrough: None,
        line_clamp: None,
        text_align: gpui::TextAlign::Left,
        text_overflow: Default::default(),
        letter_spacing: None,
        text_transform: None,
    }
}

fn make_cell(ch: char, col: i32) -> IndexedCell {
    IndexedCell {
        point: AlacPoint::new(Line(0), Column(col as usize)),
        cell: AlacCell {
            c: ch,
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Background,
            ),
            flags: Flags::empty(),
            extra: None,
        },
    }
}

#[test]
fn build_row_accumulates_text_for_same_style_cells() {
    let base = default_text_style();
    let cells = vec![
        make_cell('h', 0),
        make_cell('e', 1),
        make_cell('l', 2),
        make_cell('l', 3),
        make_cell('o', 4),
    ];
    let (text, runs, _bg_rects) =
        build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
    assert_eq!(text, "hello", "text must contain all same-style characters");
    assert_eq!(
        runs.len(),
        1,
        "same-style cells must produce a single TextRun"
    );
    assert_eq!(
        runs[0].len,
        "hello".len(),
        "TextRun length must match text length"
    );
}

#[test]
fn build_row_handles_empty_cells() {
    let base = default_text_style();
    let cells: Vec<IndexedCell> = vec![];
    let (text, runs, _bg_rects) =
        build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
    assert_eq!(text, "");
    assert!(runs.is_empty());
}

#[test]
fn word_bounds_expands_over_word_chars() {
    let chars: Vec<char> = "  cargo build  ".chars().collect();
    // Click inside "cargo" (index 3) -> covers cols 2..=6.
    assert_eq!(terminal_word_bounds(&chars, 3), Some((2, 6)));
    // Click inside "build" (index 9) -> covers cols 8..=12.
    assert_eq!(terminal_word_bounds(&chars, 9), Some((8, 12)));
}

#[test]
fn word_bounds_includes_path_punctuation() {
    let chars: Vec<char> = "see ./src/main.rs now".chars().collect();
    // "./src/main.rs" spans cols 4..=16 (dots, slashes kept).
    assert_eq!(terminal_word_bounds(&chars, 8), Some((4, 16)));
}

#[test]
fn word_bounds_none_on_whitespace_or_oob() {
    let chars: Vec<char> = "a b".chars().collect();
    assert_eq!(terminal_word_bounds(&chars, 1), None); // space
    assert_eq!(terminal_word_bounds(&chars, 99), None); // out of bounds
}

#[test]
fn build_row_handles_wide_char_spacer() {
    let base = default_text_style();
    let cells = vec![
        IndexedCell {
            point: AlacPoint::new(Line(0), Column(0_usize)),
            cell: AlacCell {
                c: '\0',
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background,
                ),
                flags: Flags::WIDE_CHAR_SPACER,
                extra: None,
            },
        },
        make_cell('a', 1),
    ];
    let (text, _runs, _bg_rects) =
        build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
    assert_eq!(text, "a", "wide char spacer at col 0 must be skipped");
}

#[test]
fn build_row_keeps_reading_cells_after_a_wide_char() {
    let base = default_text_style();
    let mut wide = make_cell('\u{5360}', 1);
    wide.cell.flags = Flags::WIDE_CHAR;
    let mut spacer = make_cell(' ', 2);
    spacer.cell.flags = Flags::WIDE_CHAR_SPACER;
    let cells = vec![make_cell('A', 0), wide, spacer, make_cell('B', 3)];

    let (text, _runs, _bg_rects) =
        build_alacritty_row(&cells, 0, 4, &base, AppTheme::gitcomet_dark());

    // The row walk holds a single forward cursor over `cells`, so a column
    // it advances past without consuming the matching cell strands that
    // cursor and silently drops the rest of the line.
    assert_eq!(
        text, "A\u{5360}B",
        "cells after a wide char must still render"
    );
}

#[test]
fn build_row_does_not_reserve_per_column_for_runs_and_backgrounds() {
    // `terminal_panel` stores both vecs on the cached row, so anything
    // reserved here is held for the render cache's lifetime rather than
    // freed with the call. `runs` holds *merged* style spans and
    // `background_rects` only non-default backgrounds -- neither is per-column.
    let base = default_text_style();
    let cols = 200usize;
    let cells = vec![make_cell('a', 0), make_cell('b', 1)];

    let (_text, runs, backgrounds) =
        build_alacritty_row(&cells, 0, cols, &base, AppTheme::gitcomet_dark());

    assert!(
        backgrounds.is_empty(),
        "default backgrounds are not emitted"
    );
    assert_eq!(
        backgrounds.capacity(),
        0,
        "an empty background vec must not hold a per-column allocation"
    );
    assert!(
        runs.capacity() < cols,
        "reserved {} runs for {} merged span(s)",
        runs.capacity(),
        runs.len()
    );
}

#[test]
fn build_row_keeps_reading_cells_across_an_empty_column() {
    let base = default_text_style();
    let cells = vec![make_cell('A', 0), make_cell('B', 2)];

    let (text, _runs, _bg_rects) =
        build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());

    assert_eq!(
        text, "AB",
        "a column with no cell must not strand the cursor"
    );
}

#[test]
fn build_row_produces_background_rects() {
    let base = default_text_style();
    let cells = vec![IndexedCell {
        point: AlacPoint::new(Line(0), Column(0_usize)),
        cell: AlacCell {
            c: 'X',
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Red,
            ),
            flags: Flags::empty(),
            extra: None,
        },
    }];
    let (_text, _runs, bg_rects) =
        build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
    assert_eq!(
        bg_rects.len(),
        1,
        "non-default background must produce a rect"
    );
    assert_eq!(bg_rects[0].col, 0);
    assert_eq!(bg_rects[0].num_cells, 1);
    assert_eq!(bg_rects[0].row, 0);
}

#[test]
fn build_row_no_background_rects_for_default_bg() {
    let base = default_text_style();
    let cells = vec![make_cell('X', 0)];
    let (_text, _runs, bg_rects) =
        build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
    assert!(
        bg_rects.is_empty(),
        "cells with default background must not produce rects"
    );
}

#[test]
fn build_row_adjacent_same_bg_merged() {
    let base = default_text_style();
    let make_red_cell = |ch, col| IndexedCell {
        point: AlacPoint::new(Line(0), Column(col as usize)),
        cell: AlacCell {
            c: ch,
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Red,
            ),
            flags: Flags::empty(),
            extra: None,
        },
    };
    let cells = vec![make_red_cell('a', 0), make_red_cell('b', 1)];
    let (_text, _runs, bg_rects) =
        build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
    assert_eq!(bg_rects.len(), 1, "adjacent same-bg cells must merge");
    assert_eq!(bg_rects[0].num_cells, 2);
}

#[test]
fn build_row_different_bg_not_merged() {
    let base = default_text_style();
    let cells = vec![
        IndexedCell {
            point: AlacPoint::new(Line(0), Column(0_usize)),
            cell: AlacCell {
                c: 'a',
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Red,
                ),
                flags: Flags::empty(),
                extra: None,
            },
        },
        IndexedCell {
            point: AlacPoint::new(Line(0), Column(1_usize)),
            cell: AlacCell {
                c: 'b',
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Blue,
                ),
                flags: Flags::empty(),
                extra: None,
            },
        },
    ];
    let (_text, _runs, bg_rects) =
        build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
    assert_eq!(bg_rects.len(), 2, "different bg colors must not merge");
}

#[test]
fn color_to_rgba_resolves_named_ansi_colors() {
    let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());

    let red = color_to_rgba(
        alacritty_terminal::vte::ansi::Color::Named(alacritty_terminal::vte::ansi::NamedColor::Red),
        &palette,
    );
    assert_eq!(red, palette.red);

    let green = color_to_rgba(
        alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Green,
        ),
        &palette,
    );
    assert_eq!(green, palette.green);

    let foreground = color_to_rgba(
        alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Foreground,
        ),
        &palette,
    );
    assert_eq!(foreground, palette.foreground);
}

#[test]
fn color_to_rgba_resolves_spec_colors() {
    let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
    let spec = color_to_rgba(
        alacritty_terminal::vte::ansi::Color::Spec(alacritty_terminal::vte::ansi::Rgb {
            r: 100,
            g: 150,
            b: 200,
        }),
        &palette,
    );
    assert!((spec.red - 100.0 / 255.0).abs() < 0.01);
    assert!((spec.green - 150.0 / 255.0).abs() < 0.01);
    assert!((spec.blue - 200.0 / 255.0).abs() < 0.01);
}

#[test]
fn get_color_at_index_maps_correctly() {
    let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
    assert_eq!(get_color_at_index(0, &palette), palette.black);
    assert_eq!(get_color_at_index(1, &palette), palette.red);
    assert_eq!(get_color_at_index(7, &palette), palette.white);
    assert_eq!(get_color_at_index(8, &palette), palette.bright_black);
    assert_eq!(get_color_at_index(15, &palette), palette.bright_white);
    // 256-color cube: index 16 is first entry
    let cube_color = get_color_at_index(16, &palette);
    assert_eq!(cube_color.red, 0.0);
    assert_eq!(cube_color.green, 0.0);
    assert_eq!(cube_color.blue, 0.0);
    // Index 232 is first grayscale
    let gray = get_color_at_index(232, &palette);
    assert!((gray.red - 8.0 / 255.0).abs() < 0.01);
    assert_eq!(gray.red, gray.green);
    assert_eq!(gray.green, gray.blue);
}

#[test]
fn alacritty_cell_style_inverse_swaps_fg_bg() {
    let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
    let cell = AlacCell {
        c: 'X',
        fg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::White,
        ),
        bg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Red,
        ),
        flags: Flags::INVERSE,
        extra: None,
    };
    let style = alacritty_cell_style(&cell, &palette);
    assert_eq!(style.fg, palette.red, "inverse swaps fg to bg color");
    assert_eq!(
        style.bg,
        Some(palette.white),
        "inverse swaps bg to fg color"
    );
}

#[test]
fn alacritty_cell_style_inverse_default_colors_paints_background() {
    // Reverse-video over default colors (e.g. the PowerShell update banner):
    // the effective background is the default foreground, which must be
    // painted rather than dropped as "default background".
    let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
    let cell = AlacCell {
        c: 'X',
        fg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Foreground,
        ),
        bg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Background,
        ),
        flags: Flags::INVERSE,
        extra: None,
    };
    let style = alacritty_cell_style(&cell, &palette);
    assert_eq!(
        style.fg, palette.background,
        "inverse draws text in the default background color"
    );
    assert_eq!(
        style.bg,
        Some(palette.foreground),
        "inverse over default colors must still paint a (foreground-colored) background"
    );
}

#[test]
fn alacritty_cell_style_dim_reduces_alpha() {
    let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
    let cell = AlacCell {
        c: 'X',
        fg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::White,
        ),
        bg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Background,
        ),
        flags: Flags::DIM,
        extra: None,
    };
    let style = alacritty_cell_style(&cell, &palette);
    assert!(style.fg.alpha < 1.0, "dim must reduce foreground alpha");
}

#[test]
fn alacritty_cell_style_default_bg_is_none() {
    let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
    let cell = AlacCell {
        c: 'X',
        fg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Foreground,
        ),
        bg: alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Background,
        ),
        flags: Flags::empty(),
        extra: None,
    };
    let style = alacritty_cell_style(&cell, &palette);
    assert!(
        style.bg.is_none(),
        "default background must produce None bg"
    );
}

#[test]
fn terminal_text_run_no_background_color() {
    let base = default_text_style();
    let style = TerminalCellStyle {
        fg: gpui::rgb(0xff0000),
        bg: Some(gpui::rgb(0x00ff00)),
        bold: false,
        italic: false,
        underline: false,
    };
    let run = terminal_text_run(&base, &style, 5);
    assert!(
        run.background_color.is_none(),
        "TextRun must never set background_color"
    );
    let expected_color: gpui::Hsla = style.fg.into_color();
    assert_eq!(run.color, expected_color);
}

#[test]
fn terminal_palette_dark_vs_light_differs() {
    let dark = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
    let light = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_light());
    assert_ne!(
        dark.background, light.background,
        "dark/light palettes must differ"
    );
}

#[test]
fn rgb_for_index_computes_6x6x6_cube() {
    assert_eq!(rgb_for_index(0), (0, 0, 0));
    assert_eq!(rgb_for_index(1), (0, 0, 1));
    assert_eq!(rgb_for_index(5), (0, 0, 5));
    assert_eq!(rgb_for_index(6), (0, 1, 0));
    assert_eq!(rgb_for_index(35), (0, 5, 5));
    assert_eq!(rgb_for_index(36), (1, 0, 0));
    assert_eq!(rgb_for_index(215), (5, 5, 5));
}

#[test]
fn build_row_includes_spaces_for_grid_alignment() {
    let base = default_text_style();
    let cells = vec![
        make_cell('a', 0),
        make_cell(' ', 1),
        make_cell(' ', 2),
        make_cell('b', 3),
    ];
    let (text, _runs, _bg_rects) =
        build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
    assert_eq!(
        text, "a  b",
        "text must include spaces so shape_line positions glyphs at correct columns"
    );
}

#[test]
fn build_row_handles_zero_width_chars() {
    let base = default_text_style();
    let combining = '\u{0301}';
    let cell = IndexedCell {
        point: AlacPoint::new(Line(0), Column(0_usize)),
        cell: {
            let mut c = AlacCell {
                c: 'e',
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background,
                ),
                flags: Flags::empty(),
                extra: None,
            };
            c.push_zerowidth(combining);
            c
        },
    };
    let (text, runs, _bg_rects) =
        build_alacritty_row(&[cell], 0, 2, &base, AppTheme::gitcomet_dark());
    let expected = format!("e{}", combining);
    assert_eq!(
        text, expected,
        "text must include zero-width combining char"
    );
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len, expected.len());
}

// -----------------------------------------------------------------------
// Mouse encoding tests
// -----------------------------------------------------------------------

#[test]
fn grid_point_computes_correctly() {
    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
    let cell_width = px(8.0);
    let line_height = px(16.0);
    let display_offset = 0;
    let cols = 100u16;

    let (row, col) = terminal_grid_point(
        point(px(40.0), px(80.0)),
        bounds,
        cell_width,
        line_height,
        display_offset,
        cols,
    )
    .unwrap();
    assert_eq!(col, 5);
    assert_eq!(row, 5);
}

#[test]
fn grid_point_with_display_offset() {
    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
    let cell_width = px(8.0);
    let line_height = px(16.0);
    let display_offset = 5;
    let cols = 100u16;

    let (row, _col) = terminal_grid_point(
        point(px(0.0), px(0.0)),
        bounds,
        cell_width,
        line_height,
        display_offset,
        cols,
    )
    .unwrap();
    assert_eq!(row, -5, "viewport row 0 with offset 5 = grid row -5");
}

fn selection_bounds() -> Bounds<Pixels> {
    Bounds::new(point(px(100.0), px(200.0)), size(px(800.0), px(384.0)))
}

/// 800x384 at 8x16 = 100 columns, 24 rows.
fn selection_point(
    x: f32,
    y: f32,
    display_offset: usize,
    history_size: usize,
) -> TerminalGridPoint {
    terminal_selection_grid_point(
        point(px(x), px(y)),
        selection_bounds(),
        px(8.0),
        px(16.0),
        display_offset,
        history_size,
        100,
        24,
    )
    .expect("selection point resolves inside a non-empty grid")
}

#[test]
fn clamp_grid_row_spans_history_and_screen() {
    assert_eq!(terminal_clamp_grid_row(-500, 100, 24), -100);
    assert_eq!(terminal_clamp_grid_row(-100, 100, 24), -100);
    assert_eq!(terminal_clamp_grid_row(0, 100, 24), 0);
    assert_eq!(terminal_clamp_grid_row(23, 100, 24), 23);
    assert_eq!(terminal_clamp_grid_row(500, 100, 24), 23);
    // No history yet: negative rows are not addressable at all.
    assert_eq!(terminal_clamp_grid_row(-3, 0, 24), 0);
    // Degenerate grid: `top` wins so the range never inverts.
    assert_eq!(terminal_clamp_grid_row(5, 0, 0), 0);
}

#[test]
fn selection_grid_point_resolves_scrollback_rows_as_negatives() {
    // The top visible row while scrolled back 5 lines is grid row -5. The
    // old u16 selection point clamped this to 0, which put the highlight 5
    // rows below the pointer and copied the wrong text.
    let p = selection_point(100.0, 200.0, 5, 100);
    assert_eq!(p.row, -5);
    assert_eq!(p.col, 0);
    // Third visible row, tenth column.
    let p = selection_point(100.0 + 80.0, 200.0 + 32.0, 5, 100);
    assert_eq!(p.row, -3);
    assert_eq!(p.col, 10);
}

#[test]
fn selection_grid_point_clamps_positions_outside_the_viewport() {
    // Dragging above/left of the terminal must resolve to the edge cell.
    // `terminal_grid_point` returns None there, which would freeze the
    // selection instead of extending it upward.
    let p = selection_point(-500.0, -500.0, 0, 100);
    assert_eq!(p.row, 0, "above the viewport clamps to the top visible row");
    assert_eq!(p.col, 0, "left of the viewport clamps to column 0");
    // Below the viewport clamps to the last visible row.
    let p = selection_point(100.0, 5_000.0, 0, 100);
    assert_eq!(p.row, 23);
    // The exact right edge floors to `columns`, so it must be clamped back.
    let p = selection_point(900.0, 200.0, 0, 100);
    assert_eq!(p.col, 99);
}

#[test]
fn selection_grid_point_below_the_viewport_stops_at_the_last_visible_row() {
    // Scrolled back 5, the visible window is grid rows -5..=18. Dragging
    // below must land on 18, not on the row after it: sitting on the bottom
    // edge floors to screen row 24, one past the last of the 24 rows.
    let p = selection_point(100.0, 5_000.0, 5, 100);
    assert_eq!(p.row, 18);
    let p = selection_point(100.0, 200.0 + 384.0, 5, 100);
    assert_eq!(p.row, 18, "the exact bottom edge behaves the same");
}

#[test]
fn selection_grid_point_cannot_escape_a_short_history() {
    // Dragging far above with only 2 lines of scrollback stops at row -2.
    let p = selection_point(100.0, -5_000.0, 2, 2);
    assert_eq!(p.row, -2);
}

#[test]
fn selection_grid_point_rejects_an_empty_grid() {
    let resolved = terminal_selection_grid_point(
        point(px(0.0), px(0.0)),
        selection_bounds(),
        px(8.0),
        px(16.0),
        0,
        0,
        0,
        0,
    );
    assert!(resolved.is_none());
}

#[test]
fn autoscroll_lines_scroll_into_history_above_the_viewport() {
    let (top, bottom, lh) = (px(200.0), px(584.0), px(16.0));
    assert_eq!(terminal_autoscroll_lines(px(300.0), top, bottom, lh), 0);
    assert_eq!(terminal_autoscroll_lines(top, top, bottom, lh), 0);
    assert_eq!(terminal_autoscroll_lines(bottom, top, bottom, lh), 0);
    // Above the top scrolls up into history: `Scroll::Delta` is positive.
    assert_eq!(terminal_autoscroll_lines(px(196.0), top, bottom, lh), 1);
    assert_eq!(terminal_autoscroll_lines(px(168.0), top, bottom, lh), 2);
    // Below the bottom scrolls toward the live tail.
    assert_eq!(terminal_autoscroll_lines(px(588.0), top, bottom, lh), -1);
    assert_eq!(terminal_autoscroll_lines(px(616.0), top, bottom, lh), -2);
    // Ramp is monotonic and capped so a flick off-screen stays controllable.
    assert_eq!(terminal_autoscroll_lines(px(-5_000.0), top, bottom, lh), 6);
    assert_eq!(terminal_autoscroll_lines(px(5_000.0), top, bottom, lh), -6);
}

#[test]
fn selection_visible_rows_clamps_to_the_screen_window() {
    // Whole-buffer selection at the live tail: exactly the visible screen.
    assert_eq!(
        terminal_selection_visible_rows(-10_000, 23, 0, 24),
        Some(0..=23)
    );
    // Scrolled back 5: the visible window is grid rows -5..=18.
    assert_eq!(
        terminal_selection_visible_rows(-10_000, 23, 5, 24),
        Some(-5..=18)
    );
    // A selection entirely inside the window is returned untouched.
    assert_eq!(terminal_selection_visible_rows(3, 7, 0, 24), Some(3..=7));
    // Reversed endpoints normalise.
    assert_eq!(terminal_selection_visible_rows(7, 3, 0, 24), Some(3..=7));
    // Scrolled far past the selection: nothing to paint.
    assert_eq!(terminal_selection_visible_rows(-30, -25, 0, 24), None);
    assert_eq!(terminal_selection_visible_rows(0, 5, 100, 24), None);
    assert_eq!(terminal_selection_visible_rows(0, 5, 0, 0), None);
}

#[test]
fn grid_point_ordering_holds_across_scrollback() {
    // `copy_grid_range` and the paint loop both rely on `start <= end`
    // normalising correctly once rows can be negative.
    assert!(TerminalGridPoint::new(-5, 3) < TerminalGridPoint::new(0, 0));
    assert!(TerminalGridPoint::new(-5, 9) < TerminalGridPoint::new(-5, 10));
    assert!(TerminalGridPoint::new(-10, 0) < TerminalGridPoint::new(-5, 0));
    assert!(TerminalGridPoint::new(23, 0) > TerminalGridPoint::new(-1, 99));
}

// -----------------------------------------------------------------------
// Background rect merging tests
// -----------------------------------------------------------------------

fn make_bg_rect(row: i32, col: i32, cells: usize, color: gpui::Rgba) -> TerminalBackgroundRect {
    TerminalBackgroundRect {
        row,
        col,
        num_cells: cells,
        num_rows: 1,
        color,
    }
}

#[test]
fn merge_background_rects_vertical_adjacent_same_span() {
    let red = gpui::rgb(0xff0000);
    let rects = vec![
        make_bg_rect(0, 0, 5, red),
        make_bg_rect(1, 0, 5, red),
        make_bg_rect(2, 0, 5, red),
    ];
    let merged = merge_background_rects(&rects);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].row, 0);
    assert_eq!(merged[0].num_rows, 3);
    assert_eq!(merged[0].num_cells, 5);
}

#[test]
fn merge_background_rects_different_colors_not_merged() {
    let red = gpui::rgb(0xff0000);
    let blue = gpui::rgb(0x0000ff);
    let rects = vec![make_bg_rect(0, 0, 5, red), make_bg_rect(1, 0, 5, blue)];
    let merged = merge_background_rects(&rects);
    assert_eq!(merged.len(), 2);
}

#[test]
fn merge_background_rects_empty_input() {
    let rects: Vec<TerminalBackgroundRect> = vec![];
    let merged = merge_background_rects(&rects);
    assert!(merged.is_empty());
}

// -----------------------------------------------------------------------
// Alt / Meta key encoding tests
// -----------------------------------------------------------------------

#[test]
fn terminal_modes_intersects_partial() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
    assert!(mode.intersects(TerminalModes::MOUSE_MODE));
    assert!(mode.intersects(TerminalModes::MOUSE_REPORT_CLICK));
    assert!(!mode.intersects(TerminalModes::MOUSE_DRAG));
}

// -----------------------------------------------------------------------
// Build row with grid-relative rows (scrollback) tests
// -----------------------------------------------------------------------

#[test]
fn build_row_with_negative_grid_row() {
    let base = default_text_style();
    let cells = vec![IndexedCell {
        point: AlacPoint::new(Line(-3), Column(0_usize)),
        cell: AlacCell {
            c: 'S',
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Background,
            ),
            flags: Flags::empty(),
            extra: None,
        },
    }];
    let (text, runs, _bg) = build_alacritty_row(&cells, -3, 5, &base, AppTheme::gitcomet_dark());
    assert_eq!(
        text, "S",
        "build_alacritty_row must accept negative grid rows for scrollback"
    );
    assert_eq!(runs.len(), 1);
}

#[test]
fn build_row_empty_when_no_cells_at_row() {
    let base = default_text_style();
    let cells = vec![IndexedCell {
        point: AlacPoint::new(Line(0), Column(0_usize)),
        cell: AlacCell {
            c: 'X',
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Background,
            ),
            flags: Flags::empty(),
            extra: None,
        },
    }];
    let (text, runs, _bg) = build_alacritty_row(&cells, 5, 3, &base, AppTheme::gitcomet_dark());
    assert_eq!(text, "", "no cells at row 5");
    assert!(runs.is_empty());
}

#[test]
fn build_row_reads_only_the_requested_row_from_mixed_cells() {
    let cell = |row, col, ch| IndexedCell {
        point: AlacPoint::new(Line(row), Column(col)),
        cell: AlacCell {
            c: ch,
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Background,
            ),
            flags: Flags::empty(),
            extra: None,
        },
    };
    let cells = vec![
        cell(0, 0, 'X'),
        cell(1, 0, 'A'),
        cell(0, 1, 'Y'),
        cell(1, 1, 'B'),
    ];

    let (text, runs, backgrounds) = build_alacritty_row(
        &cells,
        1,
        2,
        &default_text_style(),
        AppTheme::gitcomet_dark(),
    );

    assert_eq!(text, "AB");
    assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), 2);
    assert!(backgrounds.is_empty());
}
