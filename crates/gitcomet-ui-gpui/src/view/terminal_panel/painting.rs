use super::*;
use rustc_hash::FxHasher;
use std::hash::Hasher;

/// Place backend-shaped glyphs on Alacritty's authoritative cell grid. Terminal
/// hit testing and selection use that grid directly, rather than text-layout
/// caret geometry. Shape once per cached row, including combining characters
/// and wide-cell spacers, instead of reshaping individual cells.
#[allow(clippy::too_many_arguments)]
pub(super) fn shape_terminal_grid_line(
    text: SharedString,
    runs: &[TextRun],
    cells: &[IndexedCell],
    row: i32,
    cols: usize,
    font_size: Pixels,
    cell_width: Pixels,
    window: &mut Window,
) -> gpui::ShapedLine {
    use alacritty_terminal::term::cell::Flags;

    let mut source = Vec::new();
    let mut offset = 0;
    for cell in cells
        .iter()
        .filter(|cell| cell.point.line.0 == row && cell.point.column.0 < cols)
    {
        if cell.cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let len = if matches!(cell.cell.c, ' ' | '\0') {
            1
        } else {
            cell.cell.c.len_utf8()
                + cell
                    .cell
                    .zerowidth()
                    .map_or(0, |chars| chars.iter().map(|ch| ch.len_utf8()).sum())
        };
        source.push((
            offset,
            offset + len,
            cell.point.column.0,
            cell.point.column.0
                + if cell.cell.flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                },
        ));
        offset += len;
    }
    debug_assert_eq!(offset, text.len(), "terminal text and cell source agree");

    let ascii = text.is_ascii();
    let mut shaped = window.text_system().shape_line(text, font_size, runs);
    let mut layout = (**shaped).clone();
    let native = &layout.platform_layout;
    let mut run_starts = Vec::with_capacity(runs.len() + 1);
    run_starts.push(0);
    for run in runs {
        run_starts.push(run_starts.last().copied().unwrap_or(0) + run.len);
    }
    // Plain output usually has one glyph per ASCII cell. Avoid a native
    // hit test and caret lookup for every glyph in that case. Split font
    // fragments and ligatures retain the general cluster-aware mapping.
    let mut seen_runs = vec![false; runs.len()];
    let ascii_grid = ascii
        && source.len() == offset
        && layout.paint_fragments.iter().all(|fragment| {
            let unique = !seen_runs[fragment.source_run];
            seen_runs[fragment.source_run] = true;
            unique && fragment.glyphs.len() == runs[fragment.source_run].len
        })
        && seen_runs.into_iter().all(|seen| seen);
    for fragment in &mut layout.paint_fragments {
        let glyphs = Arc::make_mut(&mut fragment.glyphs);
        for (glyph_index, glyph) in glyphs.iter_mut().enumerate() {
            if ascii_grid {
                let cell = &source[run_starts[fragment.source_run] + glyph_index];
                glyph.position.x = cell_width * cell.2 as f32;
                continue;
            }
            let index = native
                .byte_index_from_pixel_point(point(glyph.position.x + px(0.001), px(0.5)), px(1.0))
                .unwrap_or_else(|index| index);
            let count = source.partition_point(|cell| cell.0 <= index);
            let Some(cell) = count.checked_sub(1).map(|index| &source[index]) else {
                continue;
            };
            let origin = native
                .caret_bounds(
                    gpui::CaretPosition::attached_to_next_cluster(cell.0),
                    px(1.0),
                )
                .map_or(glyph.position.x, |bounds| bounds.origin.x);
            glyph.position.x = cell_width * cell.2 as f32 + glyph.position.x - origin;
        }
        let start = run_starts[fragment.source_run];
        let end = run_starts[fragment.source_run + 1];
        let first = source.partition_point(|cell| cell.1 <= start);
        let last = source.partition_point(|cell| cell.0 < end);
        let columns =
            source[first..last]
                .iter()
                .fold(None, |range: Option<(usize, usize)>, cell| {
                    Some(range.map_or((cell.2, cell.3), |(start, end)| {
                        (start.min(cell.2), end.max(cell.3))
                    }))
                });
        if let Some((start, end)) = columns {
            fragment.x_range = cell_width * start as f32..cell_width * end as f32;
        }
    }
    layout.width = cell_width * cols as f32;
    if let Some(line) = layout.visual_lines.first_mut() {
        line.advance_width = layout.width;
    }
    *shaped = Arc::new(layout);
    shaped
}

