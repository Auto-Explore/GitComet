use super::state::*;
use super::*;

impl TextInput {
    pub fn new(options: TextInputOptions, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::from_options(options, cx)
    }

    pub fn new_inert(options: TextInputOptions, cx: &mut Context<Self>) -> Self {
        Self::from_options(options, cx)
    }

    pub(super) fn from_options(options: TextInputOptions, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let selection_owner_observer = crate::text_selection_owner::observe(cx, |this, cx| {
            this.clear_selection_on_ownership_loss(cx);
        });
        Self {
            focus_handle,
            text_size: options
                .text_size
                .filter(|size| size.is_finite() && *size > 0.0),
            probe_action: 0,
            content: TextModel::new(),
            placeholder: options.placeholder,
            leading_icon: options.leading_icon,
            multiline: options.multiline,
            read_only: options.read_only,
            chromeless: options.chromeless,
            display_text: false,
            soft_wrap: options.soft_wrap,
            min_lines: options.min_lines,
            display_truncation: None,
            masked: false,
            line_ending: if cfg!(windows) { "\r\n" } else { "\n" },
            tab_size: TEXT_INPUT_WRAP_TAB_STOP_COLUMNS,
            style: TextInputStyle::from_theme(AppTheme::gitcomet_dark()),
            line_height_override: None,
            appearance_metrics: crate::appearance::current(cx),
            editor_font: false,
            editor_line_height: px(20.0),
            vertical_padding_override: None,
            highlight: HighlightState::new(),
            layout: LayoutState::new(),
            wrap: WrapState::new(),
            content_width_cache: None,
            selection: SelectionState::new(),
            interaction: InteractionState::new(),
            protected_ranges: Arc::from([]),
            selection_owner: Default::default(),
            _selection_owner_observer: selection_owner_observer,
        }
    }

    pub fn text(&self) -> &str {
        self.content.as_ref()
    }

