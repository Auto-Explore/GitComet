use super::control_height_md;
use crate::kit::{Scrollbar, ScrollbarAxis, TextInput};
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use crate::view::restrict_scroll_to_vertical_axis;
use crate::view::tooltip_host::TooltipHost;
use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, CursorStyle, Div, Entity, FontWeight, HighlightStyle, MouseButton,
    MouseDownEvent, MouseMoveEvent, ScrollHandle, SharedString, UniformListScrollHandle,
    WeakEntity, Window, div, px, uniform_list,
};
use std::ops::Range;
use std::sync::Arc;

use super::{TextTruncationProfile, TruncatedText};

pub struct PickerPrompt {
    query_input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
    items: Vec<PickerPromptItem>,
    empty_text: SharedString,
    max_height: gpui::Pixels,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    selected_index: Option<usize>,
    marked_index: Option<usize>,
    leading_icon: Option<&'static str>,
    selected_hint: Option<SharedString>,
    accent_selection: bool,
    attached_list_surface: bool,
    padded_query_row: bool,
    select_on_mouse_down: bool,
    query_row_trailing: Option<gpui::AnyElement>,
    list_override: Option<gpui::AnyElement>,
    remove_tooltip: Option<SharedString>,
    uniform_scroll: Option<UniformListScrollHandle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerPromptItem {
    display_text: SharedString,
    match_text: SharedString,
    parts: Vec<PickerPromptItemPart>,
    icon: Option<&'static str>,
    repository_initials: Option<SharedString>,
    section: Option<SharedString>,
    removable: bool,
}

/// Where a filtered picker list ends up on screen: which items survived the
/// query, in render order, and which scroll child each one is (section headers
/// take child slots too, so `scroll_to_item` needs the translated index).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PickerPromptLayout {
    pub item_indices: Vec<usize>,
    pub child_indices: Vec<usize>,
}