#[cfg(test)]
mod grid_layout_tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
    use alacritty_terminal::term::cell::{Cell, Flags};

    #[gpui::test]
    fn ascii_with_style_changes_keeps_each_glyph_on_its_cell(cx: &mut gpui::TestAppContext) {
        gitcomet_ui_kit::test_support::use_real_text_backend(cx);
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            for family in ["Lilex", "IBM Plex Sans"] {
                for text in [
                    "GitComet-terminal-output",
                    "AV To 1234 /src/main.rs",
                    "          ",
                ] {
                    let cells: Vec<_> = text
                        .chars()
                        .enumerate()
                        .map(|(col, c)| {
                            let mut cell = Cell {
                                c,
                                ..Cell::default()
                            };
                            if col % 3 == 0 {
                                cell.flags.insert(Flags::BOLD);
                            }
                            IndexedCell {
                                point: AlacPoint::new(Line(0), Column(col)),
                                cell,
                            }
                        })
                        .collect();
                    let style = gpui::TextStyle {
                        font_family: family.into(),
                        ..Default::default()
                    };
                    let (text, runs, _) = build_alacritty_row(
                        &cells,
                        0,
                        cells.len(),
                        &style,
                        AppTheme::gitcomet_dark(),
                    );
                    let shaped = shape_terminal_grid_line(
                        text,
                        &runs,
                        &cells,
                        0,
                        cells.len(),
                        px(12.),
                        px(16.),
                        window,
                    );
                    let mut positions: Vec<_> = shaped
                        .paint_fragments
                        .iter()
                        .flat_map(|f| f.glyphs.iter())
                        .map(|g| g.position.x)
                        .collect();
                    positions.sort_by(|a, b| f32::from(*a).total_cmp(&f32::from(*b)));
                    assert_eq!(
                        positions,
                        (0..cells.len())
                            .map(|i| px(i as f32 * 16.))
                            .collect::<Vec<_>>(),
                        "{family}"
                    );
                }
            }
        });
    }

    #[gpui::test]
    fn wide_and_combining_cells_keep_the_authoritative_grid(cx: &mut gpui::TestAppContext) {
        gitcomet_ui_kit::test_support::use_real_text_backend(cx);
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let mut wide = Cell {
                c: '日',
                ..Cell::default()
            };
            wide.flags.insert(Flags::WIDE_CHAR);
            let mut spacer = Cell::default();
            spacer.flags.insert(Flags::WIDE_CHAR_SPACER);
            let cells = vec![
                IndexedCell {
                    point: AlacPoint::new(Line(0), Column(0)),
                    cell: Cell {
                        c: 'a',
                        ..Cell::default()
                    },
                },
                IndexedCell {
                    point: AlacPoint::new(Line(0), Column(1)),
                    cell: wide,
                },
                IndexedCell {
                    point: AlacPoint::new(Line(0), Column(2)),
                    cell: spacer,
                },
                IndexedCell {
                    point: AlacPoint::new(Line(0), Column(3)),
                    cell: Cell {
                        c: 'b',
                        ..Cell::default()
                    },
                },
                IndexedCell {
                    point: AlacPoint::new(Line(0), Column(4)),
                    cell: Cell {
                        c: 'é',
                        ..Cell::default()
                    },
                },
            ];
            let style = gpui::TextStyle {
                font_family: "Lilex".into(),
                ..gpui::TextStyle::default()
            };
            let (text, runs, _) =
                build_alacritty_row(&cells, 0, 5, &style, AppTheme::gitcomet_dark());
            let line =
                shape_terminal_grid_line(text, &runs, &cells, 0, 5, px(12.0), px(16.0), window);
            let positions: Vec<_> = line
                .paint_fragments
                .iter()
                .flat_map(|fragment| fragment.glyphs.iter())
                .map(|glyph| glyph.position.x)
                .collect();
            assert_eq!(positions, [px(0.0), px(16.0), px(48.0), px(64.0)]);
            assert_eq!(line.width, px(80.0));

            let mut accent = Cell {
                c: 'e',
                ..Cell::default()
            };
            accent.push_zerowidth('\u{301}');
            let cells = vec![
                IndexedCell {
                    point: AlacPoint::new(Line(0), Column(0)),
                    cell: accent,
                },
                IndexedCell {
                    point: AlacPoint::new(Line(0), Column(1)),
                    cell: Cell {
                        c: 'b',
                        ..Cell::default()
                    },
                },
            ];
            let (text, runs, _) =
                build_alacritty_row(&cells, 0, 2, &style, AppTheme::gitcomet_dark());
            let line =
                shape_terminal_grid_line(text, &runs, &cells, 0, 2, px(12.0), px(16.0), window);
            let positions: Vec<_> = line
                .paint_fragments
                .iter()
                .flat_map(|fragment| fragment.glyphs.iter())
                .map(|glyph| glyph.position.x)
                .collect();
            assert_eq!(
                positions.last(),
                Some(&px(16.0)),
                "combining marks do not consume a cell"
            );
            assert_eq!(line.width, px(32.0));
        });
    }
}