    pub fn text_snapshot(&self) -> TextModelSnapshot {
        self.content.snapshot()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn theme_for_test(&self) -> AppTheme {
        self.style.menu_theme
    }

    /// Restyles the input for its window's theme, so shaped lines drawn with
    /// the old colours are not reused.
    pub(super) fn sync_theme(&mut self, cx: &App) {
        let style = TextInputStyle::from_theme(AppTheme::current(cx));
        if self.style != style {
            self.style = style;
            self.bump_shape_style_epoch();
        }
    }

    pub fn set_chromeless(&mut self, chromeless: bool, cx: &mut Context<Self>) {
        if self.chromeless == chromeless {
            return;
        }
        self.chromeless = chromeless;
        self.invalidate_layout_caches();
        cx.notify();
    }

    pub fn set_leading_icon(&mut self, leading_icon: Option<&'static str>, cx: &mut Context<Self>) {
        if self.leading_icon == leading_icon {
            return;
        }
        self.leading_icon = leading_icon;
        self.invalidate_layout_caches();
        cx.notify();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn debug_text_color(&self) -> gpui::Hsla {
        self.style.text
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn debug_displayed_single_line(&self) -> Option<&str> {
        match self.layout.last.as_ref()? {
            TextInputLayout::TruncatedSingleLine(line) => Some(line.display_text.as_ref()),
            _ => None,
        }
    }

    /// Opt into label typography without changing the defaults for form inputs.
    pub fn set_display_text(&mut self, cx: &mut Context<Self>) {
        assert!(self.read_only && self.chromeless);
        self.display_text = true;
        cx.notify();
    }

    pub fn set_line_height(&mut self, line_height: Option<Pixels>, cx: &mut Context<Self>) {
        if self.line_height_override == line_height {
            return;
        }
        self.line_height_override = line_height;
        cx.notify();
    }

    /// `padding` is in design px, so it follows the UI zoom.
    pub fn set_vertical_padding(&mut self, padding: Option<f32>, cx: &mut Context<Self>) {
        if self.vertical_padding_override == padding {
            return;
        }
        self.vertical_padding_override = padding;
        cx.notify();
    }

    pub fn set_editor_font(&mut self, cx: &mut Context<Self>) {
        self.editor_font = true;
        cx.notify();
    }

    pub(super) fn effective_line_height(&self, window: &Window) -> Pixels {
        if let Some(size) = self.text_size {
            return self
                .line_height_override
                .unwrap_or_else(|| crate::ui_scale::design_px_from_window(size * 1.5, window));
        }
        if self.display_text {
            return window.line_height();
        }
        if self.editor_font {
            return self.editor_line_height;
        }
        let line_height = self
            .line_height_override
            .unwrap_or_else(|| window.line_height());
        line_height.max(crate::ui_scale::design_px_from_window(
            self.appearance_metrics.ui_text(20.0),
            window,
        ))
    }

    /// The explicit line height set by `set_line_height`, if any. Tests use it to
    /// assert a separate line-number gutter advances at the same rate as the buffer.
    #[cfg(any(test, feature = "test-support"))]
    pub fn line_height_override(&self) -> Option<Pixels> {
        self.line_height_override
    }

    pub fn take_enter_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.enter_pressed)
    }

    pub fn take_escape_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.escape_pressed)
    }

    pub fn clear_transient_key_presses(&mut self) {
        self.interaction.enter_pressed = false;
        self.interaction.escape_pressed = false;
        self.interaction.arrow_up_pressed = false;
        self.interaction.arrow_left_at_start_pressed = false;
        self.interaction.arrow_right_at_end_pressed = false;
        self.interaction.document_home_pressed = false;
        self.interaction.document_end_pressed = false;
        self.interaction.page_up_pressed = false;
        self.interaction.page_down_pressed = false;

        self.interaction.arrow_down_pressed = false;
        self.interaction.tab_pressed = false;
        self.interaction.shift_tab_pressed = false;
    }

    pub fn take_document_home_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.document_home_pressed)
    }

    pub fn take_document_end_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.document_end_pressed)
    }

    pub fn take_page_up_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.page_up_pressed)
    }

    pub fn take_page_down_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.page_down_pressed)
    }

    pub fn take_arrow_up_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.arrow_up_pressed)
    }

    pub fn take_arrow_left_at_start_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.arrow_left_at_start_pressed)
    }

    pub fn take_arrow_right_at_end_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.arrow_right_at_end_pressed)
    }

    pub fn take_arrow_down_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.arrow_down_pressed)
    }

    pub fn take_tab_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.tab_pressed)
    }

    pub fn take_shift_tab_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.shift_tab_pressed)
    }

    pub fn set_submit_on_enter(&mut self, submit_on_enter: bool) {
        self.interaction.submit_on_enter = submit_on_enter;
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        if self.read_only == read_only {
            return;
        }
        self.read_only = read_only;
        if !self.read_only && self.display_truncation.is_some() {
            self.display_truncation = None;
            self.invalidate_layout_caches();
        }
        cx.notify();
    }

    pub fn set_display_truncation(
        &mut self,
        display_truncation: Option<TextTruncationProfile>,
        cx: &mut Context<Self>,
    ) {
        debug_assert!(
            display_truncation.is_none() || (self.read_only && !self.multiline),
            "display truncation is only supported for single-line read-only text inputs"
        );
        let next = display_truncation.filter(|_| self.read_only && !self.multiline);
        if self.display_truncation == next {
            return;
        }
        self.display_truncation = next;
        self.layout.scroll_x = px(0.0);
        self.invalidate_layout_caches();
        cx.notify();
    }

    pub fn set_suppress_right_click(&mut self, suppress: bool) {
        self.interaction.suppress_right_click = suppress;
    }

    pub fn set_vertical_scroll_handle(&mut self, handle: Option<ScrollHandle>) {
        self.interaction.vertical_scroll_handle = handle;
        // Offsets of another handle say nothing about this one.
        self.layout.painted_scroll_offset = None;
    }

    /// Enable content-width layout: a multiline input lays out at its widest-line
    /// width so an outer `overflow_scroll` container can scroll it horizontally
    /// and drive a real horizontal `max_offset` on the shared scroll handle.
    pub fn set_content_width_layout(&mut self, enabled: bool) {
        if enabled && self.content_width_cache.is_none() {
            self.rebuild_content_width_cache();
        }
        self.interaction.content_width_layout = enabled;
    }

    /// Whether the buffer is currently wrapping long lines.
    #[cfg(any(test, feature = "test-support"))]
    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    /// How many visual rows each logical line occupies under the current wrap
    /// width, or an empty slice when the counts are not yet in step with the
    /// text (before the first prepaint, or between an edit and the wrap pass).
    ///
    /// This is the very array the element lays the buffer out from — it builds
    /// its y-offsets by scanning it — so a gutter that projects through it lands
    /// on the same rows as the text, whatever the wrap estimate got right or
    /// wrong. Deriving the gutter independently is what would drift.
    pub fn wrap_row_counts(&self) -> &[usize] {
        if !(self.multiline && self.soft_wrap) {
            return &[];
        }
        &self.wrap.row_counts
    }

    pub fn set_soft_wrap(&mut self, soft_wrap: bool, cx: &mut Context<Self>) {
        if soft_wrap {
            self.layout.scroll_x = px(0.0);
            if let Some(handle) = &self.interaction.vertical_scroll_handle {
                let offset = handle.offset();
                if offset.x != px(0.0) {
                    handle.set_offset(point(px(0.0), offset.y));
                }
            }
        }
        if self.soft_wrap == soft_wrap {
            return;
        }
        self.soft_wrap = soft_wrap;
        self.invalidate_layout_caches();
        if soft_wrap {
            self.request_wrap_recompute();
        }
        if !soft_wrap {
            self.wrap.last_rows = None;
        }
        cx.notify();
    }

    pub fn set_masked(&mut self, masked: bool, cx: &mut Context<Self>) {
        if self.masked == masked {
            return;
        }
        self.masked = masked;
        self.invalidate_layout_caches();
        if self.multiline && self.soft_wrap {
            self.request_wrap_recompute();
        }
        cx.notify();
    }

    /// Columns a tab advances to. Tabs stay tab characters in the text.
    pub fn set_tab_size(&mut self, tab_size: usize, cx: &mut Context<Self>) {
        let tab_size = tab_size.max(1);
        if self.tab_size == tab_size {
            return;
        }
        self.tab_size = tab_size;
        self.bump_shape_style_epoch();
        self.rebuild_content_width_cache_if_present();
        cx.notify();
    }

    pub fn set_line_ending(&mut self, line_ending: &'static str) {
        self.line_ending = line_ending;
    }

    /// Detect CRLF or LF from the content, using the OS default only when
    /// there are no line breaks yet.
    pub fn detect_line_ending(content: &str) -> &'static str {
        if content.contains("\r\n") {
            "\r\n"
        } else if content.contains('\n') {
            "\n"
        } else if cfg!(windows) {
            "\r\n"
        } else {
            "\n"
        }
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