/// Resolve the display layout the same way [`PickerPrompt::render`] does, so
/// keyboard navigation over a filtered list stays in lockstep with the rows the
/// user sees.
pub fn picker_prompt_layout(items: &[PickerPromptItem], query: &str) -> PickerPromptLayout {
    let matches = match_items(items, &section_groups(items), query);
    let mut layout = PickerPromptLayout {
        item_indices: Vec::with_capacity(matches.len()),
        child_indices: Vec::with_capacity(matches.len()),
    };
    let mut child_ix = 0usize;
    let mut sections = SectionRun::default();
    for m in &matches {
        if sections.starts_new_section(items[m.index].section.as_ref()) {
            child_ix += 1;
        }
        layout.item_indices.push(m.index);
        layout.child_indices.push(child_ix);
        child_ix += 1;
    }
    layout
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerPromptItemPart {
    text: SharedString,
    profile: TextTruncationProfile,
    flexible: bool,
    searchable: bool,
    match_range: Option<Range<usize>>,
}

type OnSelectFn<V> =
    dyn Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static;
type OnRemoveFn<V> = dyn Fn(&mut V, usize, &mut Window, &mut gpui::Context<V>) + 'static;

impl PickerPrompt {
    pub fn new(query_input: Entity<TextInput>, scroll_handle: ScrollHandle) -> Self {
        Self {
            query_input,
            scroll_handle,
            items: Vec::new(),
            empty_text: "No matches".into(),
            max_height: px(360.0),
            tooltip_host: None,
            selected_index: None,
            marked_index: None,
            leading_icon: None,
            selected_hint: None,
            accent_selection: false,
            attached_list_surface: false,
            padded_query_row: false,
            select_on_mouse_down: false,
            query_row_trailing: None,
            list_override: None,
            remove_tooltip: None,
            uniform_scroll: None,
        }
    }

    pub fn items<I, T>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<PickerPromptItem>,
    {
        self.items = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn tooltip_host(mut self, tooltip_host: WeakEntity<TooltipHost>) -> Self {
        self.tooltip_host = Some(tooltip_host);
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

    pub fn selected_index(mut self, ix: Option<usize>) -> Self {
        self.selected_index = ix;
        self
    }

    /// Item (by original index, pre-filter) rendered with a trailing check —
    /// e.g. the currently checked-out branch in the branch picker.
    pub fn marked_index(mut self, ix: Option<usize>) -> Self {
        self.marked_index = ix;
        self
    }

    pub fn leading_icon(mut self, icon: &'static str) -> Self {
        self.leading_icon = Some(icon);
        self
    }

    pub fn selected_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.selected_hint = Some(hint.into());
        self
    }

    pub fn accent_selection(mut self) -> Self {
        self.accent_selection = true;
        self
    }

    pub fn attached_list_surface(mut self) -> Self {
        self.attached_list_surface = true;
        self
    }

    /// Pads and vertically centers the query row (matching the attached-surface
    /// layout) without drawing the surface border. Use when the picker already
    /// sits inside a bordered card (e.g. a popover) but the input is chromeless.
    pub fn padded_query_row(mut self) -> Self {
        self.padded_query_row = true;
        self
    }

    pub fn select_on_mouse_down(mut self) -> Self {
        self.select_on_mouse_down = true;
        self
    }

    /// Control pinned to the right of the query row, e.g. a sort toggle.
    pub fn query_row_trailing(mut self, element: impl IntoElement) -> Self {
        self.query_row_trailing = Some(element.into_any_element());
        self
    }

    /// Replaces the result rows with `element` — for menus that take over the
    /// list area while the query row stays put (the sort menu).
    pub fn list_override(mut self, element: impl IntoElement) -> Self {
        self.list_override = Some(element.into_any_element());
        self
    }

    /// Draws the rows through a virtualized `uniform_list` tracked by `handle`,
    /// building only the ones on screen instead of laying every match out on
    /// every frame. For lists long enough that the cost shows (thousands of
    /// authors, say); rows must be uniform height, so section headers are not
    /// rendered on this path.
    ///
    /// Keyboard navigation has to scroll `handle` rather than the
    /// [`ScrollHandle`] passed to [`Self::new`].
    pub fn virtualized(mut self, handle: UniformListScrollHandle) -> Self {
        self.uniform_scroll = Some(handle);
        self
    }

    /// Tooltip for the trailing remove button on
    /// [`PickerPromptItem::removable`] rows.
    pub fn remove_tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.remove_tooltip = Some(tooltip.into());
        self
    }

    pub fn render<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale: impl Into<UiScale>,
        cx: &gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Div {
        self.render_with_remove(theme, ui_scale, cx, on_select, |_, _, _, _| {})
    }

    /// Like [`Self::render`], but also wires the trailing remove button that
    /// [`PickerPromptItem::removable`] rows carry. `on_remove` receives the
    /// item's original (pre-filter) index, like `on_select`.
    pub fn render_with_remove<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale: impl Into<UiScale>,
        cx: &gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
        on_remove: impl Fn(&mut V, usize, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Div {
        let on_select: Arc<OnSelectFn<V>> = Arc::new(on_select);
        let on_remove: Arc<OnRemoveFn<V>> = Arc::new(on_remove);
        let remove_tooltip = self.remove_tooltip;
        let scroll_handle = self.scroll_handle;
        let leading_icon = self.leading_icon;
        let selected_hint = self.selected_hint;
        let accent_selection = self.accent_selection;
        let attached_list_surface = self.attached_list_surface;
        let padded_query_row = self.padded_query_row;
        let select_on_mouse_down = self.select_on_mouse_down;
        let ui_scale = ui_scale.into();
        let scaled_px = |value| ui_scale.px(value);

        let query = self
            .query_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        let matches = match_items(&self.items, &section_groups(&self.items), &query);

        let selected_index = self.selected_index.and_then(|ix| {
            if matches.is_empty() {
                None
            } else {
                Some(ix.min(matches.len() - 1))
            }
        });

        let body = div()
            .flex()
            .flex_col()
            .w_full()
            .when(attached_list_surface, |surface| {
                surface
                    .border_1()
                    .border_color(theme.colors.border_variant)
                    .rounded(px(theme.radii.control))
                    .bg(theme.colors.surface_bg_elevated)
                    .overflow_hidden()
            })
            .child(
                div()
                    .flex()
                    .w_full()
                    .min_w(px(0.0))
                    .when(attached_list_surface || padded_query_row, |query_row| {
                        query_row
                            .h(control_height_md(ui_scale))
                            .items_center()
                            .px(scaled_px(10.0))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(self.query_input.clone()),
                    )
                    .when_some(self.query_row_trailing, |query_row, trailing| {
                        query_row.child(div().flex_shrink_0().child(trailing))
                    }),
            )
            .child(div().h(px(1.0)).w_full().bg(if attached_list_surface {
                theme.colors.border
            } else {
                theme.colors.border_variant
            }));

        if let Some(list_override) = self.list_override {
            return body.child(div().w_full().min_w(px(0.0)).child(list_override));
        }

        if matches.is_empty() {
            let list = div()
                .id("picker_prompt_list")
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .max_h(self.max_height)
                .track_scroll(&scroll_handle)
                .child(
                    div()
                        .h(control_height_md(ui_scale))
                        .w_full()
                        .flex()
                        .items_center()
                        .px(scaled_px(8.0))
                        .text_sm()
                        .line_height(scaled_px(18.0))
                        .text_color(theme.colors.text_muted)
                        .child(self.empty_text),
                );
            let gutter = Scrollbar::visible_gutter(scroll_handle.clone(), ScrollbarAxis::Vertical);
            let scrollbar = Scrollbar::new("picker_prompt_scrollbar", scroll_handle);
            #[cfg(test)]
            let scrollbar = scrollbar.debug_selector("picker_prompt_scrollbar");
            return body.child(
                div()
                    .id("picker_prompt_list_container")
                    .relative()
                    .w_full()
                    .min_w(px(0.0))
                    .child(restrict_scroll_to_vertical_axis(list).pr(gutter))
                    .child(scrollbar.render(theme)),
            );
        }

        let rows = PickerRows {
            theme,
            ui_scale,
            items: self.items,
            matches,
            selected_index,
            marked_index: self.marked_index,
            leading_icon,
            selected_hint,
            accent_selection,
            select_on_mouse_down,
            remove_tooltip,
            tooltip_host: self.tooltip_host,
            on_select,
            on_remove,
        };

        let (list, scrollbar) = if let Some(uniform_scroll) = self.uniform_scroll {
            let row_count = rows.matches.len();
            // Fit the rows when there are few of them; the list only takes the
            // full height once it has enough rows to fill it.
            let list_height = (control_height_md(ui_scale) * row_count as f32).min(self.max_height);
            let gutter = Scrollbar::visible_gutter(uniform_scroll.clone(), ScrollbarAxis::Vertical);
            let list = uniform_list(
                "picker_prompt_list",
                row_count,
                cx.processor(move |_this, range: Range<usize>, _window, cx| {
                    range
                        .map(|display_ix| rows.row(display_ix, cx).into_any_element())
                        .collect::<Vec<AnyElement>>()
                }),
            )
            .w_full()
            .h(list_height)
            .pr(gutter)
            .track_scroll(&uniform_scroll);
            let scrollbar = Scrollbar::new("picker_prompt_scrollbar", uniform_scroll);
            #[cfg(test)]
            let scrollbar = scrollbar.debug_selector("picker_prompt_scrollbar");
            (
                restrict_scroll_to_vertical_axis(list).into_any_element(),
                scrollbar.render(theme),
            )
        } else {
            let mut list = div()
                .id("picker_prompt_list")
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .max_h(self.max_height)
                .track_scroll(&scroll_handle);
            list = restrict_scroll_to_vertical_axis(list);

            let mut sections = SectionRun::default();
            for display_ix in 0..rows.matches.len() {
                let item = &rows.items[rows.matches[display_ix].index];
                if sections.starts_new_section(item.section.as_ref())
                    && let Some(section) = item.section.clone()
                {
                    list = list.child(section_header_row(
                        theme,
                        ui_scale,
                        section,
                        display_ix == 0,
                    ));
                }
                list = list.child(rows.row(display_ix, cx));
            }

            let gutter = Scrollbar::visible_gutter(scroll_handle.clone(), ScrollbarAxis::Vertical);
            let scrollbar = Scrollbar::new("picker_prompt_scrollbar", scroll_handle);
            #[cfg(test)]
            let scrollbar = scrollbar.debug_selector("picker_prompt_scrollbar");
            (list.pr(gutter).into_any_element(), scrollbar.render(theme))
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

/// Everything a result row is drawn from, kept in one place so the plain and
/// virtualized lists build identical rows.
struct PickerRows<V: 'static> {
    theme: AppTheme,
    ui_scale: UiScale,
    items: Vec<PickerPromptItem>,
    matches: Vec<Match>,
    selected_index: Option<usize>,
    marked_index: Option<usize>,
    leading_icon: Option<&'static str>,
    selected_hint: Option<SharedString>,
    accent_selection: bool,
    select_on_mouse_down: bool,
    remove_tooltip: Option<SharedString>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    on_select: Arc<OnSelectFn<V>>,
    on_remove: Arc<OnRemoveFn<V>>,
}

impl<V: 'static> PickerRows<V> {
    /// `display_ix` indexes the filtered, rendered order; the callbacks and the
    /// marked row use the item's original index.
    fn row(&self, display_ix: usize, cx: &gpui::Context<V>) -> gpui::Stateful<Div> {
        let theme = self.theme;
        let ui_scale = self.ui_scale;
        let scaled_px = |value| ui_scale.px(value);
        let m = &self.matches[display_ix];
        let original_index = m.index;
        let item = &self.items[original_index];

        let label = picker_item_label(theme, item, m.range.clone(), self.tooltip_host.clone(), cx);
        let on_select = Arc::clone(&self.on_select);
        let row_initials = item.repository_initials.clone();
        let row_icon = row_initials
            .is_none()
            .then(|| item.icon.or(self.leading_icon))
            .flatten();
        let is_selected = self.selected_index == Some(display_ix);
        let is_marked = self.marked_index == Some(original_index);
        let is_removable = item.removable;
        let selected_hint = self.selected_hint.clone();
        let accent_selection = self.accent_selection;
        let row_group: SharedString = format!("picker_prompt_row_{original_index}").into();
        let mut row = div()
            .id(("picker_prompt_item", original_index))
            .debug_selector(move || format!("picker_prompt_item_{original_index}"))
            .group(row_group.clone())
            .h(control_height_md(ui_scale))
            .w_full()
            .relative()
            .flex()
            .items_center()
            .gap(scaled_px(7.0))
            .px(scaled_px(8.0))
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .when_some(row_icon, |row, icon| {
                row.child(crate::view::icons::svg_icon(
                    icon,
                    if is_selected {
                        theme.colors.accent
                    } else {
                        theme.colors.text_muted
                    },
                    scaled_px(14.0),
                ))
            })
            .when_some(row_initials, |row, initials| {
                row.child(
                    super::repository_initials_box(
                        theme,
                        ui_scale,
                        initials,
                        is_selected || is_marked,
                    )
                    .debug_selector(move || {
                        format!("picker_prompt_repository_badge_{original_index}")
                    }),
                )
            })
            .child(div().flex_1().min_w(px(0.0)).child(label))
            .when(is_marked, |row| {
                row.child(div().flex_shrink_0().pl(scaled_px(6.0)).child(
                    crate::view::icons::svg_icon(
                        "icons/check.svg",
                        theme.colors.accent,
                        scaled_px(12.0),
                    ),
                ))
            })
            .when(is_selected, |row| {
                row.when_some(selected_hint, |row, hint| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .min_w(scaled_px(34.0))
                            .h(scaled_px(22.0))
                            .px(scaled_px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(scaled_px(4.0))
                            .bg(with_alpha(
                                theme.colors.text,
                                if theme.is_dark { 0.06 } else { 0.035 },
                            ))
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child(hint),
                    )
                })
            })
            .when(is_removable, |row| {
                row.child(remove_row_button(
                    theme,
                    ui_scale,
                    original_index,
                    row_group.clone(),
                    // Keyboard users never hover, so the row the selection sits
                    // on keeps its button visible.
                    is_selected,
                    self.remove_tooltip.clone(),
                    self.tooltip_host.clone(),
                    Arc::clone(&self.on_remove),
                    cx,
                ))
            });
        if self.select_on_mouse_down {
            row = row.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    (on_select)(this, original_index, &ClickEvent::default(), window, cx);
                }),
            );
        } else {
            row = row.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                (on_select)(this, original_index, event, window, cx);
            }));
        }
        // Text-alpha overlays keep the highlight visible on the elevated
        // popover surface, unlike the canvas-tuned tokens.
        let hover_overlay = theme.hover_overlay();
        let active_overlay = theme.active_overlay();
        if is_selected {
            row = row.bg(active_overlay).when(accent_selection, |row| {
                row.rounded_tl(px(0.0)).rounded_bl(px(0.0)).child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(scaled_px(3.0))
                        .rounded_tr(px(theme.radii.row))
                        .rounded_br(px(theme.radii.row))
                        .bg(theme.colors.accent),
                )
            });
        }
        row.hover(move |s| s.bg(hover_overlay))
            .active(move |s| s.bg(active_overlay))
    }
}

