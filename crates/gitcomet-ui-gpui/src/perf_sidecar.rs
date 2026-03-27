use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

const RUNNER_CLASS_ENV: &str = "GITCOMET_PERF_RUNNER_CLASS";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PerfSidecarReport {
    pub bench: String,
    #[serde(default, skip_serializing_if = "PerfSidecarRunner::is_empty")]
    pub runner: PerfSidecarRunner,
    #[serde(default)]
    pub metrics: Map<String, Value>,
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
        Self {
            bench: bench.into(),
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
    fn perf_sidecar_report_new_attaches_current_runner_metadata() {
        let report = PerfSidecarReport::new("diff_open_patch_first_window/200", Map::new());

        assert_eq!(report.runner.os.as_deref(), Some(std::env::consts::OS));
        assert_eq!(report.runner.arch.as_deref(), Some(std::env::consts::ARCH));
        assert!(!report.runner.is_empty());
    }
}
