//! Windows rendering defaults, installed before application or renderer startup.
//!
//! Any existing environment value, including an empty value, is an explicit
//! override. In particular, `GPUI_GPU_EXPERIMENTS=` disables these experiments.
//! Backend selection remains with the saved preference and GPUI's normal
//! Automatic DX12-to-DX11 fallback.

const DEFAULTS: [(&str, &str); 3] = [
    (
        "GPUI_GPU_EXPERIMENTS",
        "partial-redraw,retained-copy,cropped-paths",
    ),
    ("GPUI_WINDOWS_INPUT_RATE_KEEPALIVE", "off"),
    ("GPUI_WINDOWS_DX12_PRESENT_MODE", "immediate"),
];

fn missing_defaults(
    mut is_present: impl FnMut(&str) -> bool,
) -> impl Iterator<Item = (&'static str, &'static str)> {
    DEFAULTS
        .into_iter()
        .filter(move |(key, _)| !is_present(key))
}

pub(super) fn install() {
    for (key, value) in missing_defaults(|key| std::env::var_os(key).is_some()) {
        // main calls this before App::new, logging, executors, or renderers.
        // The Windows-only helper contains the edition-2024 unsafe boundary.
        gitcomet_win32_window_utils::set_process_environment_default(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;

    fn planned_defaults(values: &[(&str, OsString)]) -> Vec<(&'static str, &'static str)> {
        let values: HashMap<_, _> = values.iter().map(|(key, value)| (*key, value)).collect();
        missing_defaults(|key| values.contains_key(key)).collect()
    }

    #[test]
    fn absent_environment_enables_the_measured_windows_policy() {
        let defaults = planned_defaults(&[]);
        let experiments = defaults
            .iter()
            .find(|(key, _)| *key == "GPUI_GPU_EXPERIMENTS")
            .unwrap()
            .1;
        assert_eq!(
            experiments.split(',').collect::<Vec<_>>(),
            ["partial-redraw", "retained-copy", "cropped-paths"]
        );
        assert!(defaults.contains(&("GPUI_WINDOWS_INPUT_RATE_KEEPALIVE", "off")));
        assert!(defaults.contains(&("GPUI_WINDOWS_DX12_PRESENT_MODE", "immediate")));
    }

    #[test]
    fn each_explicit_setting_preserves_the_other_missing_defaults() {
        for key in DEFAULTS.map(|(key, _)| key) {
            let defaults = planned_defaults(&[(key, OsString::from("explicit"))]);
            assert_eq!(defaults.len(), 2);
            assert!(defaults.iter().all(|(default_key, _)| *default_key != key));
        }
    }

    #[test]
    fn empty_experiments_is_an_explicit_opt_out() {
        let defaults = planned_defaults(&[("GPUI_GPU_EXPERIMENTS", OsString::new())]);
        assert!(
            defaults
                .iter()
                .all(|(key, _)| *key != "GPUI_GPU_EXPERIMENTS")
        );
        assert_eq!(defaults.len(), 2);
    }

    #[test]
    fn all_explicit_overrides_leave_no_startup_changes() {
        let values = [
            ("GPUI_GPU_EXPERIMENTS", OsString::new()),
            ("GPUI_WINDOWS_INPUT_RATE_KEEPALIVE", OsString::from("on")),
            ("GPUI_WINDOWS_DX12_PRESENT_MODE", OsString::from("fifo")),
        ];
        assert!(planned_defaults(&values).is_empty());
    }

    #[test]
    fn defaults_do_not_select_a_backend_or_enable_diagnostic_observers() {
        assert_eq!(DEFAULTS.len(), 3);
        for key in [
            "GPUI_WINDOWS_RENDERER",
            "GPUI_GPU_TIMINGS",
            "GPUI_GPU_DIAGNOSTIC_ABLATIONS",
            "GPUI_GPU_DIAGNOSTIC_SCENE_DIFF",
            "GITCOMET_GPU_GRAPH_QUADS",
            "GITCOMET_UI_PROBE",
        ] {
            assert!(DEFAULTS.iter().all(|(default_key, _)| *default_key != key));
        }
    }
}
