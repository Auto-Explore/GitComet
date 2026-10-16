use super::*;

struct HeadingBlock {
    theme: AppTheme,
}

impl gpui::Render for HeadingBlock {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(
                context_menu_header(self.theme, 100u32, "primary.rs", None, cx)
                    .debug_selector(|| "heading_primary".to_string()),
            )
            .child(
                context_menu_label(
                    self.theme,
                    100u32,
                    ContextMenuText::path_single_line("/tmp/some/where/primary.rs"),
                    None,
                    cx,
                )
                .debug_selector(|| "heading_secondary".to_string()),
            )
    }
}

/// A menu naming an object renders two headings, and the second one has to
/// read as subordinate. Both lines go through `TruncatedText`, which takes
/// its size from the element rather than the wrapping div's text class — so
/// dropping the explicit size silently returns both lines to the window
/// default and they render identically, which is the bug this guards.
///
/// Glyph metrics are useless here (the test text system measures every glyph
/// the same), but the line box each heading reserves still follows its font
/// size, so height is the observable that separates them.
#[gpui::test]
fn secondary_heading_renders_smaller_than_the_primary(cx: &mut gpui::TestAppContext) {
    let theme = crate::theme::AppTheme::gitcomet_dark();
    let (_view, cx) = cx.add_window_view(|_window, _cx| HeadingBlock { theme });
    crate::test_support::redraw(cx);

    // The primary heading's outer box is a row, so measure the text box
    // inside it; the secondary heading is the text box itself.
    let primary = cx
        .debug_bounds("context_menu_header_text")
        .expect("expected the primary heading");
    let secondary = cx
        .debug_bounds("heading_secondary")
        .expect("expected the secondary heading");

    let primary_line = primary.size.height;
    let secondary_line = secondary.size.height - px(4.0);
    assert!(
        secondary_line < primary_line,
        "secondary heading must render smaller than the primary, got \
         {secondary_line:?} vs {primary_line:?}"
    );
}

struct RowKinds {
    theme: AppTheme,
}

impl gpui::Render for RowKinds {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let scale = crate::ui_scale::UiScale::current(cx);
        div()
            .flex()
            .flex_col()
            .child(
                ContextMenuEntry::new("plain", SharedString::from("Plain"))
                    .icon(ContextMenuIconSlot::Reserved)
                    .render(theme, scale, cx)
                    .debug_selector(|| "row_plain".to_string()),
            )
            .child(
                ContextMenuEntry::new("text", SharedString::from("Text shortcut"))
                    .icon(ContextMenuIconSlot::Reserved)
                    .shortcut(Some("A".into()))
                    .render(theme, scale, cx)
                    .debug_selector(|| "row_text_shortcut".to_string()),
            )
            .child(
                ContextMenuEntry::new("keycap", SharedString::from("Keycap shortcut"))
                    .icon(ContextMenuIconSlot::Reserved)
                    .shortcut(Some("Ctrl+W".into()))
                    .shortcut_keycaps(true)
                    .render(theme, scale, cx)
                    .debug_selector(|| "row_keycap".to_string()),
            )
            .child(
                ContextMenuEntry::new("collapse_command", "Collapse feat/")
                    .icon(ContextMenuIconSlot::Icon("icons/chevron_down.svg".into()))
                    .render(theme, scale, cx)
                    .debug_selector(|| "row_collapse_command".to_string()),
            )
            .child(
                context_menu_group(theme, scale, false)
                    .debug_selector(|| "closed_group".to_string())
                    .child(
                        ContextMenuEntry::new("closed_group", "Remote branch origin/main")
                            .icon(ContextMenuIconSlot::Icon("icons/chevron_right.svg".into()))
                            .render(theme, scale, cx)
                            .debug_selector(|| "row_closed_group".to_string()),
                    ),
            )
            .child(
                context_menu_group(theme, scale, true)
                    .debug_selector(|| "open_group".to_string())
                    .child(
                        ContextMenuEntry::new("open_group", "Remote branch upstream/main")
                            .icon(ContextMenuIconSlot::Icon("icons/chevron_down.svg".into()))
                            .render(theme, scale, cx)
                            .debug_selector(|| "row_open_group".to_string()),
                    )
                    .child(
                        ContextMenuEntry::new("selected_group", "Checkout")
                            .icon(ContextMenuIconSlot::Reserved)
                            .selected(true)
                            .render(theme, scale, cx)
                            .debug_selector(|| "row_selected_group".to_string()),
                    ),
            )
            .child(
                context_menu_header(theme, scale, SharedString::from("Header"), None, cx)
                    .id("row_header_id")
                    .debug_selector(|| "row_header".to_string()),
            )
    }
}

