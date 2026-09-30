//! Cached, process-local diagnostics. Collect system details on a worker; the
//! UI supplies the renderer actually selected for each window. Readers (including
//! crash handlers) never start processes or query a graphics API.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock, mpsc};

pub const UNAVAILABLE: &str = "Unavailable";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default = "EnvironmentSnapshot::unavailable")]
pub struct EnvironmentSnapshot {
    pub app_version: String,
    pub git_version: Option<String>,
    pub system: SystemDetails,
    /// Window identities are process-local; no paths or window titles are stored.
    pub graphics: BTreeMap<u64, GraphicsDetails>,
}

impl Default for EnvironmentSnapshot {
    fn default() -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION").into(),
            git_version: None,
            system: SystemDetails::default(),
            graphics: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default = "SystemDetails::unavailable")]
pub struct SystemDetails {
    pub operating_system: Option<String>,
    pub kernel: Option<String>,
    pub architecture: Option<String>,
    pub cpu_model: Option<String>,
    pub logical_processors: Option<usize>,
    pub total_memory_bytes: Option<u64>,
}

impl Default for SystemDetails {
    fn default() -> Self {
        Self {
            operating_system: Some(os_display_name(std::env::consts::OS).into()),
            architecture: Some(std::env::consts::ARCH.into()),
            kernel: None,
            cpu_model: None,
            logical_processors: None,
            total_memory_bytes: None,
        }
    }
}

impl SystemDetails {
    fn unavailable() -> Self {
        Self {
            operating_system: None,
            kernel: None,
            architecture: None,
            cpu_model: None,
            logical_processors: None,
            total_memory_bytes: None,
        }
    }
    /// Call on a background worker. No process enumeration, utilization sampling,
    /// external diagnostic commands, or sleeps are needed for these static fields.
    pub fn collect_once() -> Self {
        static DETAILS: OnceLock<SystemDetails> = OnceLock::new();
        DETAILS
            .get_or_init(|| {
                let mut system = sysinfo::System::new();
                system.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
                system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
                let mut models = Vec::new();
                for cpu in system.cpus() {
                    if !cpu.brand().trim().is_empty() && !models.contains(&cpu.brand()) {
                        models.push(cpu.brand());
                    }
                }
                Self {
                    operating_system: sysinfo::System::long_os_version()
                        .or_else(sysinfo::System::name),
                    kernel: sysinfo::System::kernel_version(),
                    cpu_model: (!models.is_empty()).then(|| models.join(" / ")),
                    logical_processors: (!system.cpus().is_empty()).then(|| system.cpus().len()),
                    total_memory_bytes: (system.total_memory() > 0).then(|| system.total_memory()),
                    ..Self::default()
                }
            })
            .clone()
    }
}

pub fn os_display_name(os: &str) -> &str {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        "freebsd" => "FreeBSD",
        other => other,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsDetails {
    pub window_system: Option<String>,
    pub device_name: Option<String>,
    pub backend: Option<String>,
    pub driver_name: Option<String>,
    pub driver_info: Option<String>,
    /// Unavailable means GPUI did not expose GPU specs.
    pub rendering: Rendering,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Rendering {
    Hardware,
    Software,
    #[default]
    Unavailable,
}

impl Rendering {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hardware => "Hardware",
            Self::Software => "Software (CPU)",
            Self::Unavailable => UNAVAILABLE,
        }
    }
}

pub struct EnvironmentRow {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
}

pub struct EnvironmentSection {
    pub title: String,
    pub rows: Vec<EnvironmentRow>,
}

fn row(key: &'static str, label: &'static str, value: Option<&str>) -> EnvironmentRow {
    EnvironmentRow {
        key,
        label,
        value: value
            .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| UNAVAILABLE.into()),
    }
}