impl PickerPromptItem {
    pub fn plain(text: impl Into<SharedString>) -> Self {
        Self::single(text, TextTruncationProfile::End)
    }

    pub fn single(text: impl Into<SharedString>, profile: TextTruncationProfile) -> Self {
        Self::from_parts([PickerPromptItemPart::new(text).profile(profile)])
    }

    pub fn from_parts<I>(parts: I) -> Self
    where
        I: IntoIterator<Item = PickerPromptItemPart>,
    {
        let mut display_text = String::new();
        let mut match_text = String::new();
        let mut built_parts = Vec::new();

        for mut part in parts {
            display_text.push_str(part.text.as_ref());

            if part.searchable {
                let start = match_text.len();
                match_text.push_str(part.text.as_ref());
                part.match_range = Some(start..match_text.len());
            }

            built_parts.push(part);
        }

        Self {
            display_text: display_text.into(),
            match_text: match_text.into(),
            parts: built_parts,
            icon: None,
            repository_initials: None,
            section: None,
            removable: false,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Uses the shared repository initials box in the row's leading slot.
    /// This takes precedence over both item and picker-level SVG icons.
    pub fn repository_initials(mut self, repository_name: &str) -> Self {
        self.repository_initials = Some(super::repository_initials(repository_name).into());
        self
    }

    /// Gives the row a trailing `x` that drops the entry from the list instead
    /// of activating it. Requires [`PickerPrompt::render_with_remove`].
    pub fn removable(mut self) -> Self {
        self.removable = true;
        self
    }

    /// Groups the item under a labelled section header. Items sharing a label
    /// must be contiguous in the list passed to [`PickerPrompt::items`].
    pub fn section(mut self, section: impl Into<SharedString>) -> Self {
        self.section = Some(section.into());
        self
    }

    fn display_text(&self) -> &str {
        self.display_text.as_ref()
    }

    fn match_text(&self) -> &str {
        self.match_text.as_ref()
    }

    fn parts(&self) -> &[PickerPromptItemPart] {
        self.parts.as_slice()
    }
}

impl PickerPromptItemPart {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            profile: TextTruncationProfile::End,
            flexible: true,
            searchable: true,
            match_range: None,
        }
    }

