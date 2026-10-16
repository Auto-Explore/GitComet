use super::*;

#[test]
fn theme_orbs_paint_each_themes_chrome_accent_and_keyword() {
    use super::theme_grid::{OrbPaint, orb_svg};
    let hex = |color: gpui::Rgba| {
        format!(
            "#{:02x}{:02x}{:02x}",
            (color.red * 255.0).round() as u8,
            (color.green * 255.0).round() as u8,
            (color.blue * 255.0).round() as u8
        )
    };
    let tokyo = crate::theme::theme_preview_colors("tokyo_night").expect("bundled theme");
    let svg = orb_svg(OrbPaint::Single(tokyo));
    assert!(
        svg.contains(&format!("stop-color=\"{}\"", hex(tokyo.glow))),
        "{svg}"
    );
    assert!(
        svg.contains(&format!("stop-color=\"{}\"", hex(tokyo.secondary))),
        "{svg}"
    );
    assert!(svg.contains("radialGradient"));
    // The circle and its edge are gpui's anti-aliased corners, not the image's.
    assert!(!svg.contains("<circle"), "{svg}");

    let themes = crate::theme::ThemeCatalog::load();
    let OrbPaint::Split { light, dark } =
        OrbPaint::for_mode(&ThemeMode::Automatic, &themes).expect("automatic orb")
    else {
        panic!("Automatic paints GitComet Light and Dark halves");
    };
    assert_eq!(light.base, AppTheme::gitcomet_light().colors.surface.chrome);
    assert_eq!(dark.base, AppTheme::gitcomet_dark().colors.surface.chrome);
    let split = orb_svg(OrbPaint::Split { light, dark });
    for color in [light.glow, dark.glow] {
        assert!(split.contains(&hex(color)), "{split}");
    }
    assert!(split.contains("url(#l)") && split.contains("url(#r)"));
}

#[test]
fn theme_orbs_keep_a_translucent_accent_translucent() {
    use super::theme_grid::{OrbPaint, orb_svg};
    let opaque = crate::theme::theme_preview_colors("tokyo_night").expect("bundled theme");
    let translucent = crate::theme::ThemePreviewColors {
        glow: crate::theme::with_alpha(opaque.glow, 0.5),
        ..opaque
    };
    assert_ne!(
        orb_svg(OrbPaint::Single(opaque)),
        orb_svg(OrbPaint::Single(translucent)),
        "the accent's alpha reaches the orb"
    );
}

/// Each save of a live-reloaded custom theme is a new palette.
#[test]
fn orb_image_cache_stays_bounded_across_palette_edits() {
    use super::theme_grid::{OrbImageCache, OrbPaint};
    let mut cache = OrbImageCache::default();
    let base = crate::theme::theme_preview_colors("tokyo_night").expect("bundled theme");
    for step in 0..300u32 {
        let mut glow = base.glow;
        glow.red = (step % 256) as f32 / 255.0;
        glow.green = (step / 256) as f32 / 255.0;
        let _ = cache.get(OrbPaint::Single(crate::theme::ThemePreviewColors {
            glow,
            ..base
        }));
    }
    assert!(cache.len() <= 128, "{} orb images cached", cache.len());
}