#[derive(Default)]
pub(super) struct TerminalCanvasPaintState {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) terminal_bg: gpui::Rgba,
    pub(super) selection_rects: Vec<Bounds<Pixels>>,
    pub(super) background_rects: Vec<(Point<Pixels>, gpui::Size<Pixels>, gpui::Rgba)>,
    pub(super) lines: Vec<(ShapedLine, Point<Pixels>, Pixels)>,
    pub(super) cursor: Option<TerminalPaintCursor>,
    pub(super) ime_bounds: Option<Bounds<Pixels>>,
    pub(super) ime_marked_text: Option<String>,
    pub(super) ime_base_style: Option<gpui::TextStyle>,
}

#[derive(Clone)]
pub(super) struct TerminalPaintCursor {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) shape: TerminalCursorShape,
}

/// Grid dimensions read live from the backing `Term`, rather than from the
/// `last_content` snapshot that is only refreshed during canvas prepaint. Any
/// `scroll_display` (autoscroll tick, wheel, scrollbar drag, scrollback keys)
/// leaves that snapshot's `display_offset` stale until the next paint, so
/// selection must resolve against these values instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TerminalGridGeometry {
    pub(super) display_offset: usize,
    pub(super) history_size: usize,
    pub(super) columns: usize,
    pub(super) screen_lines: usize,
}