    pub fn separator(text: impl Into<SharedString>) -> Self {
        Self::new(text).flexible(false).searchable(false)
    }

    pub fn path(text: impl Into<SharedString>) -> Self {
        Self::new(text).profile(TextTruncationProfile::Path)
    }

    pub fn profile(mut self, profile: TextTruncationProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn flexible(mut self, flexible: bool) -> Self {
        self.flexible = flexible;
        self
    }

    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        if !searchable {
            self.match_range = None;
        }
        self
    }

    fn local_match_range(&self, range: Option<&Range<usize>>) -> Option<Range<usize>> {
        let range = range?;
        let part_range = self.match_range.as_ref()?;
        let start = range.start.max(part_range.start);
        let end = range.end.min(part_range.end);
        (start < end).then(|| (start - part_range.start)..(end - part_range.start))
    }
}

impl From<SharedString> for PickerPromptItem {
    fn from(value: SharedString) -> Self {
        Self::plain(value)
    }
}

impl From<String> for PickerPromptItem {
    fn from(value: String) -> Self {
        Self::plain(value)
    }
}

impl From<&str> for PickerPromptItem {
    fn from(value: &str) -> Self {
        Self::plain(value.to_owned())
    }
}

#[derive(Clone, Debug)]
struct Match {
    index: usize,
    range: Option<Range<usize>>,
    sort_key: (usize, usize, usize, SharedString),
}