struct ExtensionIcon {
    theme: AppTheme,
}

impl gpui::Render for ExtensionIcon {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let scale = crate::ui_scale::UiScale::current(cx);
        gpui::div()
            .child(
                ContextMenuEntry::new("ext", "Mark reviewed")
                    .icon(ContextMenuIconSlot::Icon(
                        "extensions/com.example.review/icons/review.svg".into(),
                    ))
                    .render(self.theme, scale, cx),
            )
            .child(
                ContextMenuEntry::new("kit", "Agents")
                    .icon(ContextMenuIconSlot::Icon("icons/terminal.svg".into()))
                    .render(self.theme, scale, cx),
            )
    }
}

/// An extension's own asset, and a kit icon the table does not name, are
/// drawn as given: dropping them left a blank slot.
#[gpui::test]
fn extension_menu_icons_draw_their_asset(cx: &mut gpui::TestAppContext) {
    let theme = AppTheme::gitcomet_dark();
    let (_view, cx) = cx.add_window_view(|_, _| ExtensionIcon { theme });
    crate::test_support::redraw(cx);
    assert!(
        cx.debug_bounds("context_menu_entry_icon_Mark reviewed")
            .is_some()
    );
    assert!(cx.debug_bounds("context_menu_entry_icon_Agents").is_some());
}

#[gpui::test]
fn review_collapse_commands_do_not_inherit_group_header_background(cx: &mut gpui::TestAppContext) {
    let theme = AppTheme::from_key("sunset_veil").unwrap();
    let (_view, cx) = cx.add_window_view(|_, _| RowKinds { theme });
    crate::test_support::redraw(cx);
    assert!(
        !crate::test_support::painted_control_quads(cx, "row_collapse_command")
            .iter()
            .any(|(fill, _)| *fill == theme.colors.surface.panel.into())
    );
}

/// Every kind of menu row is one height, at both densities. Keycaps and
/// headings each used to reserve a taller box than a plain entry.
#[gpui::test]
fn every_menu_row_kind_shares_one_height(cx: &mut gpui::TestAppContext) {
    for density in crate::appearance::UiDensity::ALL {
        cx.update(|app| {
            app.set_global(crate::appearance::Appearance {
                density,
                ..crate::appearance::Appearance::default()
            });
        });
        let theme = cx.update(|app| {
            crate::theme::AppTheme::gitcomet_dark().with_appearance(crate::appearance::current(app))
        });
        let (_view, cx) = cx.add_window_view(|_window, _cx| RowKinds { theme });
        crate::test_support::redraw(cx);

        let plain = cx
            .debug_bounds("row_plain")
            .expect("expected a plain entry to render")
            .size
            .height;
        for selector in [
            "row_text_shortcut",
            "row_keycap",
            "row_header",
            "row_closed_group",
            "row_open_group",
            "row_selected_group",
        ] {
            let height = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("expected {selector} to render"))
                .size
                .height;
            assert_eq!(
                height, plain,
                "{selector} must match a plain entry at {density:?} density"
            );
        }
        for selector in ["closed_group", "open_group"] {
            assert!(
                crate::test_support::painted_control_quads(cx, selector)
                    .iter()
                    .any(|(fill, _)| *fill == theme.colors.surface.panel.into())
            );
        }
        // One block spans the header and its rows; neighbours stay apart.
        let closed = cx.debug_bounds("closed_group").unwrap();
        let open = cx.debug_bounds("open_group").unwrap();
        let header = cx.debug_bounds("row_open_group").unwrap();
        let action = cx.debug_bounds("row_selected_group").unwrap();
        assert_eq!((open.top(), open.bottom()), (header.top(), action.bottom()));
        assert!(open.top() > closed.bottom());
        assert!(
            crate::test_support::painted_control_quads(cx, "row_selected_group")
                .iter()
                .any(|(fill, _)| *fill == theme.hover_overlay().into()),
            "group backgrounds must preserve keyboard selection feedback"
        );
    }
}

