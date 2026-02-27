use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let current_format = this.date_time_format;
    let current_timezone = this.timezone;
    let preview_now = std::time::SystemTime::now();

    let row = |id: &'static str, label: &'static str, value: SharedString, open: bool| {
        div()
            .id(id)
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(theme.radii.row))
            .hover(move |s| s.bg(theme.colors.hover))
            .active(move |s| s.bg(theme.colors.active))
            .cursor(CursorStyle::PointingHand)
            .child(div().text_sm().child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(value)
                    .child(
                        div()
                            .font_family("monospace")
                            .child(if open { "▴" } else { "▾" }),
                    ),
            )
    };

    // --- Date format dropdown ---
    let mut date_dropdown = div().flex().flex_col().gap_1().px_2().pb_2();

    if this.settings_date_format_open {
        for fmt in DateTimeFormat::all() {
            let selected = *fmt == current_format;
            let fmt_val = *fmt;
            let preview: SharedString =
                format_datetime(preview_now, fmt_val, current_timezone).into();
            date_dropdown = date_dropdown.child(
                div()
                    .id(("settings_date_format_item", *fmt as usize))
                    .px_2()
                    .py_1()
                    .rounded(px(theme.radii.row))
                    .when(!selected, |d| {
                        d.hover(move |s| s.bg(theme.colors.hover))
                            .active(move |s| s.bg(theme.colors.active))
                    })
                    .when(selected, |d| d.bg(with_alpha(theme.colors.accent, 0.15)))
                    .cursor(CursorStyle::PointingHand)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(div().text_sm().child(fmt.label()))
                            .child(
                                div()
                                    .font_family("monospace")
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child(preview),
                            ),
                    )
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.settings_date_format_open = false;
                        this.set_date_time_format(fmt_val, cx);
                        cx.notify();
                    })),
            );
        }
    }

    // --- Timezone dropdown ---
    let mut tz_dropdown = div().flex().flex_col().gap_1().px_2().pb_2();

    if this.settings_timezone_open {
        for tz in Timezone::all() {
            let selected = *tz == current_timezone;
            let tz_val = *tz;
            let preview: SharedString =
                format_datetime(preview_now, current_format, tz_val).into();
            tz_dropdown = tz_dropdown.child(
                div()
                    .id(SharedString::from(format!(
                        "settings_tz_item_{}",
                        tz.offset_seconds()
                    )))
                    .px_2()
                    .py_1()
                    .rounded(px(theme.radii.row))
                    .when(!selected, |d| {
                        d.hover(move |s| s.bg(theme.colors.hover))
                            .active(move |s| s.bg(theme.colors.active))
                    })
                    .when(selected, |d| d.bg(with_alpha(theme.colors.accent, 0.15)))
                    .cursor(CursorStyle::PointingHand)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().text_sm().child(tz.label()))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.colors.text_muted)
                                            .child(tz.cities()),
                                    ),
                            )
                            .child(
                                div()
                                    .font_family("monospace")
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child(preview),
                            ),
                    )
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.settings_timezone_open = false;
                        this.set_timezone(tz_val, cx);
                        cx.notify();
                    })),
            );
        }
    }

    let header = div()
        .px_2()
        .py_1()
        .text_sm()
        .font_weight(FontWeight::BOLD)
        .child("Settings");

    let section_label = div()
        .px_2()
        .pt(px(6.0))
        .pb(px(4.0))
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child("General");

    let date_row = row(
        "settings_date_format",
        "Date format",
        current_format.label().into(),
        this.settings_date_format_open,
    )
    .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
        this.settings_date_format_open = !this.settings_date_format_open;
        this.settings_timezone_open = false;
        cx.notify();
    }));

    let tz_row = row(
        "settings_timezone",
        "Date timezone",
        current_timezone.label().into(),
        this.settings_timezone_open,
    )
    .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
        this.settings_timezone_open = !this.settings_timezone_open;
        this.settings_date_format_open = false;
        cx.notify();
    }));

    let mut content = div()
        .flex()
        .flex_col()
        .min_w(px(560.0))
        .max_w(px(720.0))
        .child(header)
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(section_label)
        .child(
            div()
                .px_2()
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(date_row)
                .child(tz_row),
        );

    if this.settings_date_format_open {
        content = content
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child("Choose a format:"),
            )
            .child(date_dropdown);
    }

    if this.settings_timezone_open {
        content = content
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child("Choose a timezone:"),
            )
            .child(tz_dropdown);
    }

    zed::context_menu(theme, content)
}
