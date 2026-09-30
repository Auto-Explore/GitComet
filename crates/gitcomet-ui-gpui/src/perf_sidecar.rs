use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

const RUNNER_CLASS_ENV: &str = "GITCOMET_PERF_RUNNER_CLASS";
/// Set by the suite drivers so every artifact of one run can be matched up.
const RUN_ID_ENV: &str = "GITCOMET_PERF_RUN_ID";
/// The Cargo profile the measuring binary was built with, set by the drivers.
const CARGO_PROFILE_ENV: &str = "GITCOMET_PERF_CARGO_PROFILE";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PerfSidecarReport {
    pub bench: String,
    #[serde(default, skip_serializing_if = "PerfSidecarRunner::is_empty")]
    pub runner: PerfSidecarRunner,
    /// Absent in sidecars written before measurement labels existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<PerfSidecarMeasurement>,
    #[serde(default)]
    pub metrics: Map<String, Value>,
}

/// What a result covers. Several benchmark names (`frame_timing`, `display`,
/// `keyboard`) predate these labels and do not measure native frames.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementKind {
    /// Git backend or filesystem work, without view state or rendering.
    BackendOperation,
    /// View-model and row preparation on the benchmark thread; nothing is
    /// laid out, painted, submitted, or presented.
    PreparedRowWork,
    /// Layout and paint in a GPUI test-platform window: no GPU, compositor,
    /// or platform event loop.
    GpuiTestPlatformDraw,
    /// A real application process with a native window.
    LiveApplication,
}

impl MeasurementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BackendOperation => "backend_operation",
            Self::PreparedRowWork => "prepared_row_work",
            Self::GpuiTestPlatformDraw => "gpui_test_platform_draw",
            Self::LiveApplication => "live_application",
        }
    }
}

/// Classifies a result by its benchmark group (the first path segment).
pub fn measurement_kind_for_bench(bench: &str) -> MeasurementKind {
    let group = bench.split('/').next().unwrap_or(bench);
    match group {
        "git_ops" | "fs_event" | "real_repo" | "idle" => MeasurementKind::BackendOperation,
        "picker_prompt"
        | "status_truncation"
        | "markdown_preview_render_single"
        | "markdown_preview_render_diff"
        | "markdown_preview_scroll"
        | "diff_open_markdown_preview_first_window" => MeasurementKind::GpuiTestPlatformDraw,
        "app_launch" => MeasurementKind::LiveApplication,
        _ => MeasurementKind::PreparedRowWork,
    }
}

/// Identifies the run, build, and allocator a result came from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PerfSidecarMeasurement {
    pub kind: MeasurementKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_profile: Option<String>,
    pub debug_assertions: bool,
    pub allocator: PerfSidecarAllocator,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PerfSidecarAllocator {
    /// `MIMALLOC_*` settings in effect; unset options use mimalloc defaults.
    #[serde(default)]
    pub mimalloc_env: BTreeMap<String, String>,
}

impl PerfSidecarMeasurement {
    pub fn current(bench: &str) -> Self {
        Self {
            kind: measurement_kind_for_bench(bench),
            run_id: env_string(RUN_ID_ENV),
            cargo_profile: env_string(CARGO_PROFILE_ENV),
            debug_assertions: cfg!(debug_assertions),
            allocator: PerfSidecarAllocator {
                mimalloc_env: env::vars()
                    .filter(|(key, _)| key.starts_with("MIMALLOC_"))
                    .collect(),
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PerfSidecarRunner {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u64>,
}

impl PerfSidecarRunner {
    fn is_empty(&self) -> bool {
        self.runner_class.is_none()
            && self.hostname.is_none()
            && self.os.is_none()
            && self.arch.is_none()
            && self.cpu_count.is_none()
    }
}

impl PerfSidecarReport {
    pub fn new(bench: impl Into<String>, metrics: Map<String, Value>) -> Self {
        let bench = bench.into();
        Self {
            measurement: Some(PerfSidecarMeasurement::current(&bench)),
            bench,
            runner: current_runner_metadata(),
            metrics,
        }
    }
}

pub fn current_runner_metadata() -> PerfSidecarRunner {
    let cpu_count = thread::available_parallelism()
        .ok()
        .and_then(|count| u64::try_from(count.get()).ok());
    build_runner_metadata(
        env_string(RUNNER_CLASS_ENV),
        current_hostname(),
        Some(std::env::consts::OS.to_string()),
        Some(std::env::consts::ARCH.to_string()),
        cpu_count,
    )
}

pub fn criterion_output_root() -> PathBuf {
    if let Some(root) = env_string("GITCOMET_PERF_CRITERION_ROOT") {
        return PathBuf::from(root);
    }

    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            env::current_exe()
                .ok()
                .and_then(|path| path.parent()?.parent()?.parent().map(Path::to_path_buf))
        })
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("target")
        })
        .join("criterion")
}

pub fn criterion_sidecar_path(criterion_root: &Path, bench: &str) -> PathBuf {
    criterion_root.join(bench).join("new").join("sidecar.json")
}