pub(super) fn paint_terminal_canvas_state(
    paint_state: TerminalCanvasPaintState,
    theme: AppTheme,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(paint_state.bounds, paint_state.terminal_bg));
    for (origin, rect_size, color) in paint_state.background_rects {
        window.paint_quad(fill(Bounds::new(origin, rect_size), color));
    }
    for rect in paint_state.selection_rects {
        window.paint_quad(fill(
            rect,
            with_alpha(theme.colors.accent.foreground, TERMINAL_SELECTION_ALPHA),
        ));
    }
    for (line, origin, line_height) in paint_state.lines {
        let _ = line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx);
    }
    if let Some(cursor) = paint_state.cursor {
        paint_terminal_cursor(cursor, theme, window);
    }

    // IME preedit (marked) text
    if let Some(ref marked_text) = paint_state.ime_marked_text
        && let Some(ime_bounds) = paint_state.ime_bounds
        && let Some(ref base_style) = paint_state.ime_base_style
    {
        let mut ime_style = base_style.clone();
        ime_style.underline = Some(gpui::UnderlineStyle {
            color: Some(ime_style.color),
            thickness: px(1.0),
            wavy: false,
        });
        let shaped = window.text_system().shape_line(
            marked_text.clone().into(),
            ime_style.font_size.to_pixels(window.rem_size()),
            &[TextRun {
                len: marked_text.len(),
                font: ime_style.font(),
                color: ime_style.color,
                underline: ime_style.underline,
                ..Default::default()
            }],
        );
        let ime_bg = Bounds::new(
            ime_bounds.origin,
            size(shaped.width, ime_bounds.size.height),
        );
        window.paint_quad(fill(ime_bg, paint_state.terminal_bg));
        let _ = shaped.paint(
            ime_bounds.origin,
            ime_bounds.size.height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

pub(super) fn terminal_caret_bounds(cell_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = (cell_bounds.size.width * TERMINAL_CARET_WIDTH_RATIO)
        .max(px(TERMINAL_CARET_MIN_WIDTH_PX))
        .min(px(TERMINAL_CARET_MAX_WIDTH_PX))
        .min(cell_bounds.size.width.max(px(1.0)));
    let inset_y = (cell_bounds.size.height * 0.08)
        .max(px(TERMINAL_CARET_VERTICAL_INSET_PX))
        .min((cell_bounds.size.height / 2.0).max(px(0.0)));
    let height = (cell_bounds.size.height - inset_y * 2.0).max(px(1.0));
    Bounds::new(
        point(cell_bounds.left(), cell_bounds.top() + inset_y),
        size(width, height),
    )
}

pub(super) fn paint_terminal_cursor(
    cursor: TerminalPaintCursor,
    theme: AppTheme,
    window: &mut Window,
) {
    let cursor_color = terminal_default_foreground(theme);
    match cursor.shape {
        TerminalCursorShape::Beam => {
            let caret = terminal_caret_bounds(cursor.bounds);
            window.paint_quad(fill(caret, cursor_color).corner_radii(px(TERMINAL_CARET_RADIUS_PX)));
        }
        TerminalCursorShape::Underline => {
            let height = (cursor.bounds.size.height * 0.12).max(px(1.0));
            let underline = Bounds::new(
                point(cursor.bounds.left(), cursor.bounds.bottom() - height),
                size(cursor.bounds.size.width.max(px(1.0)), height),
            );
            window.paint_quad(fill(underline, cursor_color));
        }
        TerminalCursorShape::Block => {
            window.paint_quad(fill(cursor.bounds, cursor_color));
        }
        TerminalCursorShape::Hollow => {
            let thickness = px(1.0)
                .min(cursor.bounds.size.width / 2.0)
                .min(cursor.bounds.size.height / 2.0)
                .max(px(1.0));
            let top = Bounds::new(
                cursor.bounds.origin,
                size(cursor.bounds.size.width, thickness),
            );
            let bottom = Bounds::new(
                point(cursor.bounds.left(), cursor.bounds.bottom() - thickness),
                size(cursor.bounds.size.width, thickness),
            );
            let left = Bounds::new(
                cursor.bounds.origin,
                size(thickness, cursor.bounds.size.height),
            );
            let right = Bounds::new(
                point(cursor.bounds.right() - thickness, cursor.bounds.top()),
                size(thickness, cursor.bounds.size.height),
            );
            for edge in [top, bottom, left, right] {
                window.paint_quad(fill(edge, cursor_color));
            }
        }
        TerminalCursorShape::Hidden => {}
    }
}

pub(super) fn terminal_cursor_width(
    cursor_char: char,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    cell_width: Pixels,
    window: &Window,
) -> Pixels {
    if cursor_char.is_whitespace() {
        return cell_width;
    }
    let cursor_text = cursor_char.to_string();
    let shaped = window.text_system().shape_line(
        cursor_text.clone().into(),
        font_size,
        &[TextRun {
            len: cursor_text.len(),
            font: base_style.font(),
            color: base_style.color,
            ..Default::default()
        }],
    );
    shaped.width.max(cell_width).ceil()
}

pub(super) fn terminal_snap_to_device_pixels(window: &Window, value: Pixels) -> Pixels {
    let scale_factor = window.scale_factor().max(1.0);
    Pixels::from((f32::from(value) * scale_factor).floor() / scale_factor)
}

pub(super) fn terminal_row_fingerprint(cells: &[IndexedCell], row: i32, cols: usize) -> u64 {
    let mut hasher = FxHasher::default();
    row.hash(&mut hasher);
    cols.hash(&mut hasher);

    for cell in cells.iter().filter(|cell| cell.point.line.0 == row) {
        cell.point.column.0.hash(&mut hasher);
        cell.cell.c.hash(&mut hasher);
        cell.cell.flags.hash(&mut hasher);
        hash_terminal_color(cell.cell.fg, &mut hasher);
        hash_terminal_color(cell.cell.bg, &mut hasher);
        if let Some(zw_chars) = cell.cell.zerowidth() {
            zw_chars.len().hash(&mut hasher);
            for ch in zw_chars {
                ch.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

pub(super) fn hash_terminal_color<H: Hasher>(
    color: alacritty_terminal::vte::ansi::Color,
    hasher: &mut H,
) {
    use alacritty_terminal::vte::ansi::Color;

    match color {
        Color::Named(name) => {
            0u8.hash(hasher);
            std::mem::discriminant(&name).hash(hasher);
        }
        Color::Spec(rgb) => {
            1u8.hash(hasher);
            rgb.r.hash(hasher);
            rgb.g.hash(hasher);
            rgb.b.hash(hasher);
        }
        Color::Indexed(index) => {
            2u8.hash(hasher);
            index.hash(hasher);
        }
    }
}