/// Tracks the section of the previously emitted row so a header is rendered
/// exactly once per contiguous run of items sharing a section label.
#[derive(Default)]
struct SectionRun<'a> {
    previous: Option<Option<&'a SharedString>>,
}

impl<'a> SectionRun<'a> {
    /// True when `section` opens a labelled run that needs a header row.
    fn starts_new_section(&mut self, section: Option<&'a SharedString>) -> bool {
        let changed = self.previous != Some(section);
        self.previous = Some(section);
        changed && section.is_some()
    }
}

/// Numbers each contiguous run of items sharing a section label. Matches sort
/// within their group, so filtering never interleaves sections.
fn section_groups(items: &[PickerPromptItem]) -> Vec<usize> {
    let mut groups = Vec::with_capacity(items.len());
    let mut group = 0usize;
    let mut previous: Option<&SharedString> = None;
    for (ix, item) in items.iter().enumerate() {
        if ix > 0 && item.section.as_ref() != previous {
            group += 1;
        }
        previous = item.section.as_ref();
        groups.push(group);
    }
    groups
}

fn match_items(items: &[PickerPromptItem], groups: &[usize], query: &str) -> Vec<Match> {
    let group_of = |index: usize| groups.get(index).copied().unwrap_or(0);

    if query.is_empty() {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| Match {
                index,
                range: None,
                sort_key: (
                    group_of(index),
                    0,
                    item.display_text().len(),
                    item.display_text.clone(),
                ),
            })
            .collect();
    }

    let mut out = Vec::with_capacity(items.len());
    let needle_bytes = query.as_bytes();
    let first_lower = needle_bytes[0].to_ascii_lowercase();
    let first_upper = needle_bytes[0].to_ascii_uppercase();

    for (index, item) in items.iter().enumerate() {
        let match_text = item.match_text();
        if match_text.is_empty() {
            continue;
        }

        let Some(range) = find_ascii_case_insensitive_precomputed(
            match_text.as_bytes(),
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
            sort_key: (
                group_of(index),
                start,
                item.display_text().len(),
                item.display_text.clone(),
            ),
        });
    }

    out.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
    out
}

