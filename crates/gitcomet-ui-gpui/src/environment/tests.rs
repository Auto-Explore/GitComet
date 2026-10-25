use super::*;
use gpui::AppContext as _;

#[test]
fn selected_gpu_specs_cover_hardware_software_missing_and_partial() {
    for (software, rendering) in [(false, Rendering::Hardware), (true, Rendering::Software)] {
        let details = graphics_details(
            None,
            Some(GpuSpecs {
                is_software_emulated: software,
                device_name: "selected device".into(),
                driver_name: "".into(),
                driver_info: "driver version".into(),
            }),
            Some("Vulkan".into()),
        );
        assert_eq!(details.rendering, rendering);
        assert_eq!(details.device_name.as_deref(), Some("selected device"));
        assert_eq!(details.driver_name, None);
        assert_eq!(details.driver_info.as_deref(), Some("driver version"));
        assert_eq!(details.backend.as_deref(), Some("Vulkan"));
    }
    let unavailable = graphics_details(None, None, Some("Metal".into()));
    assert_eq!(unavailable.rendering, Rendering::Unavailable);
    assert_eq!(unavailable.device_name, None);
    assert_eq!(unavailable.backend.as_deref(), Some("Metal"));
}

#[test]
fn x11_handle_wins_even_in_a_wayland_session() {
    const CHILD: &str = "GITCOMET_TEST_X11_ENVIRONMENT";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "environment::tests::x11_handle_wins_even_in_a_wayland_session",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("WAYLAND_DISPLAY", "wayland-test")
            .env("XDG_SESSION_TYPE", "wayland")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    assert_eq!(std::env::var("WAYLAND_DISPLAY").unwrap(), "wayland-test");
    assert_eq!(
        window_system(Some(RawWindowHandle::Xlib(
            raw_window_handle::XlibWindowHandle::new(1)
        ))),
        Some("X11")
    );
    assert_eq!(
        window_system(Some(RawWindowHandle::Xcb(
            raw_window_handle::XcbWindowHandle::new(std::num::NonZeroU32::new(1).unwrap())
        ))),
        Some("X11")
    );
    assert_eq!(
        window_system(Some(RawWindowHandle::Wayland(
            raw_window_handle::WaylandWindowHandle::new(std::ptr::NonNull::dangling())
        ))),
        Some("Wayland")
    );
    assert_eq!(window_system(None), None);
}

struct Probe;
impl gpui::Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

#[gpui::test]
fn refresh_keeps_each_live_window_and_removes_closed_windows(cx: &mut gpui::TestAppContext) {
    let (_, cx) = cx.add_window_view(|window, cx| {
        track_window(window, cx);
        Probe
    });
    let second = cx.update(|_, cx| {
        cx.open_window(Default::default(), |window, cx| {
            cx.new(|cx| {
                track_window(window, cx);
                Probe
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        refresh_current(window, cx);
        assert_eq!(cx.global::<Environment>().0.graphics.len(), 2);
        second
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(cx.global::<Environment>().0.graphics.len(), 1));
}
