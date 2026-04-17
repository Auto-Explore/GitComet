use super::control_height_md;
use crate::kit::{Scrollbar, ScrollbarAxis, TextInput};
use crate::theme::AppTheme;
use crate::ui_scale;
use gpui::prelude::*;
use gpui::{
    ClickEvent, CursorStyle, Div, Entity, FontWeight, ScrollHandle, SharedString, Window, div, px,
};
use std::ops::Range;
use std::sync::Arc;

pub struct PickerPrompt {
    query_input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
    items: Vec<SharedString>,
    empty_text: SharedString,
    max_height: gpui::Pixels,
}

type OnSelectFn<V> =
    dyn Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static;

impl PickerPrompt {
    pub fn new(query_input: Entity<TextInput>, scroll_handle: ScrollHandle) -> Self {
        Self {
            query_input,
            scroll_handle,
            items: Vec::new(),
            empty_text: "No matches".into(),
            max_height: px(360.0),
        }
    }

    pub fn items(mut self, items: impl IntoIterator<Item = SharedString>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    pub fn empty_text(mut self, text: impl Into<SharedString>) -> Self {
        self.empty_text = text.into();
        self
    }

    pub fn max_height(mut self, height: gpui::Pixels) -> Self {
        self.max_height = height;
        self
    }

    #[allow(dead_code)]
    pub fn render<V: 'static>(
        self,
        theme: AppTheme,
        cx: &gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Div {
        self.render_scaled(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT, cx, on_select)
    }

    pub fn render_scaled<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale_percent: u32,
        cx: &gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Div {
        let on_select: Arc<OnSelectFn<V>> = Arc::new(on_select);
        let scroll_handle = self.scroll_handle;
        let scaled_px = |value| ui_scale::design_px_from_percent(value, ui_scale_percent);

        let query = self
            .query_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        let matches = match_items(&self.items, &query);

        let body = div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .flex()
                    .w_full()
                    .min_w(px(0.0))
                    .child(self.query_input.clone()),
            )
            .child(div().border_t_1().border_color(theme.colors.border));

        let mut list = div()
            .id("picker_prompt_list")
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .max_h(self.max_height)
            .track_scroll(&scroll_handle);

        if matches.is_empty() {
            list = list.child(
                div()
                    .h(control_height_md(ui_scale_percent))
                    .w_full()
                    .flex()
                    .items_center()
                    .px(scaled_px(8.0))
                    .text_sm()
                    .line_height(scaled_px(18.0))
                    .text_color(theme.colors.text_muted)
                    .child(self.empty_text),
            );
        } else {
            for m in matches {
                let label = highlighted_label(theme, &self.items[m.index], &query, m.range);
                let on_select = Arc::clone(&on_select);
                let original_index = m.index;
                list = list.child(
                    div()
                        .id(("picker_prompt_item", original_index))
                        .debug_selector(move || format!("picker_prompt_item_{original_index}"))
                        .h(control_height_md(ui_scale_percent))
                        .w_full()
                        .flex()
                        .items_center()
                        .px(scaled_px(8.0))
                        .rounded(px(theme.radii.row))
                        .hover(move |s| s.bg(theme.colors.hover))
                        .active(move |s| s.bg(theme.colors.active))
                        .cursor(CursorStyle::PointingHand)
                        .child(label)
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            (on_select)(this, original_index, event, window, cx);
                        })),
                );
            }
        }

        let scrollbar_gutter =
            Scrollbar::visible_gutter(scroll_handle.clone(), ScrollbarAxis::Vertical);
        let list = list.pr(scrollbar_gutter);
        let scrollbar = {
            let scrollbar = Scrollbar::new("picker_prompt_scrollbar", scroll_handle);
            #[cfg(test)]
            let scrollbar = scrollbar.debug_selector("picker_prompt_scrollbar");
            scrollbar.render(theme)
        };

        body.child(
            div()
                .id("picker_prompt_list_container")
                .relative()
                .w_full()
                .min_w(px(0.0))
                .child(list)
                .child(scrollbar),
        )
    }
}

#[derive(Clone, Debug)]
struct Match {
    index: usize,
    range: Option<Range<usize>>,
    sort_key: (usize, usize, SharedString),
}

fn match_items(items: &[SharedString], query: &str) -> Vec<Match> {
    if query.is_empty() {
        return items
            .iter()
            .enumerate()
            .map(|(index, label)| Match {
                index,
                range: None,
                sort_key: (0, label.len(), label.clone()),
            })
            .collect();
    }

    let mut out = Vec::with_capacity(items.len());
    let needle_bytes = query.as_bytes();
    let first_lower = needle_bytes[0].to_ascii_lowercase();
    let first_upper = needle_bytes[0].to_ascii_uppercase();

    for (index, label) in items.iter().enumerate() {
        let Some(range) = find_ascii_case_insensitive_precomputed(
            label.as_bytes(),
            needle_bytes,
            first_lower,
            first_upper,
        ) else {
            continue;
        };
        let start = range.start;
        out.push(Match {
            index,
            range: Some(range),
            sort_key: (start, label.len(), label.clone()),
        });
    }

    out.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
    out
}

fn highlighted_label(
    theme: AppTheme,
    label: &str,
    query: &str,
    range: Option<Range<usize>>,
) -> Div {
    let base = div()
        .flex()
        .min_w(px(0.0))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_sm();

    let Some(range) = range.filter(|_| !query.is_empty()) else {
        return base.child(label.to_string());
    };

    let prefix = label.get(..range.start).unwrap_or("");
    let hit = label.get(range.clone()).unwrap_or("");
    let suffix = label.get(range.end..).unwrap_or("");

    base.child(prefix.to_string())
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.accent)
                .child(hit.to_string()),
        )
        .child(suffix.to_string())
}

/// Substring search with precomputed first-byte lowercase/uppercase values.
/// Skips positions where the first byte cannot match, avoiding the inner loop
/// overhead for most non-matching positions.
fn find_ascii_case_insensitive_precomputed(
    haystack_bytes: &[u8],
    needle_bytes: &[u8],
    first_lower: u8,
    first_upper: u8,
) -> Option<Range<usize>> {
    if needle_bytes.is_empty() {
        return Some(0..0);
    }
    if haystack_bytes.len() < needle_bytes.len() {
        return None;
    }

    let end = haystack_bytes.len() - needle_bytes.len();
    'outer: for start in 0..=end {
        let first = haystack_bytes[start];
        if first != first_lower && first != first_upper {
            continue;
        }
        for (offset, needle_byte) in needle_bytes.iter().copied().enumerate().skip(1) {
            let haystack_byte = haystack_bytes[start + offset];
            if !haystack_byte.eq_ignore_ascii_case(&needle_byte) {
                continue 'outer;
            }
        }
        return Some(start..(start + needle_bytes.len()));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_items_skips_queries_longer_than_candidate_labels() {
        let items = vec!["ab".into(), "alphabet".into()];

        let matches = match_items(&items, "alphabet soup");

        assert!(matches.is_empty());
    }

    #[test]
    fn ascii_matcher_returns_none_when_needle_is_longer_than_haystack() {
        let needle = b"alphabet soup";

        let range = find_ascii_case_insensitive_precomputed(
            b"ab",
            needle,
            needle[0].to_ascii_lowercase(),
            needle[0].to_ascii_uppercase(),
        );

        assert_eq!(range, None);
    }
}
