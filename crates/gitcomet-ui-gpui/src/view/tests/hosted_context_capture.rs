use super::*;

struct CardPreview {
    _owner: gpui::Entity<GitCometView>,
    pane: DiffPane,
}

impl Render for CardPreview {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family("IBM Plex Sans")
            .child(self.pane.view())
            .child(self._owner.read(cx).popover_host.clone())
    }
}

#[test]
#[ignore = "renders inline cards with native fonts and a GPU"]
fn inline_card_visual_capture() {
    let mut cx = gpui::HeadlessAppContext::with_platform(
        gpui_platform::current_platform(true).text_system(),
        Arc::new(crate::assets::GitCometAssets::default()),
        || Some(Box::new(gpui_wgpu::WgpuHeadlessRenderer::new().unwrap())),
    );
    cx.update(|app| {
        gitcomet_ui_kit::bundled_fonts::register(app).unwrap();
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(
                gitcomet_extension_example::review::ReviewExtension,
            )])
            .unwrap(),
            app,
        );
        app.set_global(crate::appearance::Appearance {
            density: crate::appearance::UiDensity::Comfortable,
            ..Default::default()
        });
    });
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/inline-card-previews");
    std::fs::create_dir_all(&directory).unwrap();
    for scale in [100, 150, 200] {
        cx.update(|app| ui_scale::set_default(app, scale));
        for width in [420.0, 900.0] {
            for layout in [DiffLayout::Inline, DiffLayout::Split] {
                let (store, events) = AppStore::new_test(Arc::new(TestBackend));
                let handle = cx.open_window(gpui::size(px(width), px(700.0)), move |window, app| {
                    ui_scale::apply_to_window(window, scale);
                    let owner = app.new(|cx| GitCometView::new(store, events, None, window, cx));
                    let host = owner.read(app).extension_window.as_ref().unwrap().host();
                    let pane = host.create_snapshot_pane(
                        DiffSnapshot::new("example.rs", "fn current() -> bool {\n    false\n}\n", "fn current() -> bool {\n    true\n}\n"),
                        DiffPaneOptions { layout, ..Default::default() }, app,
                    ).unwrap();
                    let marker = |color: gpui::Hsla, icon: &'static str, label: &'static str| {
                        gitcomet_extension_api::DiffAnnotation::new(color)
                            .with_glyph(gitcomet_extension_api::RowGlyph::Icon(icon.into()))
                            .with_label(label)
                    };
                    pane.set_annotations(
                        gitcomet_extension_api::DiffAnnotations::new()
                            .with(DiffLineSide::New, 1, marker(gpui::blue(), "icons/pin.svg", "Flagged"))
                            .with(DiffLineSide::New, 3, marker(gpui::green(), "icons/circle_check.svg", "Resolved")),
                        app,
                    );
                    pane.set_inline_cards(vec![
                        DiffInlineCard::new("answer", DiffLineRange { side: DiffLineSide::New, start: 2, end: 2 }, "AI explanation · line 2", "The function now returns true for every call. Check that its callers need this unconditional result and that the relevant tests cover the changed behaviour.\n\nMixed direction: עברית and English remain readable.")
                            .pending(true)
                            .with_header_action(gitcomet_extension_api::HostedAction::new("Stop", |_| {}), "icons/generic_maximize.svg")
                            .with_header_action(gitcomet_extension_api::HostedAction::new("Close", |_| {}), "icons/generic_close.svg"),
                    ], app);
                    app.new(|_| CardPreview { _owner: owner, pane })
                }).unwrap();
                for _ in 0..4 {
                    cx.run_until_parked();
                    cx.update_window(handle.into(), |_, window, app| {
                        window.refresh();
                        let _ = window.draw(app);
                    })
                    .unwrap();
                }
                cx.capture_screenshot(handle.into())
                    .unwrap()
                    .save(directory.join(format!("{layout:?}-{scale}-{width}.png")))
                    .unwrap();
                if scale == 100 && width == 900.0 && layout == DiffLayout::Inline {
                    let quiet = handle
                        .read_with(&cx, |preview, _| preview.pane.clone())
                        .unwrap();
                    cx.update(|app| {
                        let mut policy = gitcomet_extension_api::DiffPanePolicy::default();
                        policy.engine_label = false;
                        policy.header_divider = false;
                        quiet.set_policy(policy, app);
                    });
                    for _ in 0..4 {
                        cx.run_until_parked();
                        cx.update_window(handle.into(), |_, window, app| {
                            window.refresh();
                            let _ = window.draw(app);
                        })
                        .unwrap();
                    }
                    cx.capture_screenshot(handle.into())
                        .unwrap()
                        .save(directory.join("quiet-header.png"))
                        .unwrap();
                    use gitcomet_extension_api::{HostedAction, HostedMenuItem};
                    let owner = handle
                        .read_with(&cx, |preview, _| preview._owner.clone())
                        .unwrap();
                    cx.update(|app| {
                        owner.update(app, |owner, cx| {
                            owner.set_theme(crate::theme::AppTheme::gitcomet_dark(), cx)
                        })
                    });
                    for (name, items) in [
                        (
                            "notifications",
                            vec![
                                HostedMenuItem::action(HostedAction::new("Subscribed", |_| {}))
                                    .with_icon("icons/check.svg")
                                    .disabled(true),
                                HostedMenuItem::action(HostedAction::new("Not subscribed", |_| {})),
                                HostedMenuItem::action(HostedAction::new("Ignore", |_| {})),
                            ],
                        ),
                        (
                            "single-pr-actions",
                            vec![
                                HostedMenuItem::action(HostedAction::new("Open on GitHub", |_| {})),
                                HostedMenuItem::action(HostedAction::new("Notifications…", |_| {})),
                                HostedMenuItem::Separator,
                                HostedMenuItem::action(HostedAction::new(
                                    "Refresh local worktrees",
                                    |_| {},
                                ))
                                .with_tooltip("No local worktree at this revision"),
                            ],
                        ),
                    ] {
                        let menu = cx.update(|app| {
                            let host = owner.read(app).extension_window.as_ref().unwrap().host();
                            host.open_menu(gpui::point(px(300.0), px(64.0)), items, app)
                                .unwrap()
                        });
                        for _ in 0..4 {
                            cx.run_until_parked();
                            cx.update_window(handle.into(), |_, window, app| {
                                window.refresh();
                                let _ = window.draw(app);
                            })
                            .unwrap();
                        }
                        cx.capture_screenshot(handle.into())
                            .unwrap()
                            .save(directory.join(format!("{name}.png")))
                            .unwrap();
                        cx.update(|app| menu.close(app));
                        cx.run_until_parked();
                    }
                }
                cx.update_window(handle.into(), |_, window, _| window.remove_window())
                    .unwrap();
                cx.run_until_parked();
            }
        }
    }
}