#[test]
fn theme_tiles_list_automatic_first_then_every_theme_once_by_appearance() {
    let groups = super::theme_grid::grouped_tile_keys();
    assert_eq!(groups[0], ("automatic", vec!["automatic".to_string()]));

    let mut listed = Vec::new();
    for (group, keys) in &groups[1..] {
        for key in keys {
            let option = crate::theme::available_themes()
                .into_iter()
                .find(|option| &option.key == key)
                .unwrap_or_else(|| panic!("`{key}` is not an available theme"));
            let expected = match (option.custom, option.is_dark) {
                (true, _) => "custom",
                (false, true) => "dark",
                (false, false) => "light",
            };
            assert_eq!(*group, expected, "{key}");
            listed.push(key.clone());
        }
    }
    let mut available = crate::theme::available_themes()
        .into_iter()
        .map(|option| option.key)
        .collect::<Vec<_>>();
    available.sort();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(sorted, available, "every theme gets exactly one tile");

    let group = |name: &str| {
        groups
            .iter()
            .find(|(group, _)| *group == name)
            .map(|(_, keys)| keys.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        group("dark").first().map(String::as_str),
        Some("gitcomet_dark")
    );
    assert_eq!(
        group("light").first().map(String::as_str),
        Some("gitcomet_light")
    );
}

/// The Appearance page runs Typography -> Theme -> Interface, and typography
/// orders font pickers -> ligatures -> sizes with no presets popover.
#[gpui::test]
fn appearance_card_puts_ligatures_between_the_font_pickers_and_the_sizes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(2400.0)));
    settings_cx.run_until_parked();
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut bounds = |selector: &'static str| {
        settings_cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected `{selector}` in the appearance card"))
    };
    let theme_heading = bounds("settings_window_appearance_theme");
    let tiles = bounds("settings_window_theme_grid");
    let interface_heading = bounds("settings_window_appearance_interface");
    let ui_scale = bounds("settings_window_ui_scale");
    let density = bounds("settings_window_density_controls");
    let window_controls = bounds("settings_window_window_controls");
    let typography_heading = bounds("settings_window_appearance_typography");
    let ui_font = bounds("settings_window_ui_font");
    let editor_font = bounds("settings_window_editor_font");
    let ligatures = bounds("settings_window_use_font_ligatures");
    let sizes = bounds("settings_window_font_size_controls");

    for (upper, lower, what) in [
        (typography_heading, ui_font, "typography heading -> UI font"),
        (sizes, theme_heading, "font sizes -> theme heading"),
        (theme_heading, tiles, "theme heading -> tiles"),
        (tiles, interface_heading, "tiles -> interface heading"),
        (interface_heading, ui_scale, "interface heading -> UI scale"),
        (ui_scale, density, "UI scale -> density"),
        (density, window_controls, "density -> window controls"),
    ] {
        assert!(
            upper.bottom() <= lower.top(),
            "expected {what}: {upper:?} {lower:?}"
        );
    }

    assert!(
        ui_font.bottom() <= editor_font.top()
            && editor_font.bottom() <= ligatures.top()
            && ligatures.bottom() <= sizes.top(),
        "expected UI font -> editor font -> ligatures -> sizes, got \
         {ui_font:?} {editor_font:?} {ligatures:?} {sizes:?}"
    );

    for presets in [
        "font_size_0_presets",
        "font_size_1_presets",
        "font_size_2_presets",
    ] {
        assert!(
            settings_cx.debug_bounds(presets).is_none(),
            "the font size rows must not offer a presets popover"
        );
    }
}

#[gpui::test]
fn appearance_page_renders_theme_utilities_and_opens_theme_guide(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_links_container")
            .is_some(),
        "expected the appearance page to render theme utility links"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_custom_folder")
            .is_some(),
        "expected the appearance page to render the custom folder action"
    );

    let guide_bounds = settings_cx
        .debug_bounds("settings_window_theme_guide")
        .expect("expected theme guide row bounds");
    let scroll_bounds = settings_cx
        .debug_bounds("settings_window_scroll")
        .expect("expected settings scroll bounds");
    if guide_bounds.center().y >= scroll_bounds.bottom() {
        let scroll_delta = guide_bounds.center().y - scroll_bounds.bottom() + px(24.0);
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            let current = settings.settings_window_scroll.offset();
            settings
                .settings_window_scroll
                .set_offset(point(current.x, current.y - scroll_delta));
            cx.notify();
        });
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let guide_bounds = settings_cx
        .debug_bounds("settings_window_theme_guide")
        .expect("expected theme guide row bounds after scrolling");
    settings_cx.simulate_click(guide_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();

    assert_eq!(cx.opened_url(), themes_guide_url());
}