pub fn write_criterion_sidecar(report: &PerfSidecarReport) -> Result<PathBuf, String> {
    let path = criterion_sidecar_path(&criterion_output_root(), &report.bench);
    write_sidecar(report, &path)?;
    Ok(path)
}

pub fn write_sidecar(report: &PerfSidecarReport, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create sidecar directory {}: {err}",
                parent.display()
            )
        })?;
    }

    let mut content = serde_json::to_vec_pretty(report).map_err(|err| {
        format!(
            "failed to serialize sidecar payload for {}: {err}",
            report.bench
        )
    })?;
    content.push(b'\n');
    fs::write(path, content)
        .map_err(|err| format!("failed to write sidecar {}: {err}", path.display()))
}

pub fn read_sidecar(path: &Path) -> Result<PerfSidecarReport, String> {
    let json = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&json).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn env_string(key: &str) -> Option<String> {
    let value = env::var(key).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn current_hostname() -> Option<String> {
    env_string("HOSTNAME")
        .or_else(|| env_string("COMPUTERNAME"))
        .or_else(|| read_hostname_file("/etc/hostname"))
        .or_else(|| read_hostname_file("/proc/sys/kernel/hostname"))
}

fn read_hostname_file(path: &str) -> Option<String> {
    let value = fs::read_to_string(path).ok()?;
    normalize_string(value)
}

fn build_runner_metadata(
    runner_class: Option<String>,
    hostname: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    cpu_count: Option<u64>,
) -> PerfSidecarRunner {
    PerfSidecarRunner {
        runner_class: normalize_option_string(runner_class),
        hostname: normalize_option_string(hostname),
        os: normalize_option_string(os),
        arch: normalize_option_string(arch),
        cpu_count,
    }
}

fn normalize_option_string(value: Option<String>) -> Option<String> {
    value.and_then(normalize_string)
}

fn normalize_string(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn criterion_sidecar_path_uses_bench_new_sidecar_shape() {
        let path = criterion_sidecar_path(
            Path::new("/tmp/criterion"),
            "diff_open_patch_first_window/200",
        );
        assert_eq!(
            path,
            PathBuf::from("/tmp/criterion/diff_open_patch_first_window/200/new/sidecar.json")
        );
    }

    #[test]
    fn write_and_read_sidecar_round_trip() {
        let temp_dir = TempDir::new().expect("tempdir");
        let path = criterion_sidecar_path(temp_dir.path(), "diff_open_patch_first_window/200");
        let mut metrics = Map::new();
        metrics.insert("rows_requested".to_string(), json!(200));
        metrics.insert("rows_materialized".to_string(), json!(224));
        let report = PerfSidecarReport::new("diff_open_patch_first_window/200", metrics);

        write_sidecar(&report, &path).expect("write sidecar");
        let round_trip = read_sidecar(&path).expect("read sidecar");

        assert_eq!(round_trip, report);
    }

    #[test]
    fn build_runner_metadata_normalizes_expected_fields() {
        let runner = build_runner_metadata(
            Some(" workstation-linux ".to_string()),
            Some(" linuxdesktop ".to_string()),
            Some(" linux ".to_string()),
            Some(" x86_64 ".to_string()),
            Some(32),
        );

        assert_eq!(runner.runner_class.as_deref(), Some("workstation-linux"));
        assert_eq!(runner.hostname.as_deref(), Some("linuxdesktop"));
        assert_eq!(runner.os.as_deref(), Some("linux"));
        assert_eq!(runner.arch.as_deref(), Some("x86_64"));
        assert_eq!(runner.cpu_count, Some(32));
    }

    #[test]
    fn measurement_kind_labels_what_each_group_times() {
        assert_eq!(
            measurement_kind_for_bench("fs_event/single_file_save_to_status_update"),
            MeasurementKind::BackendOperation
        );
        assert_eq!(
            measurement_kind_for_bench("frame_timing/continuous_scroll_history_list"),
            MeasurementKind::PreparedRowWork
        );
        assert_eq!(
            measurement_kind_for_bench("picker_prompt/branch_filter"),
            MeasurementKind::GpuiTestPlatformDraw
        );
        assert_eq!(
            measurement_kind_for_bench("app_launch/cold_single_repo"),
            MeasurementKind::LiveApplication
        );
    }

    #[test]
    fn sidecars_without_measurement_labels_still_parse() {
        let report: PerfSidecarReport =
            serde_json::from_str(r#"{"bench":"idle/cpu_usage_single_repo_60s","metrics":{}}"#)
                .expect("parse legacy sidecar");
        assert_eq!(report.measurement, None);
    }

    #[test]
    fn perf_sidecar_report_new_attaches_current_runner_metadata() {
        let report = PerfSidecarReport::new("diff_open_patch_first_window/200", Map::new());

        assert_eq!(report.runner.os.as_deref(), Some(std::env::consts::OS));
        assert_eq!(report.runner.arch.as_deref(), Some(std::env::consts::ARCH));
        assert!(!report.runner.is_empty());
        let measurement = report.measurement.expect("measurement label");
        assert_eq!(measurement.kind, MeasurementKind::PreparedRowWork);
        assert_eq!(measurement.debug_assertions, cfg!(debug_assertions));
    }
}
