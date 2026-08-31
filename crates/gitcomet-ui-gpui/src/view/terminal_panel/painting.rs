use super::*;
use rustc_hash::FxHasher;
use std::hash::Hasher;

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
            None,
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
        None,
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
