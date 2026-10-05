use crate::bindings::Windows::Win32::{
    CreateDXGIFactory2, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_11_1, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_DEBUG,
    D3D11_SDK_VERSION, D3D11CreateDevice, DXGI_CREATE_FACTORY_DEBUG, DXGI_ERROR_NOT_FOUND, HMODULE,
    ID3D11Device, ID3D11DeviceContext, IDXGIAdapter1, IDXGIFactory6,
};
use anyhow::{Context, Result};
use gpui_util::ResultExt;
use itertools::Itertools;
use windows_core::Interface;

pub(crate) fn try_to_recover_from_device_lost<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    (0..5)
        .map(|i| {
            if i > 0 {
                // Add a small delay before retrying
                std::thread::sleep(std::time::Duration::from_millis(100 + i * 10));
            }
            f()
        })
        .find_or_last(Result::is_ok)
        .unwrap()
        .context("DirectXRenderer failed to recover from lost device after multiple attempts")
}

#[derive(Clone)]
pub(crate) struct DirectXDevices {
    pub(crate) adapter: IDXGIAdapter1,
    pub(crate) dxgi_factory: IDXGIFactory6,
    pub(crate) device: ID3D11Device,
    pub(crate) device_context: ID3D11DeviceContext,
}

// D3D/DXGI interfaces can move between threads. Callers still serialize access
// to the immediate device context when rendering.
unsafe impl Send for DirectXDevices {}

/// Whether DirectX devices are created with the D3D11/DXGI debug layer:
/// `auto` (the default) enables it in debug builds only, `on` enables it in any
/// build, and `off` disables it. The layer validates every API call, which
/// makes rendering considerably slower.
const D3D_DEBUG: &str = "GPUI_D3D_DEBUG";

impl DirectXDevices {
    pub(crate) fn new() -> Result<Self> {
        Self::with_debug_layer(check_debug_layer_available())
    }

    pub(crate) fn with_debug_layer(debug_layer_available: bool) -> Result<Self> {
        let dxgi_factory =
            get_dxgi_factory(debug_layer_available).context("Creating DXGI factory")?;
        let (adapter, device, device_context, feature_level) =
            get_adapter(&dxgi_factory, debug_layer_available).context("Getting DXGI adapter")?;
        match feature_level {
            D3D_FEATURE_LEVEL_11_1 => {
                log::info!("Created device with Direct3D 11.1 feature level.")
            }
            D3D_FEATURE_LEVEL_11_0 => {
                log::info!("Created device with Direct3D 11.0 feature level.")
            }
            _ => unreachable!(),
        }

        Ok(Self {
            adapter,
            dxgi_factory,
            device,
            device_context,
        })
    }
}

fn check_debug_layer_available() -> bool {
    use crate::bindings::Windows::Win32::{DXGIGetDebugInterface1, IDXGIInfoQueue};

    let mode = std::env::var(D3D_DEBUG).unwrap_or_default();
    let requested = debug_layer_requested(&mode, cfg!(debug_assertions)).unwrap_or_else(|| {
        log::warn!("Ignoring {D3D_DEBUG}={mode:?}; expected auto, on, or off.");
        cfg!(debug_assertions)
    });
    if !requested {
        return false;
    }
    let available = unsafe { DXGIGetDebugInterface1::<IDXGIInfoQueue>(0) }
        .log_err()
        .is_some();
    if !available {
        log::warn!(
            "Failed to get DXGI debug interface. DirectX debugging features will be disabled."
        );
    }
    available
}

/// Parses a [`D3D_DEBUG`] value, returning `None` if it isn't recognized.
fn debug_layer_requested(mode: &str, debug_build: bool) -> Option<bool> {
    let mode = mode.trim();
    if mode.is_empty() || mode.eq_ignore_ascii_case("auto") {
        Some(debug_build)
    } else if mode.eq_ignore_ascii_case("on") {
        Some(true)
    } else if mode.eq_ignore_ascii_case("off") {
        Some(false)
    } else {
        None
    }
}

#[inline]
fn get_dxgi_factory(debug_layer_available: bool) -> Result<IDXGIFactory6> {
    let factory_flag = if debug_layer_available {
        DXGI_CREATE_FACTORY_DEBUG
    } else {
        0
    };
    unsafe { Ok(CreateDXGIFactory2(factory_flag as u32)?) }
}

#[inline]
fn get_adapter(
    dxgi_factory: &IDXGIFactory6,
    debug_layer_available: bool,
) -> Result<(
    IDXGIAdapter1,
    ID3D11Device,
    ID3D11DeviceContext,
    D3D_FEATURE_LEVEL,
)> {
    for adapter_index in 0.. {
        let adapter: IDXGIAdapter1 = match unsafe { dxgi_factory.EnumAdapters(adapter_index) } {
            Ok(adapter) => adapter.cast()?,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(error.into()),
        };
        let mut desc = crate::bindings::Windows::Win32::DXGI_ADAPTER_DESC1::default();
        if unsafe { adapter.GetDesc1(&mut desc).is_ok() } {
            let gpu_name = String::from_utf16_lossy(&desc.Description)
                .trim_matches(char::from(0))
                .to_string();
            log::info!("Using GPU: {}", gpu_name);
        }
        // Check to see whether the adapter supports Direct3D 11 and create
        // the device if it does.
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL::default();
        if let Some(device) = get_device(
            &adapter,
            Some(&mut context),
            Some(&mut feature_level),
            debug_layer_available,
        )
        .log_err()
        {
            let context = context.context("D3D11CreateDevice returned no device context")?;
            return Ok((adapter, device, context, feature_level));
        }
    }

    anyhow::bail!("No compatible Direct3D 11 adapter found")
}

#[inline]
fn get_device(
    adapter: &IDXGIAdapter1,
    context: Option<*mut Option<ID3D11DeviceContext>>,
    feature_level: Option<*mut D3D_FEATURE_LEVEL>,
    debug_layer_available: bool,
) -> Result<ID3D11Device> {
    let mut device: Option<ID3D11Device> = None;
    let device_flags = if debug_layer_available {
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_DEBUG
    } else {
        D3D11_CREATE_DEVICE_BGRA_SUPPORT
    };
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            device_flags as u32,
            // The generated shader corpus is Shader Model 5.0. Restrict device creation to
            // feature levels that can execute it instead of accepting FL10.1 and failing later
            // while constructing the renderer's first pipeline.
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION as u32,
            Some(&mut device),
            feature_level,
            context,
        )
        .ok()?;
    }
    let device = device.context("D3D11CreateDevice returned no device")?;
    // FL11.0/11.1 guarantees the compute/raw-buffer capabilities required by the SM5 DXBC
    // artifacts, so no legacy D3D10.x optional-feature probe is needed here.
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::debug_layer_requested;

    #[test]
    fn debug_layer_follows_the_build_unless_overridden() {
        for debug_build in [false, true] {
            for auto in ["", "auto", " Auto ", "AUTO"] {
                assert_eq!(debug_layer_requested(auto, debug_build), Some(debug_build));
            }
            for on in ["on", " on ", "ON"] {
                assert_eq!(debug_layer_requested(on, debug_build), Some(true));
            }
            for off in ["off", "Off\n"] {
                assert_eq!(debug_layer_requested(off, debug_build), Some(false));
            }
            for unrecognized in ["1", "true", "o n", "typo"] {
                assert_eq!(debug_layer_requested(unrecognized, debug_build), None);
            }
        }
    }
}
