//! Changed-file list chrome: filter tabs, layout and sort controls, and the
//! committed-files section.

use super::*;
use crate::kit::click::PointerClickExt as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommitFileFilterLabels {
    Full,
    Compact,
}

const COMMIT_FILE_FILTER_TEXT_WIDTH_PX: f32 = 5.2;
const COMMIT_FILE_FILTER_ICON_WIDTH_PX: f32 = 12.0;
const COMMIT_FILE_FILTER_ICON_GAP_PX: f32 = 3.0;
/// Side padding inside a committed-files filter chip. Compact keeps all five
/// in a narrow details pane; Comfortable can afford room.
const COMMIT_FILE_FILTER_TAB_PAD_X_PX: f32 = 2.0;
const COMMIT_FILE_FILTER_TAB_COMFORTABLE_PAD_X_PX: f32 = 8.0;

pub(super) fn commit_file_filter_tab_pad_x(metrics: crate::appearance::Appearance) -> f32 {
    metrics.ramp(
        COMMIT_FILE_FILTER_TAB_PAD_X_PX,
        COMMIT_FILE_FILTER_TAB_COMFORTABLE_PAD_X_PX,
    )
}
const COMMIT_FILE_FILTER_TAB_COMPACT_GAP_PX: f32 = 4.0;
const COMMIT_FILE_FILTER_TAB_FULL_GAP_PX: f32 = 6.0;

pub(super) fn commit_file_filter_labels_for_width(
    available_width: Pixels,
    counts: crate::view::rows::CommitFileKindCounts,
    ui_scale_percent: u32,
    metrics: crate::appearance::Appearance,
) -> CommitFileFilterLabels {
    if available_width <= px(0.0) {
        return CommitFileFilterLabels::Full;
    }

    let filters = crate::view::rows::CommitFileFilter::ALL;
    let text_chars = filters
        .into_iter()
        .map(|filter| {
            filter.label().chars().count()
                + counts.for_filter(filter).to_string().chars().count()
                + 3 // space and parentheses
        })
        .sum::<usize>();
    let count = filters.len() as f32;
    let needed = text_chars as f32 * metrics.ui_text(COMMIT_FILE_FILTER_TEXT_WIDTH_PX)
        + count
            * (COMMIT_FILE_FILTER_ICON_WIDTH_PX
                + COMMIT_FILE_FILTER_ICON_GAP_PX
                + 2.0 * commit_file_filter_tab_pad_x(metrics))
        + (count - 1.0) * COMMIT_FILE_FILTER_TAB_FULL_GAP_PX;

    if crate::ui_scale::design_px_from_percent(needed, ui_scale_percent) <= available_width {
        CommitFileFilterLabels::Full
    } else {
        CommitFileFilterLabels::Compact
    }
}

fn commit_file_filter_color(
    filter: crate::view::rows::CommitFileFilter,
    theme: AppTheme,
) -> gpui::Rgba {
    match filter {
        crate::view::rows::CommitFileFilter::All => theme.colors.foreground.secondary,
        crate::view::rows::CommitFileFilter::Modified => {
            crate::view::rows::commit_file_kind_visuals(FileStatusKind::Modified).color(&theme)
        }
        crate::view::rows::CommitFileFilter::Removed => {
            crate::view::rows::commit_file_kind_visuals(FileStatusKind::Deleted).color(&theme)
        }
        crate::view::rows::CommitFileFilter::Added => {
            crate::view::rows::commit_file_kind_visuals(FileStatusKind::Added).color(&theme)
        }
        crate::view::rows::CommitFileFilter::Renamed => {
            crate::view::rows::commit_file_kind_visuals(FileStatusKind::Renamed).color(&theme)
        }
    }
}

