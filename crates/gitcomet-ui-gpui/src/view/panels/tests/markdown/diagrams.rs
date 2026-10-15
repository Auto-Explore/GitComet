//! Mermaid blocks embedded in the rendered markdown preview.

use super::*;

const DIAGRAM_SOURCE: &str =
    "# Title\n\n```mermaid\nflowchart TD\nA-->B\nB-->C\nC-->D\n```\n\nAfter\n";

fn open_diagram_preview<'a>(
    cx: &'a mut gpui::TestAppContext,
    repo_id: u64,
    name: &str,
) -> (
    gpui::Entity<super::super::super::GitCometView>,
    &'a mut gpui::VisualTestContext,
    RenderedPreviewFixture,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(repo_id),
        name,
        DIAGRAM_SOURCE,
    );
    for _ in 0..20 {
        draw_frames(cx, 1);
        if cx.debug_bounds("diagram_viewport").is_some() {
            return (view, cx, fixture);
        }
    }
    panic!("the embedded diagram never finished rendering");
}

/// The window zoom listens in the capture phase and is painted last, so a
/// bubble-phase diagram listener never saw Ctrl/Cmd+scroll: the whole window
/// zoomed instead of the diagram.
#[gpui::test]
fn ctrl_scroll_over_an_embedded_diagram_zooms_the_diagram_not_the_window(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (view, cx, fixture) = open_diagram_preview(cx, 9701, "markdown_diagram_ctrl_scroll");
    let bounds = cx.debug_bounds("diagram_viewport").unwrap();
    let window_zoom = cx.update(|_, app| view.read(app).ui_scale_percent);
    cx.update(|_, app| {
        assert!(
            !crate::view::diagram_preview::cached_tile_zooms_for_tests(app).contains(&3),
            "no tile is zoomed in before the wheel"
        );
    });

    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(60.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_frames(cx, 2);

    cx.update(|_, app| {
        assert_eq!(
            view.read(app).ui_scale_percent,
            window_zoom,
            "Ctrl/Cmd+scroll over the diagram must not zoom the window"
        );
        assert!(
            crate::view::diagram_preview::cached_tile_zooms_for_tests(app).contains(&3),
            "Ctrl/Cmd+scroll over the diagram zooms it one step"
        );
    });

    // Outside the diagram the wheel still zooms the window.
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(px(5.0), px(5.0)),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(60.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_frames(cx, 1);
    cx.update(|_, app| assert!(view.read(app).ui_scale_percent > window_zoom));

    fixture.cleanup();
}