#[test]
fn secondary_heading_is_muted_against_the_primary() {
    for theme in [
        crate::theme::AppTheme::gitcomet_dark(),
        crate::theme::AppTheme::gitcomet_light(),
    ] {
        assert_ne!(
            theme.colors.foreground.secondary, theme.colors.foreground.primary,
            "the two headings must not share a color"
        );
    }
}

#[test]
fn context_menu_close_uses_normal_text_and_standard_icon_color() {
    let theme = AppTheme::gitcomet_dark();
    let close_icon = context_menu_icon_color(theme, false, ContextMenuTone::Accent);

    assert_eq!(close_icon, theme.colors.accent.foreground);
    assert_eq!(
        context_menu_entry_text_color(theme, false, ContextMenuTone::Accent),
        theme.colors.foreground.primary
    );
    assert_eq!(
        context_menu_entry_text_color(theme, false, ContextMenuTone::Danger),
        theme.colors.status.danger.foreground,
        "other destructive entries should retain their danger text"
    );
}

#[test]
fn explicit_menu_tones_resolve_icon_and_text_colors_with_disabled_precedence() {
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        for (tone, color) in [
            (ContextMenuTone::Accent, theme.colors.accent.foreground),
            (ContextMenuTone::Neutral, theme.colors.foreground.secondary),
            (
                ContextMenuTone::Success,
                theme.colors.status.success.foreground,
            ),
            (
                ContextMenuTone::Warning,
                theme.colors.status.warning.foreground,
            ),
            (
                ContextMenuTone::Danger,
                theme.colors.status.danger.foreground,
            ),
        ] {
            assert_eq!(context_menu_icon_color(theme, false, tone), color);
            assert_eq!(
                context_menu_entry_text_color(theme, false, tone),
                if tone == ContextMenuTone::Danger {
                    theme.colors.status.danger.foreground
                } else {
                    theme.colors.foreground.primary
                }
            );
            assert_eq!(
                context_menu_icon_color(theme, true, tone),
                theme.colors.foreground.secondary
            );
            assert_eq!(
                context_menu_entry_text_color(theme, true, tone),
                theme.colors.foreground.secondary
            );
        }
    }
}

#[test]
fn menu_icon_guard_rejects_unknown_assets_and_accepts_bundled_assets() {
    let sources = tempfile::tempdir().unwrap();
    std::fs::write(
        sources.path().join("menu.rs"),
        "icon: Some(\"icons/copy.svg\".into()),\nicon: Some(\"icons/missing-menu.svg\".into()),\n",
    )
    .unwrap();
    let (seen, unresolved) =
        crate::test_support::source_guards::unresolved_menu_icons(sources.path());
    assert_eq!(seen, 2);
    assert_eq!(unresolved, ["menu.rs: icons/missing-menu.svg"]);
}

#[test]
fn every_menu_icon_literal_in_the_kit_resolves() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (_seen, unresolved) = crate::test_support::source_guards::unresolved_menu_icons(&src_dir);
    assert!(
        unresolved.is_empty(),
        "menu icons must name an existing asset:\n{}",
        unresolved.join("\n")
    );
}