#[gpui::test]
fn settings_window_links_card_includes_theme_guide_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Links, cx);
        let current_x = settings.settings_window_scroll.offset().x;
        let max_offset = settings.settings_window_scroll.max_offset().y.max(px(0.0));
        settings
            .settings_window_scroll
            .set_offset(point(current_x, -max_offset));
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_links_theme_guide")
            .is_some(),
        "expected the Links card to include a Theme guide row"
    );
}

#[gpui::test]
fn ui_font_dropdown_wheel_scrolls_inner_list_before_outer_window(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let synthetic_fonts: Arc<[String]> = (0..200)
        .map(|ix| format!("Test UI Font {ix:03}"))
        .collect::<Vec<_>>()
        .into();

    cx.update(|_window, app| {
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.ui_font_options = synthetic_fonts.clone();
            settings.ui_font_family = synthetic_fonts[0].clone();
            settings.set_expanded_section(Some(SettingsSection::UiFont), cx);
            settings.settings_window_scroll = ScrollHandle::default();
            settings.ui_font_scroll = UniformListScrollHandle::default();
            cx.notify();
        });
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(460.0)));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let initial_list_bounds = settings_cx
        .debug_bounds("settings_window_ui_font_list_container")
        .expect("expected UI font list bounds");
    let scroll_bounds = settings_cx
        .debug_bounds("settings_window_scroll")
        .expect("expected settings scroll bounds");
    let list_center = initial_list_bounds.center();
    if list_center.y >= scroll_bounds.bottom() {
        let scroll_delta = list_center.y - scroll_bounds.bottom() + px(24.0);
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            let current = settings.settings_window_scroll.offset();
            settings
                .settings_window_scroll
                .set_offset(point(current.x, current.y - scroll_delta));
            cx.notify();
        });
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let list_bounds = settings_cx
        .debug_bounds("settings_window_ui_font_list_container")
        .expect("expected visible UI font list bounds");

    let (outer_before, inner_before, outer_max, inner_max) = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            (
                absolute_scroll_y(&settings.settings_window_scroll),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
                settings.settings_window_scroll.max_offset().y.max(px(0.0)),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).2,
            )
        })
        .expect("settings window should remain readable");
    assert!(
        outer_max > px(0.0),
        "expected the settings page to be scrollable during the test"
    );
    assert!(
        inner_max > px(0.0),
        "expected the UI font list to be scrollable during the test"
    );

    settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
    settings_cx.simulate_event(ScrollWheelEvent {
        position: list_bounds.center(),
        delta: ScrollDelta::Pixels(point(px(-120.0), px(0.0))),
        ..Default::default()
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (outer_after_horizontal_scroll, inner_after_horizontal_scroll) = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            (
                absolute_scroll_y(&settings.settings_window_scroll),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
            )
        })
        .expect("settings window should remain readable");

    assert!(
        (inner_after_horizontal_scroll - inner_before).abs() <= px(0.5),
        "expected horizontal-only wheel scroll not to move the UI font list vertically"
    );
    assert!(
        (outer_after_horizontal_scroll - outer_before).abs() <= px(0.5),
        "expected horizontal-only wheel scroll not to move the outer settings page vertically"
    );

    settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
    settings_cx.simulate_event(ScrollWheelEvent {
        position: list_bounds.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (outer_after_inner_scroll, inner_after_inner_scroll) = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            (
                absolute_scroll_y(&settings.settings_window_scroll),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
            )
        })
        .expect("settings window should remain readable");

    assert!(
        inner_after_inner_scroll > inner_before + px(0.5),
        "expected the UI font list to consume wheel scroll first"
    );
    assert!(
        (outer_after_inner_scroll - outer_before).abs() <= px(0.5),
        "expected the outer settings page to stay still while the UI font list can still scroll"
    );

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        let (raw_offset, _scroll_offset, max_offset) =
            uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll);
        let current_x = settings.ui_font_scroll.0.borrow().base_handle.offset().x;
        let target_y = if raw_offset > px(0.0) {
            max_offset
        } else {
            -max_offset
        };
        settings
            .ui_font_scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(current_x, target_y));
        cx.notify();
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let outer_before_boundary_handoff = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            absolute_scroll_y(&settings.settings_window_scroll)
        })
        .expect("settings window should remain readable");

    settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
    settings_cx.simulate_event(ScrollWheelEvent {
        position: list_bounds.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let outer_after_boundary_handoff = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            absolute_scroll_y(&settings.settings_window_scroll)
        })
        .expect("settings window should remain readable");

    assert!(
        outer_after_boundary_handoff > outer_before_boundary_handoff + px(0.5),
        "expected wheel scrolling to bubble to the outer settings page once the UI font list reaches its boundary"
    );
}