fn section_header_row(
    theme: AppTheme,
    ui_scale: UiScale,
    label: SharedString,
    is_first: bool,
) -> Div {
    let scaled_px = |value| ui_scale.px(value);
    div()
        .w_full()
        .flex_shrink_0()
        .px(scaled_px(8.0))
        .pt(scaled_px(if is_first { 4.0 } else { 10.0 }))
        .pb(scaled_px(4.0))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .line_height(scaled_px(16.0))
        .text_color(theme.colors.text_muted)
        .whitespace_nowrap()
        .overflow_hidden()
        .child(label)
}

fn picker_item_label<V: 'static>(
    theme: AppTheme,
    item: &PickerPromptItem,
    range: Option<Range<usize>>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let match_highlight = HighlightStyle {
        color: Some(theme.colors.accent.into()),
        font_weight: Some(FontWeight::BOLD),
        ..HighlightStyle::default()
    };

    let mut label = div()
        .flex()
        .w_full()
        .min_w(px(0.0))
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_sm();

    for (ix, part) in item.parts().iter().enumerate() {
        let highlight_range = part.local_match_range(range.as_ref());
        let mut container = div().min_w(px(0.0)).overflow_hidden().whitespace_nowrap();
        if part.flexible {
            container = container.flex_1();
        } else if part.searchable {
            container.style().flex_shrink = Some(1.0);
        } else {
            container = container.flex_shrink_0();
        }

        let mut text = TruncatedText::new(part.text.clone())
            .id(("picker_prompt_label_part_text", ix))
            .profile(part.profile)
            .text_color(theme.colors.text);
        if let Some(highlight_range) = highlight_range.clone() {
            text = text
                .focus_range(Some(highlight_range.clone()))
                .highlights([(highlight_range, match_highlight)]);
        }
        if let Some(tooltip_host) = tooltip_host.clone() {
            text = text.full_text_tooltip(tooltip_host);
        }

        label = label.child(
            container
                .id(("picker_prompt_label_part", ix))
                .child(text.render(cx)),
        );
    }

    label
}

