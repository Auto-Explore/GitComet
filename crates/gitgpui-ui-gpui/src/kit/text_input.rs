use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IsZero, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Rgba,
    ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window, WrappedLine,
    actions, div, fill, hsla, point, px, relative, size,
};
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation as _;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Enter,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Home,
        SelectHome,
        End,
        SelectEnd,
        PageUp,
        SelectPageUp,
        PageDown,
        SelectPageDown,
        Paste,
        Cut,
        Copy,
        ShowCharacterPalette,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq)]
struct TextInputStyle {
    background: Rgba,
    border: Rgba,
    hover_border: Rgba,
    focus_border: Rgba,
    radius: f32,
    text: gpui::Hsla,
    placeholder: gpui::Hsla,
    cursor: Rgba,
    selection: Rgba,
}

impl TextInputStyle {
    fn from_theme(theme: AppTheme) -> Self {
        fn mix(mut a: Rgba, b: Rgba, t: f32) -> Rgba {
            let t = t.clamp(0.0, 1.0);
            a.r = a.r + (b.r - a.r) * t;
            a.g = a.g + (b.g - a.g) * t;
            a.b = a.b + (b.b - a.b) * t;
            a.a = a.a + (b.a - a.a) * t;
            a
        }

        // Ensure inputs look like inputs even in themes where `surface_bg` and `surface_bg_elevated`
        // are equal (Ayu/One).
        let background = if theme.is_dark {
            mix(
                theme.colors.surface_bg_elevated,
                gpui::rgba(0xFFFFFFFF),
                0.03,
            )
        } else {
            mix(
                theme.colors.surface_bg_elevated,
                gpui::rgba(0x000000FF),
                0.03,
            )
        };

        let base_border = theme.colors.border;
        let hover_border = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.55 } else { 0.40 },
        );
        let focus_border = with_alpha(theme.colors.accent, if theme.is_dark { 0.98 } else { 0.92 });
        let placeholder = if theme.is_dark {
            hsla(0., 0., 1., 0.35)
        } else {
            hsla(0., 0., 0., 0.2)
        };
        Self {
            background,
            border: base_border,
            hover_border,
            focus_border,
            radius: theme.radii.row,
            text: theme.colors.text.into(),
            placeholder,
            cursor: with_alpha(theme.colors.text, if theme.is_dark { 0.78 } else { 0.62 }),
            selection: with_alpha(theme.colors.accent, if theme.is_dark { 0.28 } else { 0.18 }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TextInputOptions {
    pub placeholder: SharedString,
    pub multiline: bool,
    pub read_only: bool,
    pub chromeless: bool,
    pub soft_wrap: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct WrapCache {
    width: Pixels,
    rows: usize,
}

#[derive(Clone, Debug)]
enum TextInputLayout {
    Plain(Vec<ShapedLine>),
    Wrapped {
        lines: Vec<WrappedLine>,
        y_offsets: Vec<Pixels>,
    },
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    multiline: bool,
    read_only: bool,
    chromeless: bool,
    soft_wrap: bool,
    line_ending: &'static str,
    style: TextInputStyle,
    highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
    line_height_override: Option<Pixels>,

    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,

    scroll_x: Pixels,
    last_layout: Option<TextInputLayout>,
    last_line_starts: Option<Vec<usize>>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    wrap_cache: Option<WrapCache>,
    is_selecting: bool,
    suppress_right_click: bool,
    vertical_motion_x: Option<Pixels>,

    has_focus: bool,
    cursor_blink_visible: bool,
    cursor_blink_task: Option<gpui::Task<()>>,
}

impl TextInput {
    pub fn new(options: TextInputOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let _ = window;
        Self {
            focus_handle,
            content: "".into(),
            placeholder: options.placeholder,
            multiline: options.multiline,
            read_only: options.read_only,
            chromeless: options.chromeless,
            soft_wrap: options.soft_wrap,
            line_ending: if cfg!(windows) { "\r\n" } else { "\n" },
            style: TextInputStyle::from_theme(AppTheme::zed_ayu_dark()),
            highlights: Arc::new(Vec::new()),
            line_height_override: None,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            scroll_x: px(0.0),
            last_layout: None,
            last_line_starts: None,
            last_bounds: None,
            last_line_height: px(0.0),
            wrap_cache: None,
            is_selecting: false,
            suppress_right_click: false,
            vertical_motion_x: None,
            has_focus: false,
            cursor_blink_visible: true,
            cursor_blink_task: None,
        }
    }

    pub fn new_inert(options: TextInputOptions, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        Self {
            focus_handle,
            content: "".into(),
            placeholder: options.placeholder,
            multiline: options.multiline,
            read_only: options.read_only,
            chromeless: options.chromeless,
            soft_wrap: options.soft_wrap,
            line_ending: if cfg!(windows) { "\r\n" } else { "\n" },
            style: TextInputStyle::from_theme(AppTheme::zed_ayu_dark()),
            highlights: Arc::new(Vec::new()),
            line_height_override: None,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            scroll_x: px(0.0),
            last_layout: None,
            last_line_starts: None,
            last_bounds: None,
            last_line_height: px(0.0),
            wrap_cache: None,
            is_selecting: false,
            suppress_right_click: false,
            vertical_motion_x: None,
            has_focus: false,
            cursor_blink_visible: true,
            cursor_blink_task: None,
        }
    }

    pub fn text(&self) -> &str {
        self.content.as_ref()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        let style = TextInputStyle::from_theme(theme);
        if self.style == style {
            return;
        }
        self.style = style;
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if self.content == text {
            return;
        }
        self.content = text;
        self.selected_range = self.content.len()..self.content.len();
        self.cursor_blink_visible = true;
        self.scroll_x = px(0.0);
        cx.notify();
    }

    pub fn set_highlights(
        &mut self,
        mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
        cx: &mut Context<Self>,
    ) {
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        self.highlights = Arc::new(highlights);
        cx.notify();
    }

    pub fn set_line_height(&mut self, line_height: Option<Pixels>, cx: &mut Context<Self>) {
        if self.line_height_override == line_height {
            return;
        }
        self.line_height_override = line_height;
        cx.notify();
    }

    fn effective_line_height(&self, window: &Window) -> Pixels {
        self.line_height_override
            .unwrap_or_else(|| window.line_height())
    }

    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        if self.read_only == read_only {
            return;
        }
        self.read_only = read_only;
        cx.notify();
    }

    pub fn set_suppress_right_click(&mut self, suppress: bool) {
        self.suppress_right_click = suppress;
    }

    pub fn selected_text(&self) -> Option<String> {
        if self.selected_range.is_empty() {
            None
        } else {
            Some(self.content[self.selected_range.clone()].to_string())
        }
    }

    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    pub fn set_soft_wrap(&mut self, soft_wrap: bool, cx: &mut Context<Self>) {
        if self.soft_wrap == soft_wrap {
            return;
        }
        self.soft_wrap = soft_wrap;
        self.wrap_cache = None;
        cx.notify();
    }

    pub fn set_line_ending(&mut self, line_ending: &'static str) {
        self.line_ending = line_ending;
    }

    /// Detect line ending from file content. Returns `\r\n` if CRLF is found,
    /// otherwise falls back to the OS default (`\n` on Unix, `\r\n` on Windows).
    pub fn detect_line_ending(content: &str) -> &'static str {
        if content.contains("\r\n") || cfg!(windows) {
            "\r\n"
        } else {
            "\n"
        }
    }

    fn sanitize_insert_text(&self, text: &str) -> Option<String> {
        if self.multiline {
            return Some(text.to_string());
        }

        if text == "\n" || text == "\r" || text == "\r\n" {
            return None;
        }

        Some(
            text.replace("\r\n", "\n")
                .replace('\r', "\n")
                .replace('\n', " "),
        )
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_word_start(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_word_end(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word_start(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word_end(self.cursor_offset()), cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn row_start(&self, offset: usize) -> usize {
        self.row_boundaries(offset).0
    }

    fn row_end(&self, offset: usize) -> usize {
        self.row_boundaries(offset).1
    }

    fn logical_row_boundaries(&self, offset: usize) -> (usize, usize) {
        let s = self.content.as_ref();
        let offset = offset.min(s.len());
        let start = s[..offset].rfind('\n').map(|ix| ix + 1).unwrap_or(0);
        let rel_end = s[offset..].find('\n').unwrap_or(s.len() - offset);
        let end = offset + rel_end;
        (start, end)
    }

    fn row_boundaries(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.content.len());
        if self.content.is_empty() {
            return (0, 0);
        }
        if !(self.multiline && self.soft_wrap) {
            return self.logical_row_boundaries(offset);
        }

        let Some(TextInputLayout::Wrapped { lines, .. }) = self.last_layout.as_ref() else {
            return self.logical_row_boundaries(offset);
        };
        let Some(starts) = self.last_line_starts.as_ref() else {
            return self.logical_row_boundaries(offset);
        };
        let Some(line) = lines
            .get(starts.partition_point(|&s| s <= offset).saturating_sub(1))
            .or_else(|| lines.first())
        else {
            return self.logical_row_boundaries(offset);
        };

        let mut ix = starts.partition_point(|&s| s <= offset);
        if ix == 0 {
            ix = 1;
        }
        let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
        let line_start = starts.get(line_ix).copied().unwrap_or(0);
        let line = lines.get(line_ix).unwrap_or(line);
        let local = offset.saturating_sub(line_start).min(line.len());

        let mut row_end_indices: Vec<usize> = Vec::with_capacity(line.wrap_boundaries().len() + 1);
        for boundary in line.wrap_boundaries() {
            let Some(run) = line.unwrapped_layout.runs.get(boundary.run_ix) else {
                continue;
            };
            let Some(glyph) = run.glyphs.get(boundary.glyph_ix) else {
                continue;
            };
            row_end_indices.push(glyph.index);
        }
        row_end_indices.sort_unstable();
        row_end_indices.dedup();
        row_end_indices.push(line.len());

        let row_ix = row_end_indices
            .iter()
            .position(|&end| local <= end)
            .unwrap_or_else(|| row_end_indices.len().saturating_sub(1));
        let row_start_local = if row_ix == 0 {
            0
        } else {
            row_end_indices[row_ix - 1]
        };
        let row_end_local = row_end_indices[row_ix];
        (
            (line_start + row_start_local).min(self.content.len()),
            (line_start + row_end_local).min(self.content.len()),
        )
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_start(self.cursor_offset()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_start(self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_end(self.cursor_offset()), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_end(self.cursor_offset()), cx);
    }

    fn caret_point_for_hit_testing(&self, cursor: usize) -> Option<Point<Pixels>> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let starts = self.last_line_starts.as_ref()?;
        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts, lines, cursor);
                let line = lines.get(line_ix)?;
                let x = line.x_for_index(local_ix) - self.scroll_x;
                let y = line_height * line_ix as f32 + line_height / 2.0;
                Some(point(bounds.left() + x, bounds.top() + y))
            }
            TextInputLayout::Wrapped { lines, y_offsets } => {
                let mut ix = starts.partition_point(|&s| s <= cursor);
                if ix == 0 {
                    ix = 1;
                }
                let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
                let line = lines.get(line_ix)?;
                let start = starts.get(line_ix).copied().unwrap_or(0);
                let local = cursor.saturating_sub(start).min(line.len());
                let pos = line
                    .position_for_index(local, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                let y = y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO)
                    + pos.y
                    + line_height / 2.0;
                Some(point(bounds.left() + pos.x, bounds.top() + y))
            }
        }
    }

    fn vertical_move_target(
        &self,
        cursor: usize,
        direction: f32,
        preferred_x: Option<Pixels>,
    ) -> Option<(usize, Pixels)> {
        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };
        let caret_point = self.caret_point_for_hit_testing(cursor)?;
        let preferred_x = preferred_x.unwrap_or(caret_point.x);
        let target = point(preferred_x, caret_point.y + line_height * direction);
        Some((self.index_for_position(target), preferred_x))
    }

    fn page_move_target(
        &self,
        cursor: usize,
        direction: f32,
        preferred_x: Option<Pixels>,
    ) -> Option<(usize, Pixels)> {
        let bounds = self.last_bounds?;
        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };
        let page_height = bounds.size.height.max(line_height);
        let caret_point = self.caret_point_for_hit_testing(cursor)?;
        let preferred_x = preferred_x.unwrap_or(caret_point.x);
        let target = point(preferred_x, caret_point.y + page_height * direction);
        Some((self.index_for_position(target), preferred_x))
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || !self.multiline {
            return;
        }
        self.replace_text_in_range(None, self.line_ending, window, cx);
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            if !self.read_only {
                self.replace_text_in_range(None, "", window, cx)
            }
        }
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        self.index_for_position(position)
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    fn skip_left_while(
        s: &str,
        mut offset: usize,
        mut predicate: impl FnMut(char) -> bool,
    ) -> usize {
        offset = offset.min(s.len());
        while offset > 0 {
            let Some((idx, ch)) = s[..offset].char_indices().next_back() else {
                return 0;
            };
            if !predicate(ch) {
                break;
            }
            offset = idx;
        }
        offset
    }

    fn skip_right_while(
        s: &str,
        mut offset: usize,
        mut predicate: impl FnMut(char) -> bool,
    ) -> usize {
        offset = offset.min(s.len());
        while offset < s.len() {
            let Some(ch) = s[offset..].chars().next() else {
                break;
            };
            if !predicate(ch) {
                break;
            }
            offset += ch.len_utf8();
        }
        offset
    }

    fn previous_word_start(&self, offset: usize) -> usize {
        let s = self.content.as_ref();
        let mut offset = offset.min(s.len());

        // Skip any whitespace to the left of the cursor.
        offset = Self::skip_left_while(s, offset, |ch| ch.is_whitespace());

        // Skip punctuation/symbols (e.g. '.' '/' '-') so word navigation doesn't get stuck on them.
        offset = Self::skip_left_while(s, offset, |ch| {
            !ch.is_whitespace() && !Self::is_word_char(ch)
        });

        // Skip any whitespace again, then skip the word itself.
        offset = Self::skip_left_while(s, offset, |ch| ch.is_whitespace());
        Self::skip_left_while(s, offset, Self::is_word_char)
    }

    fn next_word_end(&self, offset: usize) -> usize {
        let s = self.content.as_ref();
        let offset = offset.min(s.len());
        if offset >= s.len() {
            return s.len();
        }

        let Some(ch) = s[offset..].chars().next() else {
            return s.len();
        };

        if ch.is_whitespace() {
            return Self::skip_right_while(s, offset, |ch| ch.is_whitespace());
        }
        if Self::is_word_char(ch) {
            return Self::skip_right_while(s, offset, Self::is_word_char);
        }

        Self::skip_right_while(s, offset, |ch| {
            !ch.is_whitespace() && !Self::is_word_char(ch)
        })
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus_handle);
        self.cursor_blink_visible = true;
        if self.read_only && event.button == MouseButton::Left && event.click_count >= 2 {
            self.move_to(0, cx);
            self.select_to(self.content.len(), cx);
            self.is_selecting = false;
            return;
        }

        self.is_selecting = true;
        self.vertical_motion_x = None;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.last_bounds.as_ref(),
            self.last_layout.as_ref(),
            self.last_line_starts.as_ref(),
        ) else {
            return 0;
        };

        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }

        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let ratio = f32::from(position.y - bounds.top()) / f32::from(line_height);
                let mut line_ix = ratio.floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.scroll_x;
                let local_ix = lines[line_ix].closest_index_for_x(local_x);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local_ix;
                doc_ix.min(self.content.len())
            }
            TextInputLayout::Wrapped { lines, y_offsets } => {
                let local_y = position.y - bounds.top();
                let mut line_ix = 0usize;
                for (ix, line) in lines.iter().enumerate() {
                    let y0 = y_offsets.get(ix).copied().unwrap_or(Pixels::ZERO);
                    let rows = line.wrap_boundaries().len().saturating_add(1);
                    let y1 = y0 + line_height * rows as f32;
                    if local_y >= y0 && local_y < y1 {
                        line_ix = ix;
                        break;
                    }
                    if local_y >= y1 {
                        line_ix = ix;
                    }
                }
                let line_ix = line_ix.min(lines.len().saturating_sub(1));
                let local_x = position.x - bounds.left();
                let local_y_in_line =
                    local_y - y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO);
                let line = &lines[line_ix];
                let local = line
                    .closest_index_for_position(point(local_x, local_y_in_line), line_height)
                    .unwrap_or_else(|ix| ix);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local;
                doc_ix.min(self.content.len())
            }
        }
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.last_bounds.as_ref(),
            self.last_layout.as_ref(),
            self.last_line_starts.as_ref(),
        ) else {
            return 0;
        };

        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let ratio = f32::from(position.y - bounds.top()) / f32::from(line_height);
                let mut line_ix = ratio.floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.scroll_x;
                let local_ix = lines[line_ix].closest_index_for_x(local_x);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local_ix;
                doc_ix.min(self.content.len())
            }
            TextInputLayout::Wrapped { lines, y_offsets } => {
                let local_y = position.y - bounds.top();
                let mut line_ix = 0usize;
                for (ix, line) in lines.iter().enumerate() {
                    let y0 = y_offsets.get(ix).copied().unwrap_or(Pixels::ZERO);
                    let rows = line.wrap_boundaries().len().saturating_add(1);
                    let y1 = y0 + line_height * rows as f32;
                    if local_y >= y0 && local_y < y1 {
                        line_ix = ix;
                        break;
                    }
                    if local_y >= y1 {
                        line_ix = ix;
                    }
                }
                let line_ix = line_ix.min(lines.len().saturating_sub(1));
                let local_x = position.x - bounds.left();
                let local_y_in_line =
                    local_y - y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO);
                let line = &lines[line_ix];
                let local = line
                    .closest_index_for_position(point(local_x, local_y_in_line), line_height)
                    .unwrap_or_else(|ix| ix);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local;
                doc_ix.min(self.content.len())
            }
        }
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            return;
        };

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + &new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            return;
        };

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + &new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let starts = self.last_line_starts.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let offset = range.start.min(self.content.len());
        let line_height = self.effective_line_height(window);

        let (line_ix, local_ix, y_offset) = match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts, lines, offset);
                (line_ix, local_ix, line_height * line_ix as f32)
            }
            TextInputLayout::Wrapped { lines, y_offsets } => {
                let mut ix = starts.partition_point(|&s| s <= offset);
                if ix == 0 {
                    ix = 1;
                }
                let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
                let start = starts.get(line_ix).copied().unwrap_or(0);
                let local = offset.saturating_sub(start).min(lines[line_ix].len());
                (
                    line_ix,
                    local,
                    y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO),
                )
            }
        };

        let (x, y) = match layout {
            TextInputLayout::Plain(lines) => {
                let line = lines.get(line_ix)?;
                (line.x_for_index(local_ix) - self.scroll_x, y_offset)
            }
            TextInputLayout::Wrapped { lines, .. } => {
                let line = lines.get(line_ix)?;
                let p = line
                    .position_for_index(local_ix, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                (p.x, y_offset + p.y)
            }
        };

        let top = bounds.top() + y;
        Some(Bounds::from_corners(
            point(bounds.left() + x, top),
            point(bounds.left() + x + px(2.0), top + px(16.0)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&p)?;
        let layout = self.last_layout.as_ref()?;
        let starts = self.last_line_starts.as_ref()?;
        let line_height = self.effective_line_height(window);
        match layout {
            TextInputLayout::Plain(lines) => {
                let mut line_ix = (local.y / line_height).floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let line = lines.get(line_ix)?;
                let local_x = local.x + self.scroll_x;
                let idx = line.index_for_x(local_x).unwrap_or(line.len());
                let doc_offset = starts.get(line_ix).copied().unwrap_or(0) + idx;
                Some(self.offset_to_utf16(doc_offset))
            }
            TextInputLayout::Wrapped { lines, y_offsets } => {
                let mut line_ix = 0usize;
                for (ix, line) in lines.iter().enumerate() {
                    let y0 = y_offsets.get(ix).copied().unwrap_or(Pixels::ZERO);
                    let rows = line.wrap_boundaries().len().saturating_add(1);
                    let y1 = y0 + line_height * rows as f32;
                    if local.y >= y0 && local.y < y1 {
                        line_ix = ix;
                        break;
                    }
                    if local.y >= y1 {
                        line_ix = ix;
                    }
                }
                let line_ix = line_ix.min(lines.len().saturating_sub(1));
                let line = lines.get(line_ix)?;
                let local_y = local.y - y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO);
                let idx = line
                    .closest_index_for_position(point(local.x, local_y), line_height)
                    .unwrap_or_else(|ix| ix);
                let doc_offset = starts.get(line_ix).copied().unwrap_or(0) + idx;
                Some(self.offset_to_utf16(doc_offset))
            }
        }
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    layout: Option<TextInputLayout>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    line_starts: Option<Vec<usize>>,
    wrap_cache: Option<WrapCache>,
    scroll_x: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let line_height = input.effective_line_height(window);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if input.multiline {
            let line_count = input.content.as_ref().split('\n').count().max(1) as f32;
            if input.soft_wrap
                && let Some(cache) = input.wrap_cache
                && cache.rows > 0
                && cache.width > px(0.0)
            {
                style.size.height = (line_height * cache.rows as f32).into();
            } else {
                style.size.height = (line_height * line_count).into();
            }
        } else {
            style.size.height = line_height.into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style_colors = input.style;
        let soft_wrap = input.soft_wrap && input.multiline;
        let style = window.text_style();
        let has_content = !content.is_empty();

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), style_colors.placeholder)
        } else {
            (content, style_colors.text)
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = input.effective_line_height(window);
        let base_font = style.font();
        let highlights = if !has_content {
            None
        } else {
            Some(Arc::clone(&input.highlights))
        };

        let (line_starts, lines_text): (Vec<usize>, Vec<SharedString>) =
            split_lines_with_starts(&display_text);

        if !soft_wrap {
            let mut scroll_x = if input.multiline {
                px(0.0)
            } else {
                input.scroll_x
            };
            let mut lines = Vec::with_capacity(lines_text.len());
            for (line_ix, line_text) in lines_text.iter().enumerate() {
                let runs = runs_for_line(
                    &base_font,
                    text_color,
                    line_starts[line_ix],
                    line_text.as_ref(),
                    highlights.as_ref().map(|h| h.as_slice()),
                );
                let shaped =
                    window
                        .text_system()
                        .shape_line(line_text.clone(), font_size, &runs, None);
                lines.push(shaped);
            }

            if !input.multiline && !lines.is_empty() {
                let viewport_w = bounds.size.width.max(px(0.0));
                let pad = px(8.0).min(viewport_w / 4.0);
                let (line_ix, local_ix) = line_for_offset(&line_starts, &lines, cursor);
                let cursor_x = lines[line_ix].x_for_index(local_ix);
                let max_scroll_x = (lines[line_ix].width - viewport_w).max(px(0.0));

                let left = scroll_x;
                let right = scroll_x + viewport_w;
                if cursor_x < left + pad {
                    scroll_x = (cursor_x - pad).max(px(0.0));
                } else if cursor_x > right - pad {
                    scroll_x = (cursor_x + pad - viewport_w).max(px(0.0));
                }
                scroll_x = scroll_x.min(max_scroll_x);
            } else {
                scroll_x = px(0.0);
            }

            let mut selections = Vec::with_capacity(lines.len());
            let cursor_quad = if selected_range.is_empty() {
                let (line_ix, local_ix) = line_for_offset(&line_starts, &lines, cursor);
                let x = lines[line_ix].x_for_index(local_ix) - scroll_x;
                let caret_inset_y = px(2.0);
                let caret_h = (line_height - caret_inset_y * 2.0).max(px(2.0));
                let top = bounds.top() + line_height * line_ix as f32 + caret_inset_y;
                Some(fill(
                    Bounds::new(point(bounds.left() + x, top), size(px(1.0), caret_h)),
                    style_colors.cursor,
                ))
            } else {
                for ix in 0..lines.len() {
                    let start = line_starts[ix];
                    let next_start = line_starts
                        .get(ix + 1)
                        .copied()
                        .unwrap_or(display_text.len());
                    let line_len = lines[ix].len();
                    let line_end = start + line_len;

                    let seg_start = selected_range.start.max(start);
                    let seg_end = selected_range.end.min(next_start);
                    if seg_start >= seg_end {
                        continue;
                    }

                    let local_start = seg_start.min(line_end) - start;
                    let local_end = seg_end.min(line_end) - start;

                    let x0 = lines[ix].x_for_index(local_start) - scroll_x;
                    let x1 = lines[ix].x_for_index(local_end) - scroll_x;
                    let top = bounds.top() + line_height * ix as f32;
                    selections.push(fill(
                        Bounds::from_corners(
                            point(bounds.left() + x0, top),
                            point(bounds.left() + x1, top + line_height),
                        ),
                        style_colors.selection,
                    ));
                }
                None
            };

            return PrepaintState {
                layout: Some(TextInputLayout::Plain(lines)),
                cursor: cursor_quad,
                selections,
                line_starts: Some(line_starts),
                wrap_cache: None,
                scroll_x,
            };
        }

        let wrap_width = bounds.size.width.max(px(0.0));
        let mut y_offsets: Vec<Pixels> = Vec::with_capacity(lines_text.len());
        let mut lines: Vec<WrappedLine> = Vec::with_capacity(lines_text.len());
        let mut y = Pixels::ZERO;
        let mut total_rows = 0usize;

        for (line_ix, line_text) in lines_text.iter().enumerate() {
            let runs = runs_for_line(
                &base_font,
                text_color,
                line_starts[line_ix],
                line_text.as_ref(),
                highlights.as_ref().map(|h| h.as_slice()),
            );
            let shaped = window
                .text_system()
                .shape_text(line_text.clone(), font_size, &runs, Some(wrap_width), None)
                .unwrap_or_default();
            let line = shaped.into_iter().next().unwrap_or_default();
            y_offsets.push(y);
            let rows = line.wrap_boundaries().len().saturating_add(1);
            total_rows += rows;
            y += line_height * rows as f32;
            lines.push(line);
        }

        let wrap_cache = Some(WrapCache {
            width: wrap_width.round(),
            rows: total_rows.max(1),
        });

        let mut selections = Vec::with_capacity(total_rows.max(1));
        let cursor_quad = if selected_range.is_empty() {
            let mut ix = line_starts.partition_point(|&s| s <= cursor);
            if ix == 0 {
                ix = 1;
            }
            let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
            let start = line_starts.get(line_ix).copied().unwrap_or(0);
            let local = cursor.saturating_sub(start).min(lines[line_ix].len());
            let caret_inset_y = px(2.0);
            let caret_h = (line_height - caret_inset_y * 2.0).max(px(2.0));
            let pos = lines[line_ix]
                .position_for_index(local, line_height)
                .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
            let top = bounds.top() + y_offsets[line_ix] + pos.y + caret_inset_y;
            Some(fill(
                Bounds::new(point(bounds.left() + pos.x, top), size(px(1.0), caret_h)),
                style_colors.cursor,
            ))
        } else {
            for ix in 0..lines.len() {
                let start = line_starts[ix];
                let next_start = line_starts
                    .get(ix + 1)
                    .copied()
                    .unwrap_or(display_text.len());
                let line_len = lines[ix].len();
                let line_end = start + line_len;

                let seg_start = selected_range.start.max(start);
                let seg_end = selected_range.end.min(next_start);
                if seg_start >= seg_end {
                    continue;
                }

                let local_start = seg_start.min(line_end) - start;
                let local_end = seg_end.min(line_end) - start;

                let start_pos = lines[ix]
                    .position_for_index(local_start, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                let end_pos = lines[ix]
                    .position_for_index(local_end, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));

                let start_row = (start_pos.y / line_height).floor().max(0.0) as usize;
                let end_row = (end_pos.y / line_height).floor().max(0.0) as usize;

                for row in start_row..=end_row {
                    let top = bounds.top() + y_offsets[ix] + line_height * row as f32;
                    let (x0, x1) = if start_row == end_row {
                        (start_pos.x, end_pos.x)
                    } else if row == start_row {
                        (start_pos.x, bounds.size.width)
                    } else if row == end_row {
                        (Pixels::ZERO, end_pos.x)
                    } else {
                        (Pixels::ZERO, bounds.size.width)
                    };
                    selections.push(fill(
                        Bounds::from_corners(
                            point(bounds.left() + x0, top),
                            point(bounds.left() + x1, top + line_height),
                        ),
                        style_colors.selection,
                    ));
                }
            }
            None
        };

        PrepaintState {
            layout: Some(TextInputLayout::Wrapped { lines, y_offsets }),
            cursor: cursor_quad,
            selections,
            line_starts: Some(line_starts),
            wrap_cache,
            scroll_x: px(0.0),
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        if self.input.read(cx).is_selecting {
            let input = self.input.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
                input.update(cx, |input, cx| {
                    if input.is_selecting {
                        input.select_to(input.index_for_mouse_position(event.position), cx);
                    }
                });
            });

            let input = self.input.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, _phase, _window, cx| {
                if event.button != MouseButton::Left {
                    return;
                }
                input.update(cx, |input, _cx| {
                    input.is_selecting = false;
                });
            });
        }

        for selection in prepaint.selections.drain(..) {
            window.paint_quad(selection);
        }
        let line_height = self.input.read(cx).effective_line_height(window);
        if let Some(layout) = prepaint.layout.as_ref() {
            match layout {
                TextInputLayout::Plain(lines) => {
                    for (ix, line) in lines.iter().enumerate() {
                        line.paint(
                            point(
                                bounds.origin.x - prepaint.scroll_x,
                                bounds.origin.y + line_height * ix as f32,
                            ),
                            line_height,
                            window,
                            cx,
                        )
                        .unwrap();
                    }
                }
                TextInputLayout::Wrapped { lines, y_offsets } => {
                    for (ix, line) in lines.iter().enumerate() {
                        let y = y_offsets.get(ix).copied().unwrap_or(Pixels::ZERO);
                        let _ = line.paint(
                            point(bounds.origin.x, bounds.origin.y + y),
                            line_height,
                            TextAlign::Left,
                            Some(bounds),
                            window,
                            cx,
                        );
                    }
                }
            }
        }

        let cursor_blink_visible = self.input.read(cx).cursor_blink_visible;
        if focus_handle.is_focused(window)
            && cursor_blink_visible
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = prepaint.layout.take();
            input.last_line_starts = prepaint.line_starts.clone();
            input.last_bounds = Some(bounds);
            input.last_line_height = line_height;
            input.wrap_cache = prepaint.wrap_cache;
            input.scroll_x = prepaint.scroll_x;
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.style;
        let focus = self.focus_handle.clone();
        let chromeless = self.chromeless;
        let padding = if chromeless { px(0.0) } else { px(8.0) };
        let is_focused = focus.is_focused(window);

        if self.has_focus != is_focused {
            self.has_focus = is_focused;
            self.cursor_blink_visible = true;
            if !is_focused {
                self.cursor_blink_task.take();
            }
        }

        if is_focused && self.cursor_blink_task.is_none() {
            let task = cx.spawn(
                async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| {
                    loop {
                        gpui::Timer::after(Duration::from_millis(800)).await;
                        let should_continue = input
                            .update(cx, |input, cx| {
                                if !input.has_focus {
                                    input.cursor_blink_visible = true;
                                    input.cursor_blink_task = None;
                                    cx.notify();
                                    return false;
                                }

                                if input.selected_range.is_empty() {
                                    input.cursor_blink_visible = !input.cursor_blink_visible;
                                } else {
                                    input.cursor_blink_visible = true;
                                }
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);

                        if !should_continue {
                            break;
                        }
                    }
                },
            );
            self.cursor_blink_task = Some(task);
        }

        let mut outer = div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .track_focus(&focus)
            .key_context("TextInput")
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _e: &MouseDownEvent, _w, cx| {
                    if this.suppress_right_click {
                        return;
                    }
                    cx.stop_propagation();
                    if !this.selected_range.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            this.content[this.selected_range.clone()].to_string(),
                        ));
                    }
                }),
            )
            .line_height(self.effective_line_height(window))
            .text_size(px(13.0))
            .when(self.multiline, |d| d.items_start())
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .p(padding)
                    .overflow_hidden()
                    .child(TextElement { input: cx.entity() }),
            );

        if !chromeless {
            outer = outer
                .bg(style.background)
                .border_1()
                .rounded(px(style.radius));

            if is_focused {
                outer = outer.border_color(style.focus_border);
            } else {
                outer = outer
                    .border_color(style.border)
                    .hover(move |s| s.border_color(style.hover_border));
            }

            outer = outer.focus(move |s| s.border_color(style.focus_border));
        }

        outer
    }
}

