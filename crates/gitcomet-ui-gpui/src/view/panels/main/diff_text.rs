use super::{
    AppTheme, DiffContentMode, DiffRenderer, DiffSplitResizeDragGhost, DiffSplitResizeHandle,
    DiffSplitResizeState, DiffTextEmptySpaceDecoration, DiffTextRegion, DiffViewMode, Loadable,
    PANE_RESIZE_HANDLE_PX, components, diff_split_column_widths, diff_split_drag_params,
    empty_diff_text_document, next_diff_split_drag_ratio,
};
use crate::view::panes::diff::DiffBody;
use crate::view::panes::diff::helpers::state::DiffHorizontalScrollColumn;
use gitcomet_core::diff_engine::DiffEngineStatus;
use gpui::{
    AnyElement, CursorStyle, MouseButton, MouseDownEvent, Window, div, prelude::*, px, uniform_list,
};

impl DiffRenderer {
    pub(super) fn render_selected_patch_diff(
        &mut self,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        match self.rendered_patch_diff_loadable().cloned() {
            None => components::empty_state(theme, "Diff", "No repository.").into_any_element(),
            Some(Loadable::NotLoaded) => {
                components::empty_state(theme, "Diff", "Select a file.").into_any_element()
            }
            Some(Loadable::Loading) => {
                components::empty_state(theme, "Diff", "Loading").into_any_element()
            }
            Some(Loadable::Error(error)) => {
                components::empty_state(theme, "Diff", error).into_any_element()
            }
            Some(Loadable::Ready(_)) => {
                self.ensure_rendered_patch_diff_cache(cx);
                if self.rendered_patch_diff_cache_is_current() {
                    self.render_text_diff(theme, window, cx)
                } else {
                    components::empty_state(theme, "Diff", "Processing patch…").into_any_element()
                }
            }
        }
    }

    pub(super) fn collapsed_difftastic_has_only_text_changes(&self) -> bool {
        self.is_collapsed_diff_projection_active()
            && matches!(
                self.diff_engine_status,
                DiffEngineStatus::Difftastic {
                    syntactic_changes: false,
                    text_differs: true,
                }
            )
    }

    pub(super) fn render_text_diff(
        &mut self,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        self.ensure_diff_visible_indices();
        self.ensure_diff_wrap_visible_rows(window, cx);
        self.maybe_autoscroll_diff_to_first_change();

        if self.collapsed_difftastic_has_only_text_changes() {
            return components::empty_state(
                theme,
                "Difftastic - Collapsed mode",
                "No syntactic changes; text differs",
            )
            .debug_selector(|| "difftastic_text_only_canvas".to_string())
            .size_full()
            .text_center()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .child(
                        components::Button::new("difftastic_show_full", "Show full text").on_click(
                            theme,
                            cx,
                            |this, _, _, cx| this.set_diff_content_mode(DiffContentMode::Full, cx),
                        ),
                    )
                    .child(
                        components::Button::new(
                            "difftastic_show_line_changes",
                            "Show line changes",
                        )
                        .on_click(theme, cx, |this, _, _, cx| this.show_line_changes(cx)),
                    ),
            )
            .into_any_element();
        }

