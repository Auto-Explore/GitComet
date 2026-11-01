use super::*;
use crate::view::repository_views::RoutedArea;

#[test]
#[ignore = "renders sidebar previews using the native text backend and a GPU"]
fn sidebar_header_visual_capture() {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = gpui::HeadlessAppContext::with_platform(
        text_system,
        Arc::new(crate::assets::GitCometAssets::default()),
        || {
            Some(Box::new(
                gpui_wgpu::WgpuHeadlessRenderer::new().expect("headless GPU renderer"),
            ))
        },
    );
    let registry = gitcomet_extension_api::Registry::build(vec![Box::new(
        super::super::extension_view_panels::Panels,
    )])
    .unwrap();
    cx.update(|app| crate::view::extension_host::install(registry, app));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let state = Arc::new(view_state_with_active_ready_repo(RepoId(1)));
    store.replace_snapshot_for_test(state.clone());
    let handle = cx
        .open_window(gpui::size(px(1600.0), px(900.0)), |window, app| {
            app.new(|cx| GitCometView::new(store.clone(), events, None, window, cx))
        })
        .unwrap();
    handle
        .update(&mut cx, |view, window, cx| {
            test_support::push_test_state(view, state, cx);
            view.select_routed_view(RoutedArea::Main, Some(0), window, cx);
            view.open_file_history(RepoId(1), "src/main.rs".into(), window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sidebar-preview");
    std::fs::create_dir_all(&directory).unwrap();
    for (name, selected, width) in [
        ("branches", None, 280.0),
        ("history", None, 280.0),
        ("extension", Some(1), 280.0),
        ("wide", Some(0), 600.0),
        ("light", Some(0), 400.0),
    ] {
        handle
            .update(&mut cx, |view, window, cx| {
                view.set_theme(
                    if name == "light" {
                        AppTheme::gitcomet_light()
                    } else {
                        AppTheme::gitcomet_dark()
                    },
                    cx,
                );
                view.select_routed_view(RoutedArea::Sidebar, selected, window, cx);
                view.sidebar_pane.update(cx, |pane, cx| {
                    pane.file_history_selected = name == "history";
                    cx.emit(panes::SidebarHeaderChanged);
                    cx.notify();
                });
                test_support::set_sidebar_width_for_test(view, px(width), cx);
                view.toast_host
                    .update(cx, |host, cx| host.dismiss_all_errors(cx));
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, app| {
            window.refresh();
            let _ = window.draw(app);
        })
        .unwrap();
        let image = cx.capture_screenshot(handle.into()).unwrap();
        image.save(directory.join(format!("{name}.png"))).unwrap();
        let header = cx
            .debug_bounds(handle.into(), "sidebar_header")
            .unwrap()
            .unwrap();
        println!("{name}: {header:?}");
    }
}