fn runs_for_line(
    base_font: &gpui::Font,
    base_color: gpui::Hsla,
    line_start: usize,
    line_text: &str,
    highlights: Option<&[(Range<usize>, gpui::HighlightStyle)]>,
) -> Vec<TextRun> {
    if line_text.is_empty() {
        return Vec::new();
    }

    let Some(highlights) = highlights else {
        return vec![TextRun {
            len: line_text.len(),
            font: base_font.clone(),
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
    };

    let line_end = line_start + line_text.len();
    let mut line_highlights: Vec<(usize, usize, &gpui::HighlightStyle)> = Vec::new();
    for (range, style) in highlights {
        if range.end <= line_start {
            continue;
        }
        if range.start >= line_end {
            break;
        }
        let seg_start = range.start.max(line_start) - line_start;
        let seg_end = range.end.min(line_end) - line_start;
        if seg_start < seg_end {
            line_highlights.push((seg_start, seg_end, style));
        }
    }

    if line_highlights.is_empty() {
        return vec![TextRun {
            len: line_text.len(),
            font: base_font.clone(),
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
    }

    let mut boundaries = Vec::with_capacity(line_highlights.len() * 2 + 2);
    boundaries.push(0usize);
    boundaries.push(line_text.len());
    for (start, end, _) in &line_highlights {
        boundaries.push(*start);
        boundaries.push(*end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut runs = Vec::with_capacity(boundaries.len().saturating_sub(1));
    for w in boundaries.windows(2) {
        let a = w[0];
        let b = w[1];
        if a >= b {
            continue;
        }
        let style = line_highlights
            .iter()
            .rev()
            .find(|(start, end, _)| *start <= a && *end >= b)
            .map(|(_, _, style)| *style);

        let mut font = base_font.clone();
        let mut color = base_color;
        let mut background_color = None;
        let mut underline = None;
        let mut strikethrough = None;

        if let Some(style) = style {
            if let Some(next_color) = style.color {
                color = next_color;
            }
            if let Some(next_weight) = style.font_weight {
                font.weight = next_weight;
            }
            if let Some(next_style) = style.font_style {
                font.style = next_style;
            }
            background_color = style.background_color;
            underline = style.underline;
            strikethrough = style.strikethrough;
            if let Some(fade_out) = style.fade_out {
                color.a *= (1.0 - fade_out).clamp(0.0, 1.0);
            }
        }

        runs.push(TextRun {
            len: b - a,
            font,
            color,
            background_color,
            underline,
            strikethrough,
        });
    }

    runs
}

fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.a = alpha;
    color
}

fn split_lines_with_starts(text: &SharedString) -> (Vec<usize>, Vec<SharedString>) {
    let s = text.as_ref();
    let mut starts = Vec::with_capacity(8);
    let mut lines = Vec::with_capacity(8);
    starts.push(0);
    let mut start = 0usize;
    for (ix, b) in s.bytes().enumerate() {
        if b == b'\n' {
            lines.push(s[start..ix].to_string().into());
            start = ix + 1;
            starts.push(start);
        }
    }
    lines.push(s[start..].to_string().into());
    (starts, lines)
}

fn line_for_offset(starts: &[usize], lines: &[ShapedLine], offset: usize) -> (usize, usize) {
    let mut ix = starts.partition_point(|&s| s <= offset);
    if ix == 0 {
        ix = 1;
    }
    let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
    let start = starts.get(line_ix).copied().unwrap_or(0);
    let local = offset.saturating_sub(start).min(lines[line_ix].len());
    (line_ix, local)
}