        let total_len = if self.diff_surface().body == DiffBody::Patch {
            self.patch_diff_row_len()
        } else if self.is_collapsed_diff_projection_active() {
            self.collapsed_diff_visible_rows.len()
        } else {
            match self.diff_view {
                DiffViewMode::Inline => self.file_diff_inline_row_len(),
                DiffViewMode::Split => self.file_diff_split_row_len(),
            }
        };
        if total_len == 0 {
            let message = if self.diff_surface().body == DiffBody::Patch {
                "No changes."
            } else {
                "Empty file."
            };
            empty_diff_text_document(
                cx.entity(),
                DiffTextRegion::Inline,
                components::empty_state(theme, "Diff", message).into_any_element(),
            )
        } else if self.diff_visible_len() == 0 {
            components::empty_state(theme, "Diff", "Nothing to render.").into_any_element()
        } else {
            let markers = self.diff_scrollbar_markers_cache.clone();
            match self.diff_view {
                DiffViewMode::Inline => {
                    let horizontal_scrollbar_gutter =
                        components::Scrollbar::gutter(components::ScrollbarAxis::Horizontal);
                    let scrollbar_gutter = self.diff_vertical_scrollbar_gutter_for_column(
                        DiffHorizontalScrollColumn::Primary,
                        self.diff_scroll.clone(),
                    );
                    let list = uniform_list(
                        "diff",
                        self.diff_list_len(),
                        cx.processor(Self::render_projected_inline),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .pb(if self.diff_word_wrap {
                        px(0.0)
                    } else {
                        horizontal_scrollbar_gutter
                    })
                    .track_scroll(&self.diff_scroll)
                    .with_decoration(DiffTextEmptySpaceDecoration {
                        view: cx.entity(),
                        region: DiffTextRegion::Inline,
                    })
                    .when(!self.diff_word_wrap, |list| {
                        list.with_horizontal_sizing_behavior(
                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                        )
                    });
                    div()
                        .id("diff_scroll_container")
                        .relative()
                        .h_full()
                        .min_h(px(0.0))
                        .bg(theme.colors.surface.canvas)
                        .font_family(editor_font_family.clone())
                        .child(
                            div()
                                .h_full()
                                .min_h(px(0.0))
                                .pr(scrollbar_gutter)
                                .child(list),
                        )
                        // Anchored to the rows container so the
                        // handle's hover highlight matches the
                        // annotation column height exactly.
                        .when(self.annotation_active(), |d| {
                            d.child(self.annotate_resize_handle(ui_scale_percent, theme, cx))
                        })
                        .child(
                            components::Scrollbar::new("diff_scrollbar", self.diff_scroll.clone())
                                .markers(markers)
                                .always_visible()
                                .render(theme),
                        )
                        .when(!self.diff_word_wrap, |d| {
                            d.child(Self::render_diff_horizontal_scrollbar(
                                theme,
                                "diff_hscrollbar",
                                self.diff_scroll.clone(),
                                scrollbar_gutter,
                                "diff_hscrollbar",
                            ))
                        })
                        .into_any_element()
                }
                DiffViewMode::Split => {
                    self.sync_diff_split_scroll();
                    let vertical_sync_enabled = self.diff_scroll_sync.includes_vertical();
                    let count = self.diff_list_len();
                    let horizontal_scrollbar_gutter =
                        components::Scrollbar::gutter(components::ScrollbarAxis::Horizontal);
                    let left_scrollbar_gutter = self.diff_vertical_scrollbar_gutter_for_column(
                        DiffHorizontalScrollColumn::Primary,
                        self.diff_scroll.clone(),
                    );
                    let right_scrollbar_gutter = self.diff_vertical_scrollbar_gutter_for_column(
                        DiffHorizontalScrollColumn::SplitRight,
                        self.diff_split_right_scroll.clone(),
                    );
                    let shared_scrollbar_gutter = if vertical_sync_enabled {
                        left_scrollbar_gutter
                    } else {
                        px(0.0)
                    };
                    let handle_w = px(PANE_RESIZE_HANDLE_PX);
                    let main_w =
                        (self.main_pane_content_width(cx) - shared_scrollbar_gutter).max(px(0.0));
                    let (_, min_col_w) = diff_split_drag_params(main_w);
                    let (left_w, right_w) = diff_split_column_widths(main_w, self.diff_split_ratio);
                    let left = uniform_list(
                        "diff_split_left",
                        count,
                        cx.processor(Self::render_projected_left),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .pb(if self.diff_word_wrap {
                        px(0.0)
                    } else {
                        horizontal_scrollbar_gutter
                    })
                    .track_scroll(&self.diff_scroll)
                    .with_decoration(DiffTextEmptySpaceDecoration {
                        view: cx.entity(),
                        region: DiffTextRegion::SplitLeft,
                    })
                    .when(!self.diff_word_wrap, |list| {
                        list.with_horizontal_sizing_behavior(
                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                        )
                    });
                    let right = uniform_list(
                        "diff_split_right",
                        count,
                        cx.processor(Self::render_projected_right),
                    )
                    .h_full()
                    .min_h(px(0.0))
                    .pb(if self.diff_word_wrap {
                        px(0.0)
                    } else {
                        horizontal_scrollbar_gutter
                    })
                    .track_scroll(&self.diff_split_right_scroll)
                    .with_decoration(DiffTextEmptySpaceDecoration {
                        view: cx.entity(),
                        region: DiffTextRegion::SplitRight,
                    })
                    .when(!self.diff_word_wrap, |list| {
                        list.with_horizontal_sizing_behavior(
                            gpui::ListHorizontalSizingBehavior::Unconstrained,
                        )
                    });
                    // Built before `resize_handle` captures `cx`.
                    let split_annotate_handle = self
                        .annotation_active()
                        .then(|| self.annotate_resize_handle(ui_scale_percent, theme, cx));

                    let split_dragging = self.diff_split_resize.is_some();
                    let resize_handle = |id: &'static str| {
                        div()
                            .id(id)
                            .group(id)
                            .w(handle_w)
                            .h_full()
                            .cursor(CursorStyle::ResizeLeftRight)
                            .child(components::resize_grip(
                                theme,
                                ui_scale_percent,
                                id,
                                components::ResizeGripAxis::Vertical,
                                split_dragging,
                                Some(theme.colors.stroke.default),
                            ))
                            .on_drag(
                                DiffSplitResizeHandle::Divider,
                                |_handle, _offset, _window, cx| {
                                    cx.new(|_cx| DiffSplitResizeDragGhost)
                                },
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                                    cx.stop_propagation();
                                    crate::press_gesture::claim_press(cx);
                                    crate::text_selection_owner::preserve(cx);
                                    this.diff_split_resize = Some(DiffSplitResizeState {
                                        handle: DiffSplitResizeHandle::Divider,
                                        start_x: e.position.x,
                                        start_ratio: this.diff_split_ratio,
                                    });
                                    cx.notify();
                                }),
                            )
                            .on_drag_move(cx.listener(
                                move |this,
                                      e: &gpui::DragMoveEvent<DiffSplitResizeHandle>,
                                      _w,
                                      cx| {
                                    let Some(state) = this.diff_split_resize else {
                                        return;
                                    };
                                    if state.handle != *e.drag(cx) {
                                        return;
                                    }

                                    let scrollbar_gutter =
                                        if this.diff_scroll_sync.includes_vertical() {
                                            components::Scrollbar::visible_gutter(
                                                this.diff_scroll.clone(),
                                                components::ScrollbarAxis::Vertical,
                                            )
                                        } else {
                                            px(0.0)
                                        };
                                    let main_w = (this.main_pane_content_width(cx)
                                        - scrollbar_gutter)
                                        .max(px(0.0));
                                    let available = (main_w - handle_w).max(px(0.0));
                                    let dx = e.event.position.x - state.start_x;
                                    match next_diff_split_drag_ratio(
                                        available,
                                        min_col_w,
                                        state.start_ratio,
                                        dx,
                                    ) {
                                        None => {
                                            if (this.diff_split_ratio - 0.5).abs() > f32::EPSILON {
                                                this.diff_split_ratio = 0.5;
                                                cx.notify();
                                            }
                                        }
                                        Some(next_ratio) => {
                                            if (this.diff_split_ratio - next_ratio).abs()
                                                > f32::EPSILON
                                            {
                                                this.diff_split_ratio = next_ratio;
                                                cx.notify();
                                            }
                                        }
                                    }
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _e, _w, cx| {
                                    if this.diff_split_resize.take().is_some() {
                                        cx.notify();
                                    }
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _e, _w, cx| {
                                    if this.diff_split_resize.take().is_some() {
                                        cx.notify();
                                    }
                                }),
                            )
                    };

                    div()
                        .id("diff_split_scroll_container")
                        .relative()
                        .h_full()
                        .min_h(px(0.0))
                        .flex()
                        .flex_col()
                        .bg(theme.colors.surface.canvas)
                        .font_family(editor_font_family.clone())
                        .child(
                            div()
                                .relative()
                                .pr(shared_scrollbar_gutter)
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h(px(0.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h(px(0.0))
                                        .flex()
                                        .child(
                                            div()
                                                .relative()
                                                .w(left_w)
                                                .min_w(px(0.0))
                                                .h_full()
                                                .child(
                                                    div()
                                                        .h_full()
                                                        .min_h(px(0.0))
                                                        .pr(if vertical_sync_enabled {
                                                            px(0.0)
                                                        } else {
                                                            left_scrollbar_gutter
                                                        })
                                                        .child(left),
                                                )
                                                .when(!vertical_sync_enabled, |d| {
                                                    d.child(
                                                        components::Scrollbar::new(
                                                            "diff_split_left_scrollbar",
                                                            self.diff_scroll.clone(),
                                                        )
                                                        .markers(markers.clone())
                                                        .always_visible()
                                                        .render(theme),
                                                    )
                                                })
                                                .when(!self.diff_word_wrap, |d| {
                                                    d.child(Self::render_diff_horizontal_scrollbar(
                                                        theme,
                                                        "diff_split_left_hscrollbar",
                                                        self.diff_scroll.clone(),
                                                        if vertical_sync_enabled {
                                                            px(0.0)
                                                        } else {
                                                            left_scrollbar_gutter
                                                        },
                                                        "diff_split_left_hscrollbar",
                                                    ))
                                                })
                                                .when_some(split_annotate_handle, |d, handle| {
                                                    d.child(handle)
                                                }),
                                        )
                                        .child(resize_handle("diff_split_resize_handle_body"))
                                        .child(
                                            div()
                                                .relative()
                                                .w(right_w)
                                                .min_w(px(0.0))
                                                .h_full()
                                                .child(
                                                    div()
                                                        .h_full()
                                                        .min_h(px(0.0))
                                                        .pr(if vertical_sync_enabled {
                                                            px(0.0)
                                                        } else {
                                                            right_scrollbar_gutter
                                                        })
                                                        .child(right),
                                                )
                                                .when(!vertical_sync_enabled, |d| {
                                                    d.child(
                                                        components::Scrollbar::new(
                                                            "diff_split_right_scrollbar",
                                                            self.diff_split_right_scroll.clone(),
                                                        )
                                                        .markers(markers.clone())
                                                        .always_visible()
                                                        .render(theme),
                                                    )
                                                })
                                                .when(!self.diff_word_wrap, |d| {
                                                    d.child(Self::render_diff_horizontal_scrollbar(
                                                        theme,
                                                        "diff_split_right_hscrollbar",
                                                        self.diff_split_right_scroll.clone(),
                                                        if vertical_sync_enabled {
                                                            px(0.0)
                                                        } else {
                                                            right_scrollbar_gutter
                                                        },
                                                        "diff_split_right_hscrollbar",
                                                    ))
                                                }),
                                        ),
                                )
                                .when(vertical_sync_enabled, |d| {
                                    d.child(
                                        components::Scrollbar::new(
                                            "diff_scrollbar",
                                            self.diff_scroll.clone(),
                                        )
                                        .markers(markers)
                                        .always_visible()
                                        .render(theme),
                                    )
                                }),
                        )
                        .into_any_element()
                }
            }
        }
    }
}
