use super::*;

const REFLOG_COLUMN_HANDLE_PX: f32 = 8.0;
const REFLOG_COLUMN_MIN_WIDTH_PX: f32 = 40.0;
pub(super) const REFLOG_MESSAGE_MIN_WIDTH_PX: f32 = 70.0;
const REFLOG_COLUMN_GAP_COUNT: f32 = 5.0;

fn reflog_table_reserved_width(ui_scale: UiScale) -> Pixels {
    reflog_column(
        ui_scale,
        REFLOG_MARKER_COLUMN_PX + REFLOG_AVATAR_COLUMN_PX + REFLOG_MESSAGE_MIN_WIDTH_PX,
    ) + ui_scale.px(REFLOG_TABLE_PADDING_PX * 2.0 + REFLOG_CELL_GAP_PX * REFLOG_COLUMN_GAP_COUNT)
        + components::Scrollbar::gutter(components::ScrollbarAxis::Vertical)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReflogColumn {
    Selector,
    Sha,
    Date,
}

impl ReflogColumn {
    fn label(self) -> &'static str {
        match self {
            Self::Selector => "Selector",
            Self::Sha => "SHA",
            Self::Date => "Date",
        }
    }

    fn header_id(self) -> &'static str {
        match self {
            Self::Selector => "reflog_header_selector",
            Self::Sha => "reflog_header_sha",
            Self::Date => "reflog_header_date",
        }
    }

    fn handle_id(self) -> &'static str {
        match self {
            Self::Selector => "reflog_resize_selector",
            Self::Sha => "reflog_resize_sha",
            Self::Date => "reflog_resize_date",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ReflogColumnWidths {
    selector: f32,
    sha: f32,
    date: f32,
}

impl Default for ReflogColumnWidths {
    fn default() -> Self {
        Self {
            selector: REFLOG_SELECTOR_COLUMN_PX,
            sha: REFLOG_SHA_COLUMN_PX,
            date: REFLOG_DATE_COLUMN_PX,
        }
    }
}

impl ReflogColumnWidths {
    pub(super) fn fitted(mut self, table_width: Option<Pixels>, ui_scale: UiScale) -> Self {
        let Some(table_width) = table_width else {
            return self;
        };
        let available = f32::from(table_width - reflog_table_reserved_width(ui_scale))
            / f32::from(reflog_column(ui_scale, 1.0));
        let mut overflow = (self.selector + self.sha + self.date - available).max(0.0);
        for column in [
            ReflogColumn::Date,
            ReflogColumn::Sha,
            ReflogColumn::Selector,
        ] {
            let width = self.get(column);
            let reduction = overflow.min((width - REFLOG_COLUMN_MIN_WIDTH_PX).max(0.0));
            self.set(column, width - reduction);
            overflow -= reduction;
        }
        self
    }

    pub(super) fn get(self, column: ReflogColumn) -> f32 {
        match column {
            ReflogColumn::Selector => self.selector,
            ReflogColumn::Sha => self.sha,
            ReflogColumn::Date => self.date,
        }
    }

    fn set(&mut self, column: ReflogColumn, width: f32) {
        match column {
            ReflogColumn::Selector => self.selector = width,
            ReflogColumn::Sha => self.sha = width,
            ReflogColumn::Date => self.date = width,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ReflogColumnResizeState {
    column: ReflogColumn,
    start_x: Pixels,
    start_width: Pixels,
    max_width: Pixels,
}

impl ReflogPaneView {
    pub(super) fn render_table_header(
        &self,
        theme: AppTheme,
        ui_scale: UiScale,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        div()
            .id("reflog_table_header")
            .flex()
            .flex_none()
            .items_center()
            .px(ui_scale.px(REFLOG_TABLE_PADDING_PX))
            .py(ui_scale.px(3.0))
            .gap(ui_scale.px(REFLOG_CELL_GAP_PX))
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .on_prepaint(
                cx.listener(move |this, info: &gpui::InteractivityPrepaint, _, cx| {
                    if let Some(panel) = this.panels.get_mut(&repo_id)
                        && panel.table_width != Some(info.bounds.size.width)
                    {
                        panel.table_width = Some(info.bounds.size.width);
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .w(reflog_column(ui_scale, REFLOG_MARKER_COLUMN_PX))
                    .flex_none(),
            )
            .child(self.render_column_header(ReflogColumn::Selector, theme, ui_scale, repo_id, cx))
            .child(self.render_column_header(ReflogColumn::Sha, theme, ui_scale, repo_id, cx))
            .child(
                div()
                    .w(reflog_column(ui_scale, REFLOG_AVATAR_COLUMN_PX))
                    .flex_none(),
            )
            .child(self.render_column_header(ReflogColumn::Date, theme, ui_scale, repo_id, cx))
            .child(
                div()
                    .debug_selector(|| "reflog_header_message".into())
                    .flex_1()
                    .min_w(
                        reflog_column(ui_scale, REFLOG_MESSAGE_MIN_WIDTH_PX)
                            + components::Scrollbar::gutter(components::ScrollbarAxis::Vertical),
                    )
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .pr(components::Scrollbar::gutter(
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child("Message"),
            )
            .into_any()
    }

    fn render_column_header(
        &self,
        column: ReflogColumn,
        theme: AppTheme,
        ui_scale: UiScale,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let panel = &self.panels[&repo_id];
        let widths = panel.column_widths.fitted(panel.table_width, ui_scale);
        let dragging = panel
            .column_resize
            .is_some_and(|state| state.column == column);
        let id = column.handle_id();
        let handle_width = ui_scale.px(REFLOG_COLUMN_HANDLE_PX);
        let handle = div()
            .id(id)
            .debug_selector(|| id.into())
            .group(id)
            .absolute()
            .top_0()
            .bottom_0()
            .right(-(ui_scale.px(REFLOG_CELL_GAP_PX) + handle_width) / 2.0)
            .w(handle_width)
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .child(components::resize_grip(
                theme,
                self.ui_scale_percent,
                id,
                components::ResizeGripAxis::Vertical,
                dragging,
                Some(theme.colors.stroke.subtle),
            ))
            .on_resize_drag(column)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    crate::press_gesture::claim_press(cx);
                    crate::text_selection_owner::preserve(cx);
                    let Some(panel) = this.panels.get_mut(&repo_id) else {
                        return;
                    };
                    let Some(table_width) = panel.table_width else {
                        return;
                    };
                    let widths = panel.column_widths.fitted(panel.table_width, ui_scale);
                    let start_width = reflog_column(ui_scale, widths.get(column));
                    let other_columns = reflog_column(
                        ui_scale,
                        widths.selector + widths.sha + widths.date - widths.get(column),
                    );
                    panel.column_resize = Some(ReflogColumnResizeState {
                        column,
                        start_x: event.position.x,
                        start_width,
                        max_width: (table_width
                            - other_columns
                            - reflog_table_reserved_width(ui_scale))
                        .max(reflog_column(ui_scale, REFLOG_COLUMN_MIN_WIDTH_PX)),
                    });
                    cx.notify();
                }),
            )
            .on_drag_move(cx.listener(
                move |this, event: &gpui::DragMoveEvent<ReflogColumn>, _, cx| {
                    if *event.drag(cx) != column {
                        return;
                    }
                    let Some(panel) = this.panels.get_mut(&repo_id) else {
                        return;
                    };
                    let Some(state) = panel.column_resize else {
                        return;
                    };
                    let width = (state.start_width + event.event.position.x - state.start_x).clamp(
                        reflog_column(ui_scale, REFLOG_COLUMN_MIN_WIDTH_PX),
                        state.max_width,
                    );
                    let design_width = f32::from(width) / f32::from(reflog_column(ui_scale, 1.0));
                    panel.column_widths.set(column, design_width);
                    cx.notify();
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(panel) = this.panels.get_mut(&repo_id) {
                        panel.column_resize = None;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if let Some(panel) = this.panels.get_mut(&repo_id) {
                        panel.column_resize = None;
                    }
                    cx.notify();
                }),
            );

        div()
            .debug_selector(|| column.header_id().into())
            .relative()
            .w(reflog_column(ui_scale, widths.get(column)))
            .flex_none()
            .child(
                div()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(column.label()),
            )
            .child(handle)
            .into_any()
    }
}
