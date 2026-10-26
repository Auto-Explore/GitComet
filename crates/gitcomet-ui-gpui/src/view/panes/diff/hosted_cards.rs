use super::*;
use gitcomet_extension_api::{DiffInlineCard, DiffLineSide};
use gitcomet_ui_kit::tooltip::GitCometTooltipExt;
use palette::IntoColor;

const CARD_TEXT_SIZE: f32 = 12.0;
const CARD_PADDING: f32 = 8.0;
const CARD_MARGIN: f32 = 12.0;

#[derive(PartialEq)]
pub(super) struct LayoutKey {
    cards: usize,
    widths: [Pixels; 2],
    font: gpui::Font,
    font_size: Pixels,
    color: gpui::Hsla,
    row_height: Pixels,
}

pub(super) struct CardLayout {
    body: Vec<Arc<gpui::InlineLayout>>,
    row_height: Pixels,
    padding: Pixels,
    stacked_header: bool,
}

impl CardLayout {
    pub(super) fn rows(&self, card: &DiffInlineCard) -> usize {
        self.body_start() + self.body.len() + card.actions.len() + 1
    }

    fn body_start(&self) -> usize {
        2 + usize::from(self.stacked_header)
    }
}

impl DiffRenderer {
    #[cfg(test)]
    pub(in crate::view) fn hosted_card_painted_widths(&self) -> Vec<Pixels> {
        self.hosted_decor
            .as_ref()
            .unwrap()
            .card_layouts
            .iter()
            .flat_map(|card| {
                card.body
                    .iter()
                    .map(|row| row.lines[0].origin.x + row.size.width + card.padding * 2.0)
            })
            .collect()
    }

    fn hosted_card_widths(&self, width: Pixels) -> [Pixels; 2] {
        let left_gutter = self.diff_vertical_scrollbar_gutter_for_column(
            DiffHorizontalScrollColumn::Primary,
            self.diff_scroll.clone(),
        );
        match self.diff_view {
            DiffViewMode::Inline => [width - left_gutter; 2],
            DiffViewMode::Split => {
                let synced = self.diff_scroll_sync.includes_vertical();
                let shared = if synced { left_gutter } else { px(0.0) };
                let (left, right) = crate::view::diff_split_column_widths(
                    (width - shared).max(px(0.0)),
                    self.diff_split_ratio,
                );
                if synced {
                    [left, right]
                } else {
                    let right_gutter = self.diff_vertical_scrollbar_gutter_for_column(
                        DiffHorizontalScrollColumn::SplitRight,
                        self.diff_split_right_scroll.clone(),
                    );
                    [left - left_gutter, right - right_gutter]
                }
            }
        }
    }

    pub(in crate::view) fn prepare_hosted_cards(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let scale = ui_scale::UiScale::current(cx);
        let row_height = self.theme.editor_row_height(scale.percent());
        let Some(decor) = self.hosted_decor.as_mut() else {
            return;
        };
        decor.row_height = row_height;
        if decor.cards.is_empty() && decor.card_layouts.is_empty() {
            return;
        }
        let width = self
            .hosted_content_width
            .unwrap_or(window.viewport_size().width);
        let padding = scale.px(CARD_PADDING);
        let margin = scale.px(CARD_MARGIN);
        let widths = self
            .hosted_card_widths(width)
            .map(|width| (width - (padding + margin) * 2.0 - px(2.0)).max(px(1.0)));
        let decor = self.hosted_decor.as_ref().expect("hosted renderer");
        let font_size = self
            .theme
            .ui_text(CARD_TEXT_SIZE)
            .to_pixels(window.rem_size());
        let mut run = window.text_style().to_run(0);
        run.color = self.theme.colors.foreground.primary.into_color();
        let key = LayoutKey {
            cards: decor.cards.as_ptr() as usize,
            widths,
            font: run.font.clone(),
            font_size,
            color: run.color,
            row_height,
        };
        if decor.card_layout_key.as_ref() == Some(&key) {
            return;
        }
        let layouts = decor
            .cards
            .iter()
            .map(|card| {
                let mut run = run.clone();
                run.len = card.text.len();
                let width = widths[usize::from(card.range.side == DiffLineSide::New)];
                let mut title_run = run.clone();
                title_run.len = card.title.len();
                title_run.font.weight = gpui::FontWeight::SEMIBOLD;
                let title = window
                    .text_system()
                    .shape_text(card.title.clone(), font_size, &[title_run], None, None)
                    .expect("a card title's run covers its UTF-8 source");
                let action_width = (row_height + scale.px(4.0)) * card.header_actions.len() as f32;
                let pending_width = if card.pending {
                    scale.px(16.0)
                } else {
                    px(0.0)
                };
                let stacked_header = !card.header_actions.is_empty()
                    && title.layout.width + action_width + pending_width > width;
                let body = window
                    .text_system()
                    .shape_text(card.text.clone(), font_size, &[run], Some(width), None)
                    .expect("a card's text run covers its UTF-8 source");
                CardLayout {
                    body: paint_rows(&body, row_height),
                    row_height,
                    padding,
                    stacked_header,
                }
            })
            .collect();
        let decor = self.hosted_decor.as_mut().expect("hosted renderer");
        decor.card_layout_key = Some(key);
        decor.card_layouts = layouts;
        decor.key = None;
    }