impl DetailsPaneView {
    pub(super) fn commit_file_filter_tabs(
        &mut self,
        list: crate::view::rows::FileListId,
        id_prefix: &'static str,
        available_width: Pixels,
        counts: crate::view::rows::CommitFileKindCounts,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        // Each caller supplies its measured filter width. Keep it explicit so
        // worktree and range controls do not read the commit list's bounds.
        let labels = commit_file_filter_labels_for_width(
            available_width,
            counts,
            self.ui_scale_percent,
            theme.metrics,
        );
        let tab_gap = match labels {
            CommitFileFilterLabels::Full => COMMIT_FILE_FILTER_TAB_FULL_GAP_PX,
            CommitFileFilterLabels::Compact => COMMIT_FILE_FILTER_TAB_COMPACT_GAP_PX,
        };
        let current = self.file_list_filter_for(list);

        let mut tabs = div()
            .id(SharedString::from(format!("{id_prefix}_filter_tabs")))
            .debug_selector(move || format!("{id_prefix}_filter_tabs"))
            .flex()
            .items_center()
            .gap(ui_scale.px(tab_gap))
            .w_full()
            .min_w(px(0.0))
            .h(components::control_height(ui_scale))
            .overflow_hidden();

        for (ix, filter) in crate::view::rows::CommitFileFilter::ALL
            .into_iter()
            .enumerate()
        {
            let count = counts.for_filter(filter);
            let selected = current == filter;
            let disabled = count == 0;
            let full_label = format!("{} ({count})", filter.label());
            let tooltip = filter.tooltip_in(list.filter_scope(), count);
            let display_label = match labels {
                CommitFileFilterLabels::Full => full_label.clone(),
                CommitFileFilterLabels::Compact => count.to_string(),
            };
            let icon_color = commit_file_filter_color(filter, theme);
            let selected_border = if theme.is_dark {
                gpui::rgba(0x00000000)
            } else {
                theme.colors.interaction.selected_indicator
            };
            let tab = div()
                .id((SharedString::from(format!("{id_prefix}_filter_tab")), ix))
                .debug_selector(move || format!("{id_prefix}_filter_tab_{ix}"))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(ui_scale.px(COMMIT_FILE_FILTER_ICON_GAP_PX))
                .h(components::control_height(ui_scale))
                .px(ui_scale.px(commit_file_filter_tab_pad_x(theme.metrics)))
                .rounded(px(theme.radii.control))
                .border_1()
                .border_color(if selected {
                    selected_border
                } else {
                    gpui::rgba(0x00000000)
                })
                .text_size(theme.ui_text(12.0))
                .whitespace_nowrap()
                .text_color(if selected {
                    theme.colors.interaction.selected_foreground
                } else {
                    theme.colors.foreground.secondary
                })
                .tab_index(0)
                .control_interaction(
                    InteractionStyle::new(theme)
                        .selection_outline(false)
                        .disabled_opacity(0.5),
                    InteractionState::default()
                        .selected(selected, theme.colors.interaction.selected_background)
                        .disabled(disabled),
                )
                .child(svg_icon(
                    filter.icon(),
                    icon_color,
                    ui_scale.px(COMMIT_FILE_FILTER_ICON_WIDTH_PX),
                ))
                .child(display_label)
                .gitcomet_tooltip(theme, tooltip.into());

            let tab = tab.on_activate(
                disabled,
                components::ControlActivation::Action,
                cx.listener(move |this, event: &ClickEvent, _window, cx| {
                    if event.standard_click() {
                        this.set_file_list_filter(list, filter, cx);
                    }
                }),
            );

            tabs = tabs.child(tab);
        }
        tabs
    }

