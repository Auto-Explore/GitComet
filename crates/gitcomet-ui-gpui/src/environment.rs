//! Event-driven collection. Nothing here runs from render, prepaint, or paint.
use gitcomet_core::environment::{EnvironmentSnapshot, GraphicsDetails, Rendering, SystemDetails};
use gpui::{App, Context, GpuSpecs, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[derive(Clone, Default)]
pub(crate) struct Environment(pub EnvironmentSnapshot);
impl gpui::Global for Environment {}

pub(crate) fn initialize(cx: &mut App) {
    if cx.has_global::<Environment>() {
        return;
    }
    cx.set_global(Environment::default());
    cx.on_window_closed(|cx, _| request_refresh(cx)).detach();
    if cfg!(test) {
        return;
    }
    let work = cx
        .background_executor()
        .spawn(async { SystemDetails::collect_once() });
    cx.spawn(async move |cx| {
        let system = work.await;
        cx.update(|cx| {
            let mut snapshot = cx.global::<Environment>().0.clone();
            snapshot.system = system;
            publish(snapshot, cx);
        });
    })
    .detach();
}

/// Call once from each root view's constructor. Defer the first capture until
/// GPUI has finished opening the window and made its renderer available.
pub(crate) fn track_window<T: 'static>(window: &mut Window, cx: &mut Context<T>) {
    initialize(cx);
    window.defer(cx, refresh_current);
    cx.observe_window_activation(window, |_, window, cx| {
        if window.is_window_active() {
            refresh_current(window, cx);
        }
    })
    .detach();
}

pub(crate) fn request_refresh(cx: &mut App) {
    cx.defer(refresh_all);
}

pub(crate) fn refresh_git(cx: &mut App) {
    initialize(cx);
    let mut snapshot = cx.global::<Environment>().0.clone();
    snapshot.git_version = gitcomet_core::process::current_git_runtime()
        .version_output()
        .map(str::to_owned);
    publish(snapshot, cx);
}

fn refresh_all(cx: &mut App) {
    refresh_windows(None, cx);
}

pub(crate) fn refresh_current(window: &mut Window, cx: &mut App) {
    refresh_windows(Some(window), cx);
}

fn refresh_windows(current: Option<&Window>, cx: &mut App) {
    initialize(cx);
    let mut snapshot = cx.global::<Environment>().0.clone();
    let handles = cx.windows();
    let current_id = current.map(|window| window.window_handle().window_id().as_u64());
    snapshot.graphics.retain(|id, _| {
        handles
            .iter()
            .any(|handle| handle.window_id().as_u64() == *id)
    });
    for handle in handles {
        let id = handle.window_id().as_u64();
        if Some(id) == current_id {
            continue;
        }
        if let Ok(details) = handle.update(cx, |_, window, _| capture(window)) {
            snapshot.graphics.insert(id, details);
        }
    }
    if let Some(window) = current {
        snapshot
            .graphics
            .insert(window.window_handle().window_id().as_u64(), capture(window));
    }
    snapshot.git_version = gitcomet_core::process::current_git_runtime()
        .version_output()
        .map(str::to_owned);
    publish(snapshot, cx);
}

fn publish(snapshot: EnvironmentSnapshot, cx: &mut App) {
    if cx.global::<Environment>().0 == snapshot {
        return;
    }
    if !cfg!(test) {
        gitcomet_core::environment::publish(snapshot.clone());
        crate::ui_probe::environment(&snapshot);
    }
    cx.set_global(Environment(snapshot));
}

fn capture(window: &Window) -> GraphicsDetails {
    let handle = HasWindowHandle::window_handle(window)
        .ok()
        .map(|handle| handle.as_raw());
    let backend = graphics_backend(window);
    graphics_details(handle, window.gpu_specs(), backend)
}

fn graphics_details(
    handle: Option<RawWindowHandle>,
    specs: Option<GpuSpecs>,
    backend: Option<String>,
) -> GraphicsDetails {
    let mut details = GraphicsDetails {
        window_system: window_system(handle).map(str::to_owned),
        backend,
        ..Default::default()
    };
    if let Some(specs) = specs {
        details.device_name = nonempty(specs.device_name);
        details.driver_name = nonempty(specs.driver_name);
        details.driver_info = nonempty(specs.driver_info);
        details.rendering = if specs.is_software_emulated {
            Rendering::Software
        } else {
            Rendering::Hardware
        };
    }
    details
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

/// Do not infer this from WAYLAND_DISPLAY/XDG_SESSION_TYPE: X11 windows can
/// run within a Wayland session, including explicit backend overrides and WSLg.
fn window_system(handle: Option<RawWindowHandle>) -> Option<&'static str> {
    match handle? {
        RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) => Some("X11"),
        RawWindowHandle::Wayland(_) => Some("Wayland"),
        RawWindowHandle::Win32(_) => Some("Win32"),
        RawWindowHandle::AppKit(_) => Some("AppKit"),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn graphics_backend(window: &Window) -> Option<String> {
    use gpui_wgpu::{WgpuBackend, WgpuContextHandle, wgpu};
    let context = WgpuContextHandle::from_window(window)?;
    Some(match context.backend() {
        WgpuBackend::Gl | WgpuBackend::Native(wgpu::Backend::Gl) => "OpenGL".into(),
        WgpuBackend::Native(wgpu::Backend::Vulkan) => "Vulkan".into(),
        other => format!("{other:?}"),
    })
}

#[cfg(not(target_os = "linux"))]
fn graphics_backend(window: &Window) -> Option<String> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(_) => Some("DirectX 11".into()),
        RawWindowHandle::AppKit(_) => Some("Metal".into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