impl EnvironmentSnapshot {
    fn unavailable() -> Self {
        Self {
            app_version: String::new(),
            git_version: None,
            system: SystemDetails::unavailable(),
            graphics: BTreeMap::new(),
        }
    }
    /// Settings, the clipboard, UI probes, and issue bodies all use these rows.
    pub fn sections(&self) -> Vec<EnvironmentSection> {
        let system = &self.system;
        let mut sections = vec![EnvironmentSection {
            title: "System".into(),
            rows: vec![
                row(
                    "build",
                    "Build",
                    (!self.app_version.trim().is_empty())
                        .then(|| format!("GitComet v{}", self.app_version))
                        .as_deref(),
                ),
                row("git", "Git", self.git_version.as_deref()),
                row("os", "Operating system", system.operating_system.as_deref()),
                row("kernel", "Kernel", system.kernel.as_deref()),
                row(
                    "architecture",
                    "Architecture",
                    system.architecture.as_deref(),
                ),
                row("cpu", "CPU", system.cpu_model.as_deref()),
                row(
                    "processors",
                    "Logical processors",
                    system.logical_processors.map(|n| n.to_string()).as_deref(),
                ),
                row(
                    "memory",
                    "Total memory",
                    system
                        .total_memory_bytes
                        .map(|bytes| format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0))
                        .as_deref(),
                ),
            ],
        }];
        let mut configurations: Vec<(&GraphicsDetails, usize)> = Vec::new();
        for details in self.graphics.values() {
            if let Some((_, count)) = configurations
                .iter_mut()
                .find(|(existing, _)| *existing == details)
            {
                *count += 1;
            } else {
                configurations.push((details, 1));
            }
        }
        let unavailable = GraphicsDetails::default();
        if configurations.is_empty() {
            configurations.push((&unavailable, 0));
        }
        let multiple = configurations.len() > 1;
        for (index, (graphics, windows)) in configurations.into_iter().enumerate() {
            sections.push(EnvironmentSection {
                title: if multiple {
                    format!(
                        "Graphics {} ({windows} window{})",
                        index + 1,
                        if windows == 1 { "" } else { "s" }
                    )
                } else {
                    "Graphics".into()
                },
                rows: vec![
                    row(
                        "window_system",
                        "Window system",
                        graphics.window_system.as_deref(),
                    ),
                    row("gpu", "Selected GPU", graphics.device_name.as_deref()),
                    row("backend", "Graphics backend", graphics.backend.as_deref()),
                    row("rendering", "Rendering", Some(graphics.rendering.label())),
                    row("driver", "Driver", graphics.driver_name.as_deref()),
                    row(
                        "driver_info",
                        "Driver details",
                        graphics.driver_info.as_deref(),
                    ),
                ],
            });
        }
        sections
    }

    pub fn summary(&self) -> String {
        let mut text = String::new();
        for section in self.sections() {
            if !text.is_empty() {
                text.push('\n');
            }
            let _ = writeln!(text, "{}", section.title);
            for row in section.rows {
                let _ = writeln!(text, "{}: {}", row.label, row.value);
            }
        }
        text
    }
}

#[derive(Default)]
struct Cache {
    snapshot: EnvironmentSnapshot,
    subscribers: Vec<mpsc::Sender<EnvironmentSnapshot>>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub fn cached() -> EnvironmentSnapshot {
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .snapshot
        .clone()
}

/// A panic while publishing must never deadlock its own panic hook.
pub fn try_cached() -> Option<EnvironmentSnapshot> {
    cache().try_lock().ok().map(|cache| cache.snapshot.clone())
}

pub fn publish(snapshot: EnvironmentSnapshot) {
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    if cache.snapshot == snapshot {
        return;
    }
    cache.snapshot = snapshot;
    let snapshot = cache.snapshot.clone();
    cache
        .subscribers
        .retain(|subscriber| subscriber.send(snapshot.clone()).is_ok());
}

/// The initial snapshot and subsequent changes are ordered under the same lock.
/// Subscribers perform disk I/O on their own worker, never on the publishing UI.
pub fn subscribe() -> mpsc::Receiver<EnvironmentSnapshot> {
    let (sender, receiver) = mpsc::channel();
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    let _ = sender.send(cache.snapshot.clone());
    cache.subscribers.push(sender);
    receiver
}

#[cfg(test)]
mod tests;
