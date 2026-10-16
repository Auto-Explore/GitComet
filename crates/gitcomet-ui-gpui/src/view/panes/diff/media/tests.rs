use super::*;
use crate::view::image_preview::{ImageViewport, Source, Zoom};
use std::sync::atomic::AtomicU64;

struct Harness {
    group: Entity<Group>,
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(ImageViewport {
            group: self.group.clone(),
            side: 0,
            theme: AppTheme::gitcomet_dark(),
        })
    }
}

#[gpui::test]
fn image_preview_regression_svg_conflict_keeps_encoded_fallback(cx: &mut gpui::TestAppContext) {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1100" height="300">
        <defs><filter id="blur"><feGaussianBlur stdDeviation="2"/></filter></defs>
        <rect width="1100" height="300" fill="#c86432" filter="url(#blur)"/>
    </svg>"##;
    let source = Source::new(gpui::ImageFormat::Svg, svg, 128 * 1024 * 1024).unwrap();
    let decoded = source.decode(0, &AtomicU64::new(0), 0).unwrap().unwrap();
    assert!(
        decoded.for_output((4400, 1200)).is_err(),
        "400% exceeds the SVG detail limit"
    );
    let encoded = Arc::new(gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        crate::view::diff_utils::rasterize_svg_preview_png(svg).unwrap(),
    ));
    let expected = image::load_from_memory(encoded.bytes()).unwrap().to_rgba8();
    let mut expected_center = expected.get_pixel(550, 150).0;
    expected_center.swap(0, 2);
    assert!(expected_center[0] < 128 && expected_center[3] == 255);
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    // Let the encoded asset finish loading before checking the handoff to
    // the image viewport, so a missing fallback fails deterministically.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let ready =
            cx.update(|window, app| encoded.clone().get_render_image(window, app).is_some());
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "encoded preview did not load"
        );
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let group = cx.update(|window, app| {
        root.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.conflict_resolver.image_preview.sources[0] = Some(source);
                pane.conflict_resolver.image_preview.images.base =
                    Loadable::Ready(Some(ConflictPreviewImage::Encoded(encoded)));
                pane.prepare_conflict_image_preview(window, cx)
            })
    });
    cx.update(|window, app| {
        window.replace_root(app, |_, _| Harness {
            group: group.clone(),
        });
        group.update(app, |group, cx| group.zoom(Zoom::Scale(4.0), None, cx));
        let preview = group.read(app).side_preview(0).expect(
            "encoded SVG conflict preview must remain renderable when detail rendering fails",
        );
        assert_eq!(
            preview.size(0),
            gpui::size(gpui::DevicePixels(1100), gpui::DevicePixels(300))
        );
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        crate::view::test_support::redraw(cx);
        cx.run_until_parked();
        if cx.update(|_, app| group.read(app).side_error(0).is_some()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "SVG detail request did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.update(|_, app| {
        let group = group.read(app);
        assert!(
            group
                .side_error(0)
                .unwrap()
                .contains("detailed preview limit")
        );
        let preview = group
            .side_preview(0)
            .expect("fallback survives the detail error");
        let center = (150 * 1100 + 550) * 4;
        assert_eq!(
            &preview.as_bytes(0).unwrap()[center..center + 4],
            &expected_center,
            "fallback preserves the encoded, filtered preview pixels"
        );
    });
}