/// The trailing `x` on a removable row — drops the entry from the list the
/// picker draws from, rather than activating it. Mirrors the repository tab's
/// close affordance: hidden until the row is hovered (or carries the keyboard
/// selection) and tinted with the danger colour.
#[allow(clippy::too_many_arguments)]
fn remove_row_button<V: 'static>(
    theme: AppTheme,
    ui_scale: UiScale,
    index: usize,
    row_group: SharedString,
    always_visible: bool,
    tooltip: Option<SharedString>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    on_remove: Arc<OnRemoveFn<V>>,
    cx: &gpui::Context<V>,
) -> impl IntoElement {
    let scaled_px = |value| ui_scale.px(value);
    let tooltip_for_move = tooltip.clone();
    let host_for_move = tooltip_host.clone();
    let host_for_hover = tooltip_host;

    div()
        .id(("picker_prompt_item_remove", index))
        .debug_selector(move || format!("picker_prompt_item_remove_{index}"))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(scaled_px(18.0))
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .when(!always_visible, |button| {
            button
                .invisible()
                .group_hover(row_group, |style| style.visible())
        })
        .hover(move |s| s.bg(with_alpha(theme.colors.danger, 0.18)))
        .active(move |s| s.bg(with_alpha(theme.colors.danger, 0.26)))
        .child(crate::view::icons::svg_icon(
            "icons/repo_tab_close.svg",
            theme.colors.danger,
            scaled_px(12.0),
        ))
        .on_mouse_move(cx.listener(move |_this, event: &MouseMoveEvent, _w, cx| {
            let (Some(host), Some(tooltip)) = (host_for_move.as_ref(), tooltip_for_move.as_ref())
            else {
                return;
            };
            let _ = host.update(cx, |host, cx| {
                host.on_mouse_moved(event.position, cx);
                host.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
            });
        }))
        .on_hover(cx.listener(move |_this, hovering: &bool, _w, cx| {
            let (false, Some(host), Some(tooltip)) =
                (*hovering, host_for_hover.as_ref(), tooltip.as_ref())
            else {
                return;
            };
            let _ = host.update(cx, |host, cx| {
                host.clear_tooltip_if_matches(tooltip, cx);
            });
        }))
        // Keeps the press off the row, which may activate on mouse-down.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _event: &MouseDownEvent, _w, cx| cx.stop_propagation()),
        )
        .on_click(cx.listener(move |this, _event: &ClickEvent, window, cx| {
            cx.stop_propagation();
            (on_remove)(this, index, window, cx);
        }))
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
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
        let items = vec![
            PickerPromptItem::plain("ab"),
            PickerPromptItem::plain("alphabet"),
        ];

        let matches = match_items(&items, &section_groups(&items), "alphabet soup");

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

    #[test]
    fn picker_prompt_item_maps_search_hits_into_part_local_ranges() {
        let item = PickerPromptItem::from_parts([
            PickerPromptItemPart::new("feature/worktree").flexible(false),
            PickerPromptItemPart::separator("  "),
            PickerPromptItemPart::path("/tmp/repo/src/main.rs"),
        ]);

        let matches = match_items(std::slice::from_ref(&item), &[0], "main");
        let range = matches
            .first()
            .and_then(|m| m.range.clone())
            .expect("expected a match");

        assert_eq!(item.parts()[0].local_match_range(Some(&range)), None);
        assert_eq!(item.parts()[1].local_match_range(Some(&range)), None);
        assert_eq!(
            item.parts()[2].local_match_range(Some(&range)),
            Some(14..18)
        );
    }

    #[test]
    fn picker_prompt_layout_reserves_a_child_slot_per_section_header() {
        let items = vec![
            PickerPromptItem::plain("alpha").section("Open"),
            PickerPromptItem::plain("beta").section("Open"),
            PickerPromptItem::plain("gamma").section("Recently Closed"),
        ];

        let layout = picker_prompt_layout(&items, "");

        assert_eq!(layout.item_indices, vec![0, 1, 2]);
        // Header, alpha, beta, header, gamma.
        assert_eq!(layout.child_indices, vec![1, 2, 4]);
    }

    #[test]
    fn picker_prompt_layout_keeps_sections_contiguous_when_filtering() {
        let items = vec![
            PickerPromptItem::plain("zulu-repo").section("Open"),
            PickerPromptItem::plain("repo-one").section("Recently Closed"),
            PickerPromptItem::plain("repo-two").section("Recently Closed"),
        ];

        let layout = picker_prompt_layout(&items, "repo");

        // The "Recently Closed" hits sort earlier on match position, but must
        // not be hoisted above the "Open" section.
        assert_eq!(layout.item_indices, vec![0, 1, 2]);
        assert_eq!(layout.child_indices, vec![1, 3, 4]);
    }

    #[test]
    fn picker_prompt_layout_drops_headers_for_sections_without_matches() {
        let items = vec![
            PickerPromptItem::plain("alpha").section("Open"),
            PickerPromptItem::plain("gamma").section("Recently Closed"),
        ];

        let layout = picker_prompt_layout(&items, "gam");

        assert_eq!(layout.item_indices, vec![1]);
        assert_eq!(layout.child_indices, vec![1]);
    }

    #[test]
    fn picker_prompt_item_search_skips_non_searchable_separators() {
        let item = PickerPromptItem::from_parts([
            PickerPromptItemPart::new("repo").flexible(false),
            PickerPromptItemPart::separator(" - "),
            PickerPromptItemPart::path("/tmp/workspace"),
        ]);

        let matches = match_items(&[item], &[0], " - ");

        assert!(matches.is_empty());
    }
}