    /// Layout toggle + sort menu, the pair every changed-file list carries.
    pub(super) fn file_list_controls(
        &mut self,
        list: crate::view::rows::FileListId,
        repo_id: RepoId,
        id_prefix: &'static str,
        disabled: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let sort = self.file_list_sort_for(list);
        let layout = self.file_list_layout_for(repo_id, list);
        let icon_color = theme.colors.foreground.secondary;

        // One icon in the layout's shape: a click steps to the next layout,
        // a right click offers them all.
        let layout_invoker: SharedString = format!("{id_prefix}_layout_button").into();
        let layout_open = self.active_context_menu_invoker.as_ref() == Some(&layout_invoker);
        let layout_button = components::list_layout_button(
            format!("{id_prefix}_layout_button"),
            layout,
            theme,
            ui_scale,
        )
        .disabled(disabled)
        .open(layout_open)
        .on_click(theme, cx, move |this, event, _window, cx| {
            if !event.standard_click() {
                return;
            }
            this.toggle_file_list_layout(repo_id, list, cx);
        })
        .on_pointer_click(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if disabled {
                    return;
                }
                this.open_popover_at(
                    PopoverKind::FileListLayoutMenu { repo_id, list }
                        .invoked_by(layout_invoker.clone()),
                    e.position,
                    window,
                    cx,
                );
                cx.notify();
            }),
        )
        .debug_selector(move || format!("{id_prefix}_layout_button"))
        .gitcomet_tooltip(theme, layout.tooltip());

        let sort_button = components::Button::new(format!("{id_prefix}_sort_button"), "")
            .style(components::ButtonStyle::Transparent)
            .disabled(disabled)
            .start_slot(svg_icon("icons/sort.svg", icon_color, ui_scale.px(14.0)))
            .on_click_with_bounds(theme, cx, move |this, event, bounds, window, cx| {
                if !event.standard_click() {
                    return;
                }
                this.open_popover_for_bounds(
                    PopoverKind::CommitFileSortMenu { list },
                    bounds,
                    window,
                    cx,
                );
            })
            .debug_selector(move || format!("{id_prefix}_sort_button"))
            .gitcomet_tooltip(theme, format!("Sort: {}", sort.label()).into());

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(layout_button)
            .child(sort_button)
            .into_any_element()
    }

    pub(super) fn commit_files_section(
        &mut self,
        repo_id: RepoId,
        commit_details_rev: u64,
        details: &gitcomet_core::domain::CommitDetails,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let projection =
            self.cached_commit_file_projection(repo_id, commit_details_rev, &details.files);
        let plan = self.cached_commit_file_plan(repo_id, commit_details_rev, &details.files);
        let visible_count = projection.source_indices.len();
        let row_count = plan.row_len();
        let files = if details.files.is_empty() {
            div()
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child("No files.")
                .into_any_element()
        } else if visible_count == 0 {
            div()
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child("No files match this filter.")
                .into_any_element()
        } else {
            Self::vertical_scroll_frame_content(
                theme,
                ("commit_details_files_container", repo_id.0),
                ("commit_details_files_scrollbar", repo_id.0),
                &self.commit_files_scroll,
                self.changed_file_list(
                    repo_id,
                    crate::view::rows::FileListId::CommitFiles,
                    row_count,
                    cx,
                ),
            )
            .min_h(crate::view::rows::sidebar::sidebar_list_row_height(
                theme,
                ui_scale.percent(),
            ))
            .into_any_element()
        };

        let controls = self.file_list_controls(
            crate::view::rows::FileListId::CommitFiles,
            repo_id,
            "commit_file",
            details.files.is_empty(),
            cx,
        );

        let heading = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .w_full()
            .min_w(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .line_clamp(1)
                    .child(format!("Committed files ({})", projection.counts.all)),
            )
            .child(controls);
        let filters_width = self
            .commit_files_section_bounds_ref
            .borrow()
            .as_ref()
            .map(|bounds| bounds.size.width)
            .unwrap_or(Pixels::MAX);
        let filters = self.commit_file_filter_tabs(
            crate::view::rows::FileListId::CommitFiles,
            "commit_file",
            filters_width,
            projection.counts,
            cx,
        );

        let section_bounds = std::rc::Rc::clone(&self.commit_files_section_bounds_ref);
        div()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .flex_1()
            .h_full()
            .min_h(ui_scale.px(90.0))
            .border_t_1()
            .border_color(theme.colors.stroke.default)
            .pt_2()
            .on_children_prepainted(move |children_bounds, window, _app| {
                let next_bounds = children_bounds.first().copied();
                let mut measured = section_bounds.borrow_mut();
                if *measured != next_bounds {
                    *measured = next_bounds;
                    window.refresh();
                }
            })
            .child(visible_bounds_probe())
            .child(heading)
            .child(filters)
            .child(files)
            .into_any_element()
    }
}