#[gpui::test]
fn appearance_sizes_apply_live_to_every_main_window_and_keep_ui_scale_independent(
    cx: &mut gpui::TestAppContext,
) {
    let (first_store, first_events) = AppStore::new_test(Arc::new(TestBackend));
    let (first, first_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(first_store, first_events, None, window, cx)
    });
    first_cx.update(|_, app| open_settings_window(app));
    first_cx.run_until_parked();
    let settings = first_cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    // A second window participates in the same application-wide preferences.
    let (second_store, second_events) = AppStore::new_test(Arc::new(TestBackend));
    let (second, second_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(second_store, second_events, None, window, cx)
    });
    let before_rem = second_cx.update(|window, _| window.rem_size());
    settings
        .update(second_cx, |settings, _, cx| {
            settings.set_density(UiDensity::Comfortable, cx);
            settings.set_font_size(FontRole::Ui, 20, cx);
            settings.set_font_size(FontRole::Editor, 26, cx);
            settings.set_font_size(FontRole::Markdown, 18, cx);
        })
        .unwrap();
    second_cx.run_until_parked();
    second_cx.update(|window, app| {
        assert_eq!(
            window.rem_size(),
            before_rem,
            "font sizes must not rescale panel geometry"
        );
        let metrics = crate::appearance::current(app);
        for view in [&first, &second] {
            let view = view.read(app);
            assert_eq!(view.theme.metrics, metrics);
            assert_eq!(view.theme.metrics.ui_font_size_px, 20);
            assert_eq!(view.theme.metrics.editor_font_size_px, 26);
            assert_eq!(view.theme.metrics.markdown_preview_font_size_px, 18);
            assert_eq!(
                view.diff_renderer
                    .read(app)
                    .file_editor_input
                    .read(app)
                    .line_height_override(),
                Some(view.theme.editor_row_height(view.ui_scale_percent))
            );
        }
    });
    settings
        .update(second_cx, |settings, _, cx| {
            settings.set_font_size(FontRole::Ui, 0, cx);
            settings.set_font_size(FontRole::Editor, 99, cx);
            assert_eq!(settings.appearance_metrics.ui_font_size_px, 20);
            assert_eq!(settings.appearance_metrics.editor_font_size_px, 26);
            settings.set_font_size(FontRole::Ui, FontRole::Ui.default_size(), cx);
            assert_eq!(settings.appearance_metrics.editor_font_size_px, 26);
            assert_eq!(
                settings.appearance_metrics.markdown_preview_font_size_px,
                18
            );
            settings.font_size_inputs[FontRole::Ui.index()]
                .update(cx, |input, cx| input.set_text("invalid", cx));
            settings.set_font_size(FontRole::Ui, FontRole::Ui.default_size(), cx);
            assert_eq!(
                settings.font_size_inputs[FontRole::Ui.index()]
                    .read(cx)
                    .text(),
                "14",
                "Reset must repair an invalid draft even at the default size"
            );
        })
        .unwrap();
}

