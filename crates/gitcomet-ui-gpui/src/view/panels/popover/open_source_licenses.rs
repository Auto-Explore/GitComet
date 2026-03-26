use super::*;

pub(super) fn panel(this: &mut PopoverHost, _cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let rows = crate::view::open_source_licenses_data::open_source_license_rows();

    let mut rows_content = div()
        .id("open_source_licenses_list")
        .flex()
        .flex_col()
        .pb_2()
        .gap_1();

    if rows.is_empty() {
        rows_content = rows_content.child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child("No dependency licenses found."),
        );
    } else {
        for (ix, row) in rows.iter().enumerate() {
            rows_content = rows_content.child(
                div()
                    .id(("open_source_license_row", ix))
                    .px_2()
                    .py_1()
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .rounded(px(theme.radii.row))
                    .hover(move |s| s.bg(theme.colors.hover))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(240.0))
                                    .text_sm()
                                    .line_clamp(1)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(row.crate_name),
                            )
                            .child(
                                div()
                                    .w(px(100.0))
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(theme.colors.text_muted)
                                    .whitespace_nowrap()
                                    .child(row.version),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(theme.colors.text_muted)
                                    .line_clamp(1)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(row.license),
                            ),
                    ),
            );
        }
    }

    let scrollbar_gutter = components::Scrollbar::visible_gutter(
        this.open_source_licenses_scroll.clone(),
        components::ScrollbarAxis::Vertical,
    );
    let rows_scroll_surface = div()
        .id("open_source_licenses_scroll_surface")
        .relative()
        .w_full()
        .max_h(px(420.0))
        .pr(scrollbar_gutter)
        .overflow_y_scroll()
        .track_scroll(&this.open_source_licenses_scroll)
        .child(rows_content);
    let rows_scrollbar = components::Scrollbar::new(
        "open_source_licenses_scrollbar",
        this.open_source_licenses_scroll.clone(),
    )
    .render(theme);

    let content = div()
        .flex()
        .flex_col()
        .min_w(px(760.0))
        .max_w(px(1020.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Open Source Licenses"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(format!("{} third-party crates listed", rows.len())),
        )
        .child(
            div()
                .id("open_source_licenses_columns")
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(240.0)).child("Crate"))
                .child(div().w(px(100.0)).child("Version"))
                .child(div().flex_1().min_w(px(0.0)).child("License")),
        )
        .child(
            div()
                .relative()
                .child(rows_scroll_surface)
                .child(rows_scrollbar),
        );

    components::context_menu(theme, content)
}