    pub(super) fn render_hosted_card_row(
        &self,
        card_index: usize,
        row: usize,
        side: Option<DiffLineSide>,
        display: usize,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let decor = self.hosted_decor.as_ref().expect("hosted renderer");
        let card = &decor.cards[card_index];
        let layout = &decor.card_layouts[card_index];
        let pane_id = decor.view_id;
        let id = card.id.clone();
        let row_height = layout.row_height;
        let theme = self.theme;
        let scale = ui_scale::UiScale::current(cx);
        let own_side = side.is_none_or(|side| side == card.range.side);
        let frame = div()
            .id(("hosted_card_row", display))
            .h(row_height)
            .w_full()
            .px(scale.px(CARD_MARGIN))
            .overflow_hidden();
        if !own_side {
            return frame.into_any_element();
        }
        let frame = frame
            .debug_selector(move || format!("hosted_diff_{pane_id}_card_{id}_row_{row}"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let surface = div()
            .w_full()
            .h_full()
            .font_family(
                decor
                    .card_layout_key
                    .as_ref()
                    .expect("prepared card")
                    .font
                    .family
                    .clone(),
            )
            .bg(theme.colors.surface.raised)
            .border_color(theme.colors.stroke.subtle)
            .border_l_1()
            .border_r_1()
            .px(layout.padding)
            .text_size(theme.ui_text(CARD_TEXT_SIZE))
            .text_color(theme.colors.foreground.primary);
        if row == 0 {
            return frame
                .child(
                    surface
                        .mt(row_height / 2.0)
                        .h(row_height / 2.0)
                        .border_t_1()
                        .rounded_t(px(theme.radii.panel)),
                )
                .into_any_element();
        }
        if row + 1 == layout.rows(card) {
            return frame
                .child(
                    surface
                        .h(row_height / 2.0)
                        .border_b_1()
                        .rounded_b(px(theme.radii.panel)),
                )
                .into_any_element();
        }
        let controls = || {
            card.header_actions
                .iter()
                .enumerate()
                .map(|(index, (action, icon))| {
                    let id = format!(
                        "hosted_diff_{pane_id}_card_{}_header_action_{index}",
                        card.id
                    );
                    let action = action.clone();
                    let label = action.label().clone();
                    crate::kit::components::BarButton::new(id.clone(), "")
                        .icon(icon.clone())
                        .into_button(theme, scale)
                        .on_click_handler(theme, scale, move |_, _, cx| action.invoke(cx))
                        .size(row_height)
                        .min_w(row_height)
                        .p_0()
                        .debug_selector(move || id.clone())
                        .gitcomet_tooltip(theme, label)
                })
        };
        if row == 1 {
            let header = surface
                .flex()
                .items_center()
                .gap_1()
                .children(card.pending.then(|| {
                    crate::kit::icons::svg_spinner(
                        format!("hosted_card_{pane_id}_{}_pending", card.id),
                        theme.colors.accent.foreground,
                        ui_scale::UiScale::current(cx).px(12.0),
                    )
                }))
                .child(
                    div()
                        .id(("hosted_card_title", card_index))
                        .debug_selector({
                            let id = card.id.clone();
                            move || format!("hosted_diff_{pane_id}_card_{id}_title")
                        })
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(card.title.clone()),
                )
                .when(!layout.stacked_header, |header| header.children(controls()));
            return frame.child(header).into_any_element();
        }
        if row == 2 && layout.stacked_header {
            return frame
                .child(
                    surface
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .children(controls()),
                )
                .into_any_element();
        }
        if row < layout.body.len() + layout.body_start() {
            let body_row = row - layout.body_start();
            let body = layout.body[body_row].clone();
            return frame
                .child(
                    surface.child(
                        gpui::canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| {
                                let _ = body.paint(bounds.origin, window, cx);
                            },
                        )
                        .w_full()
                        .h(row_height),
                    ),
                )
                .into_any_element();
        }
        let action_index = row - layout.body.len() - layout.body_start();
        let action = card.actions[action_index].clone();
        let id = format!(
            "hosted_diff_{pane_id}_card_{}_action_{action_index}",
            card.id
        );
        frame
            .child(
                surface.flex().items_center().child(
                    crate::kit::components::BarButton::new(id.clone(), action.label().clone())
                        .into_button(theme, ui_scale::UiScale::current(cx))
                        .on_click_handler(theme, ui_scale::UiScale::current(cx), move |_, _, cx| {
                            action.invoke(cx)
                        })
                        .h_full()
                        .py_0()
                        .debug_selector(move || id.clone()),
                ),
            )
            .into_any_element()
    }
}

fn paint_rows(body: &gpui::ShapedText, row_height: Pixels) -> Vec<Arc<gpui::InlineLayout>> {
    let source = &std::ops::Deref::deref(body).layout;
    source
        .visual_lines
        .iter()
        .map(|visual| {
            let fragments = source.paint_fragments[visual.paint_fragment_range.clone()].to_vec();
            let mut line = visual.clone();
            line.paint_fragment_range = 0..fragments.len();
            let layout = gpui::LineLayout {
                font_size: source.font_size,
                width: visual.advance_width,
                ascent: source.ascent,
                descent: source.descent,
                visual_lines: [line].into_iter().collect(),
                paint_fragments: fragments,
                len: source.len,
                platform_layout: source.platform_layout.clone(),
            };
            let size = gpui::size(visual.advance_width, row_height);
            Arc::new(gpui::InlineLayout {
                layout: Arc::new(layout),
                lines: vec![gpui::InlineVisualLine {
                    origin: gpui::point(visual.offset, px(0.0)),
                    size,
                    baseline: (row_height - source.ascent - source.descent) / 2.0 + source.ascent,
                }],
                boxes: Vec::new(),
                alignment_offset: visual.offset,
                size,
            })
        })
        .collect()
}