#[gpui::test]
fn zoom_is_per_window_and_the_default_moves_only_unzoomed_windows(cx: &mut gpui::TestAppContext) {
    let (first_store, first_events) = AppStore::new_test(Arc::new(TestBackend));
    let (first, first_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(first_store, first_events, None, window, cx)
    });
    let first_window = first_cx.window_handle();
    first_cx.update(|_, app| open_settings_window(app));
    first_cx.run_until_parked();
    let settings = first_cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    let (second_store, second_events) = AppStore::new_test(Arc::new(TestBackend));
    let (second, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(second_store, second_events, None, window, cx)
    });
    let second_window = cx.window_handle();

    let scales = |cx: &mut gpui::VisualTestContext| {
        (
            first.read_with(cx, |view, _| view.ui_scale_percent),
            second.read_with(cx, |view, _| view.ui_scale_percent),
        )
    };
    // What a window's own reads resolve to, and its rem size.
    let window_scale = |cx: &mut gpui::VisualTestContext, handle: gpui::AnyWindowHandle| {
        handle
            .update(cx, |_, window, app| {
                assert_eq!(
                    window.rem_size(),
                    crate::ui_scale::rem_size_for_percent(crate::ui_scale::current(app).percent)
                );
                crate::ui_scale::current(app).percent
            })
            .unwrap()
    };
    let set_zoom = |cx: &mut gpui::VisualTestContext, percent: Option<u32>| {
        gpui::TestAppContext::update(cx, |app| {
            crate::app::set_window_ui_scale_percent(app, first_window.window_id(), percent);
        });
        cx.run_until_parked();
    };

    set_zoom(cx, Some(125));
    assert_eq!(scales(cx), (125, 100), "only the zoomed window changes");
    assert_eq!(window_scale(cx, first_window), 125);
    assert_eq!(window_scale(cx, second_window), 100);

    // A new default moves windows without their own zoom, Settings included.
    settings
        .update(cx, |settings, window, cx| {
            settings.set_ui_scale_percent(110, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(scales(cx), (125, 110), "the zoomed window keeps its zoom");
    assert_eq!(window_scale(cx, second_window), 110);
    settings
        .update(cx, |settings, _, _| {
            assert_eq!(settings.default_ui_scale_percent, 110);
            assert_eq!(settings.ui_scale_percent, 110);
            assert_eq!(settings.preference_settings().ui_scale_percent, Some(110));
        })
        .unwrap();

    // Resetting returns the window to the default.
    set_zoom(cx, None);
    assert_eq!(scales(cx), (110, 110));
    assert_eq!(window_scale(cx, first_window), 110);
}

#[test]
fn density_and_font_geometry_remain_independent_across_scales() {
    for percent in [80, 100, 125, 200] {
        for density in UiDensity::ALL {
            for ui_size in [10, 14, 24] {
                for editor_size in [8, 13, 32] {
                    let metrics = Appearance {
                        density,
                        ui_font_size_px: ui_size,
                        editor_font_size_px: editor_size,
                        markdown_preview_font_size_px: 18,
                    };
                    let theme = AppTheme::gitcomet_dark().with_appearance(metrics);
                    let scale = ui_scale::UiScale::from_percent(percent).with_appearance(metrics);
                    assert!(
                        components::control_height(scale)
                            >= scale.px(if density == UiDensity::Comfortable {
                                32.0
                            } else {
                                22.0
                            })
                    );
                    assert_eq!(
                        theme.editor_font_size(percent),
                        scale.px(editor_size as f32)
                    );
                    assert!(theme.editor_row_height(percent) > theme.editor_font_size(percent));
                    assert_eq!(theme.markdown_px(13.0, percent), scale.px(18.0));
                    let gutter = crate::view::rows::resolved_output_line_no_width(1234, scale);
                    assert!(
                        gutter >= scale.px(4.0 * editor_size as f32 * 0.6),
                        "gutter must hold every digit at the selected editor size"
                    );
                }
            }
        }
    }
}

#[test]
fn appearance_page_owns_themes_fonts_scale_and_density_in_search() {
    #[cfg(target_os = "windows")]
    {
        for query in [
            "graphics",
            "graphics renderer",
            "directx 12",
            "dx11",
            "compatibility",
        ] {
            assert!(
                SettingsCategory::General.matches_query(query),
                "{query} should find the General page"
            );
            assert!(
                !SettingsCategory::Appearance.matches_query(query),
                "{query} moved off the Appearance page"
            );
        }
        assert_eq!(
            SettingsSection::GraphicsRenderer.category(),
            SettingsCategory::General
        );
    }
    for query in [
        "theme",
        "solarized",
        "catppuccin",
        "density",
        "font size",
        "ui font",
        "ligatures",
        "ui scale",
        "window controls",
    ] {
        assert!(
            SettingsCategory::Appearance.matches_query(query),
            "{query} should find the Appearance page"
        );
    }
    for query in ["theme", "density", "font size", "ui font"] {
        assert!(
            !SettingsCategory::General.matches_query(query),
            "{query} moved off the General page"
        );
    }
    for query in ["external code editor", "timezone", "command line"] {
        assert!(
            SettingsCategory::General.matches_query(query),
            "{query} stays on the General page"
        );
    }
    for section in [
        SettingsSection::UiScale,
        SettingsSection::WindowControls,
        SettingsSection::UiFont,
        SettingsSection::EditorFont,
    ] {
        assert_eq!(
            section.category(),
            SettingsCategory::Appearance,
            "{section:?}"
        );
    }
    assert_eq!(
        SettingsSection::BrowserOpenTarget.category(),
        SettingsCategory::General
    );
}

/// Custom and newly bundled themes are found by name without a hand-kept list.
#[test]
fn appearance_search_finds_every_theme_by_name() {
    for option in crate::theme::available_themes() {
        let query = option.label.to_lowercase();
        assert!(
            SettingsCategory::Appearance.matches_query(&query),
            "{query} should find the Appearance page"
        );
    }
}

/// A workspace whose theme was deleted follows the app theme, so the note must
/// not count it.
#[gpui::test]
fn appearance_page_does_not_count_an_override_whose_theme_is_gone(cx: &mut gpui::TestAppContext) {
    let mut workspace =
        gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/appearance-tiles-c")]);
    workspace.theme_mode = Some("deleted_custom_theme".to_string());
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace]);
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();
    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_grid")
            .is_some()
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_workspace_overrides")
            .is_none(),
        "an override naming a deleted theme is no override"
    );
}

/// Every theme lookup walks the themes folder in the app, so a page resolves
/// all its tiles, labels and overrides from one read per draw.
#[gpui::test]
fn theme_pages_read_the_themes_folder_once_per_draw(cx: &mut gpui::TestAppContext) {
    let workspaces = ["tokyo_night", "nord", "deleted_custom_theme"].map(|key| {
        let mut workspace = gitcomet_state::session::Workspace::new(vec![PathBuf::from(format!(
            "/tmp/theme-lookups-{key}"
        ))]);
        workspace.theme_mode = Some(key.to_string());
        workspace
    });
    let id = workspaces[0].id;
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, workspaces.to_vec());
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();
    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(2400.0)));
    let lookups_per_draw = |settings_cx: &mut gpui::VisualTestContext| {
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
            let before = crate::theme::runtime_theme_lookups_for_test();
            window.refresh();
            let _ = window.draw(app);
            crate::theme::runtime_theme_lookups_for_test() - before
        })
    };

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    let appearance = lookups_per_draw(&mut settings_cx);

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Workspaces, cx);
        settings.select_workspace(id, cx);
        settings.toggle_section(SettingsSection::WorkspaceTheme, cx);
    });
    let workspaces_page = lookups_per_draw(&mut settings_cx);

    assert_eq!(
        (appearance, workspaces_page),
        (1, 1),
        "(Appearance, Workspaces) theme-folder reads per draw"
    );
}
