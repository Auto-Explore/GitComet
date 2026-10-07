//! Mode orchestration: runs the parsed mode and returns the process exit code.

#[cfg(any(test, feature = "ui-gpui-runtime"))]
use crate::cli;
use crate::cli::{AppMode, exit_code};
#[cfg(feature = "ui-gpui-runtime")]
use crate::git_root::is_git_root_marker;
#[cfg(all(target_os = "linux", feature = "ui-gpui-runtime"))]
use crate::linux_wayland_fallback::maybe_relaunch_with_linux_x11_fallback;
#[cfg(feature = "ui-gpui-runtime")]
use crate::{browser_instance, crashlog};
use crate::{difftool_mode, extract_fixtures_mode, mergetool_mode, setup_mode};
#[cfg(feature = "ui-gpui-runtime")]
use gitcomet_core::identity;
use gitcomet_core::process::select_git_executable_path;

use std::io::{self, Write};

trait AppRunResult {
    fn stdout(&self) -> &str;
    fn stderr(&self) -> &str {
        ""
    }
    fn exit_code(&self) -> i32;
}

impl AppRunResult for difftool_mode::DifftoolRunResult {
    fn stdout(&self) -> &str {
        &self.stdout
    }

    fn stderr(&self) -> &str {
        &self.stderr
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl AppRunResult for mergetool_mode::MergetoolRunResult {
    fn stdout(&self) -> &str {
        &self.stdout
    }

    fn stderr(&self) -> &str {
        &self.stderr
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl AppRunResult for extract_fixtures_mode::ExtractMergeFixturesRunResult {
    fn stdout(&self) -> &str {
        &self.stdout
    }

    fn stderr(&self) -> &str {
        &self.stderr
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl AppRunResult for setup_mode::SetupResult {
    fn stdout(&self) -> &str {
        &self.stdout
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl AppRunResult for setup_mode::UninstallResult {
    fn stdout(&self) -> &str {
        &self.stdout
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

fn emit_result<R: AppRunResult, O: Write, E: Write>(
    result: Result<R, String>,
    stdout: &mut O,
    stderr: &mut E,
) -> i32 {
    match result {
        Ok(result) => {
            if !result.stdout().is_empty() {
                let _ = write!(stdout, "{}", result.stdout());
            }
            if !result.stderr().is_empty() {
                let _ = write!(stderr, "{}", result.stderr());
            }
            let _ = stdout.flush();
            let _ = stderr.flush();
            result.exit_code()
        }
        Err(msg) => {
            let _ = writeln!(stderr, "{msg}");
            exit_code::ERROR
        }
    }
}

fn emit<R: AppRunResult>(result: Result<R, String>) -> i32 {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    emit_result(result, &mut stdout, &mut stderr)
}

#[cfg(any(feature = "ui-gpui-runtime", test))]
fn should_launch_focused_diff_gui(
    config: &cli::DifftoolConfig,
    result: &difftool_mode::DifftoolRunResult,
) -> bool {
    config.gui && result.exit_code == exit_code::SUCCESS
}

/// Runs a parsed mode to completion and returns its exit code.
pub(crate) fn run_mode(
    mode: AppMode,
    extensions: crate::Extensions,
    repository_options: gitcomet_core::services::RepositoryOptions,
    backend_override: Option<std::sync::Arc<dyn gitcomet_core::services::GitBackend>>,
) -> i32 {
    // Validated in every mode, so a broken product fails the same way
    // whichever mode it is started in.
    #[cfg(feature = "ui-gpui-runtime")]
    let extensions = match gitcomet_extension_api::Registry::build(extensions) {
        Ok(registry) => registry,
        Err(errors) => {
            for error in errors {
                eprintln!("Extension registration failed: {error}");
            }
            return exit_code::ERROR;
        }
    };
    #[cfg(not(feature = "ui-gpui-runtime"))]
    let () = extensions;

    install_configured_git_executable_preference(&mode);
    #[cfg(feature = "ui-gpui-runtime")]
    let backend = || build_backend(&repository_options, backend_override.clone());
    #[cfg(not(feature = "ui-gpui-runtime"))]
    let _ = (&repository_options, &backend_override);

    #[cfg(all(target_os = "linux", feature = "ui-gpui-runtime"))]
    if let Some(code) = maybe_relaunch_with_linux_x11_fallback(&mode) {
        return code;
    }

    match mode {
        AppMode::Difftool(config) => {
            #[cfg(not(feature = "ui-gpui-runtime"))]
            if config.gui {
                eprintln!("GUI difftool mode is unavailable in this build.");
                return exit_code::ERROR;
            }

            let result = difftool_mode::run_difftool(&config);

            // When UI is available and --gui was requested, open a focused
            // GPUI diff window instead of printing raw text to stdout.
            #[cfg(feature = "ui-gpui-runtime")]
            if let Ok(result) = &result
                && should_launch_focused_diff_gui(&config, result)
            {
                let label_left = config
                    .label_left
                    .clone()
                    .unwrap_or_else(|| path_label(&config.local));
                let label_right = config
                    .label_right
                    .clone()
                    .unwrap_or_else(|| path_label(&config.remote));

                let gui_config = gitcomet_ui_gpui::FocusedDiffConfig {
                    label_left,
                    label_right,
                    display_path: config.display_path.clone(),
                    diff_text: result.stdout.clone(),
                };
                return gitcomet_ui_gpui::UiLaunch::new(backend())
                    .extensions(extensions)
                    .run_focused_diff(gui_config);
            }

            emit(result)
        }
        AppMode::Browser { path } => {
            #[cfg(feature = "ui-gpui-runtime")]
            {
                run_browser(path, extensions, backend())
            }

            #[cfg(not(feature = "ui-gpui-runtime"))]
            {
                let _ = path;
                eprintln!(
                    "{} UI is disabled in this build.",
                    gitcomet_core::identity::current().display_name()
                );
                exit_code::ERROR
            }
        }
        AppMode::Mergetool(config) => {
            #[cfg(not(feature = "ui-gpui-runtime"))]
            if config.gui {
                eprintln!("GUI mergetool mode is unavailable in this build.");
                return exit_code::ERROR;
            }

            // GUI mergetool must go straight into the streamed focused
            // resolver. Running the eager headless merge first duplicates
            // giant conflict payloads and defeats the streamed large-file
            // path before the window can even open.
            #[cfg(feature = "ui-gpui-runtime")]
            if config.gui {
                let gui_config = match build_focused_mergetool_gui_config(&config) {
                    Ok(gui_config) => gui_config,
                    Err(msg) => {
                        eprintln!("{msg}");
                        return exit_code::ERROR;
                    }
                };
                let backend = backend();
                return gitcomet_ui_gpui::UiLaunch::new(backend)
                    .extensions(extensions)
                    .run_focused_mergetool(gui_config);
            }

            emit(mergetool_mode::run_mergetool(&config))
        }
        AppMode::Setup { dry_run, local } => emit(setup_mode::run_setup(dry_run, local)),
        AppMode::Uninstall { dry_run, local } => emit(setup_mode::run_uninstall(dry_run, local)),
        AppMode::ExtractMergeFixtures(config) => {
            emit(extract_fixtures_mode::run_extract_merge_fixtures(&config))
        }
    }
}

#[cfg(feature = "ui-gpui-runtime")]
fn run_browser(
    path: Option<std::path::PathBuf>,
    extensions: gitcomet_extension_api::Registry,
    backend: std::sync::Arc<dyn gitcomet_core::services::GitBackend>,
) -> i32 {
    let name = identity::current().display_name();

    #[cfg(target_os = "macos")]
    if maybe_relaunch_browser_from_macos_app_bundle() {
        return exit_code::SUCCESS;
    }

    let path = browser_instance::normalize_browser_path(path);
    let browser_open_target = gitcomet_state::session::load()
        .browser_open_target
        .as_deref()
        .and_then(gitcomet_ui_gpui::BrowserOpenTarget::from_key)
        .unwrap_or_default();
    let initial_browser_request = gitcomet_ui_gpui::BrowserOpenRequest {
        path: path.clone(),
        target: browser_open_target,
    };
    let mut browser_instance =
        match browser_instance::start_or_forward(initial_browser_request.clone()) {
            Ok(browser_instance::StartResult::Forwarded) => return exit_code::SUCCESS,
            Ok(browser_instance::StartResult::Primary(primary)) => Some(primary),
            Err(err) => {
                eprintln!(
                    "Could not initialize the {name} browser-instance broker; \
                     launching this process independently: {err}"
                );
                None
            }
        };
    let browser_requests = browser_instance
        .as_mut()
        .and_then(browser_instance::PrimaryBrowserInstance::take_requests);

    // Windows open repositories only once `git --version` has answered. Ask
    // now, while GPUI and the first window start, rather than after that
    // window's view is built; windows adopt the result (`runtime_probe`).
    if let Some(probe) = gitcomet_core::process::begin_git_runtime_probe(false) {
        // A thread that fails to spawn drops the probe unrun, which releases
        // the claim, so the windows then probe for themselves.
        let _ = std::thread::Builder::new()
            .name("git-probe".into())
            .spawn(move || {
                let _ = probe.run();
            });
    }

    let startup_crash_report = crashlog::take_startup_report();
    if let Some(report) = startup_crash_report.as_ref() {
        // Keep the recovery path visible even if WSLg cannot create
        // a window for the in-app notification.
        print_startup_crash_report_hint(report);
    }
    if let Err(err) = crashlog::begin_session() {
        eprintln!("Failed to create {name} crash-recovery marker before UI launch: {err}");
    }
    let startup_report = startup_crash_report.map(|report| gitcomet_ui_gpui::StartupCrashReport {
        issue_url: report.issue_url,
        summary: report.summary,
        crash_log_path: report.crash_log_path,
    });
    let run_result = gitcomet_ui_gpui::UiLaunch::new(backend)
        .extensions(extensions)
        .initial_request(initial_browser_request)
        .startup_crash_report(startup_report)
        .on_shutdown(move || {
            if let Err(err) = crashlog::finish_session() {
                eprintln!(
                    "Failed to clear {} crash-recovery marker during graceful shutdown: {err}",
                    identity::current().display_name()
                );
            }
        })
        .browser_requests(browser_requests)
        .run();
    match run_result {
        Ok(
            gitcomet_ui_gpui::UiRunOutcome::CleanShutdown
            | gitcomet_ui_gpui::UiRunOutcome::UnexpectedEventLoopExit,
        ) => {
            if let Err(err) = crashlog::finish_session() {
                eprintln!(
                    "Failed to clear {name} crash-recovery marker after clean shutdown: {err}"
                );
            }
            exit_code::SUCCESS
        }
        Err(err) => {
            if let Err(record_err) =
                crashlog::record_session_failure("main GPUI window launch", &err.to_string())
            {
                eprintln!("Failed to preserve {name} UI launch failure data: {record_err}");
            }
            eprintln!("Failed to launch GPUI browser UI: {err}");
            exit_code::ERROR
        }
    }
}

fn mode_uses_configured_git_executable_preference(mode: &AppMode) -> bool {
    // The persisted custom Git executable is a browser-window preference.
    // Git-invoked command modes intentionally keep using `git` from PATH so
    // they track the invoking Git installation rather than browser settings.
    matches!(mode, AppMode::Browser { .. })
}

fn install_configured_git_executable_preference(mode: &AppMode) {
    if !mode_uses_configured_git_executable_preference(mode) {
        return;
    }

    let session = gitcomet_state::session::load();
    let _ = select_git_executable_path(session.git_executable_path);
}

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
const MACOS_BUNDLE_RELAUNCH_ENV: &str = "GITCOMET_SKIP_APP_BUNDLE_RELAUNCH";
#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
const GITCOMET_APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/gitcomet-512.png");

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(all(feature = "ui-gpui-runtime", any(target_os = "macos", all(test, unix))))]
fn resolve_executable_path_for_bundle_detection(path: &std::path::Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(all(feature = "ui-gpui-runtime", any(target_os = "macos", test)))]
fn is_macos_app_bundle_executable(path: &std::path::Path) -> bool {
    path.to_string_lossy().contains(".app/Contents/MacOS/")
}

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
fn macos_user_app_bundle_path_with_home(home: Option<&std::path::Path>) -> std::path::PathBuf {
    let name = identity::current().display_name();
    let base_dir = home
        .map(|value| value.join("Library/Application Support").join(name))
        .unwrap_or_else(|| std::env::temp_dir().join(name));
    base_dir.join(format!("{name}.app"))
}

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
fn macos_user_app_bundle_path() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    macos_user_app_bundle_path_with_home(home.as_deref())
}

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
fn candidate_macos_app_bundle_paths(resolved_exe: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::with_capacity(2);
    if let Some(bin_dir) = resolved_exe.parent() {
        candidates.push(bin_dir.join(format!("{}.app", identity::current().display_name())));
    }

    let fallback = macos_user_app_bundle_path();
    if !candidates.iter().any(|candidate| candidate == &fallback) {
        candidates.push(fallback);
    }

    candidates
}

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
fn maybe_relaunch_browser_from_macos_app_bundle() -> bool {
    if std::env::var_os(MACOS_BUNDLE_RELAUNCH_ENV).is_some() {
        return false;
    }

    let Ok(current_exe) = std::env::current_exe() else {
        return false;
    };
    let resolved_exe = resolve_executable_path_for_bundle_detection(&current_exe);
    if is_macos_app_bundle_executable(&resolved_exe) {
        return false;
    }

    let mut prep_errors = Vec::new();
    let mut app_exe = None;
    for app_bundle in candidate_macos_app_bundle_paths(&resolved_exe) {
        match ensure_macos_dev_app_bundle(&resolved_exe, &app_bundle) {
            Ok(path) => {
                app_exe = Some(path);
                break;
            }
            Err(err) => {
                prep_errors.push(format!("{}: {err}", app_bundle.display()));
            }
        }
    }

    let Some(app_exe) = app_exe else {
        let details = if prep_errors.is_empty() {
            "no app bundle destination available".to_string()
        } else {
            prep_errors.join("; ")
        };
        eprintln!("Failed to prepare macOS app bundle: {details}");
        return false;
    };

    let mut relaunch = std::process::Command::new(app_exe);
    relaunch.args(std::env::args_os().skip(1));
    relaunch.env(MACOS_BUNDLE_RELAUNCH_ENV, "1");
    match relaunch.spawn() {
        Ok(_) => true,
        Err(err) => {
            eprintln!("Failed to relaunch via macOS app bundle: {err}");
            false
        }
    }
}

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
fn ad_hoc_codesign(path: &std::path::Path) -> Result<(), String> {
    let output = std::process::Command::new("codesign")
        .arg("--force")
        .arg("--sign")
        .arg("-")
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run codesign for {}: {e}", path.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = match (stdout.is_empty(), stderr.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!(": {stdout}"),
            (true, false) => format!(": {stderr}"),
            (false, false) => format!(": stdout={stdout}; stderr={stderr}"),
        };
        return Err(format!(
            "codesign returned non-zero exit status while signing {}{}",
            path.display(),
            details
        ));
    }
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
fn ensure_macos_dev_app_bundle(
    current_exe: &std::path::Path,
    app_bundle: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let contents = app_bundle.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos).map_err(|e| format!("failed to create MacOS dir: {e}"))?;
    std::fs::create_dir_all(&resources)
        .map_err(|e| format!("failed to create Resources dir: {e}"))?;

    let identity = identity::current();
    let name = identity.display_name();
    let app_exe = macos.join(identity.executable_name());
    std::fs::copy(current_exe, &app_exe)
        .map_err(|e| format!("failed to copy executable into bundle: {e}"))?;

    let icon_png = resources.join(format!("{name}.png"));
    let icon_icns = resources.join(format!("{name}.icns"));
    let icon = identity
        .branding()
        .app_icon_png
        .unwrap_or(GITCOMET_APP_ICON_PNG);
    std::fs::write(&icon_png, icon).map_err(|e| format!("failed to write icon PNG: {e}"))?;

    let icon_output = std::process::Command::new("sips")
        .arg("-s")
        .arg("format")
        .arg("icns")
        .arg(&icon_png)
        .arg("--out")
        .arg(&icon_icns)
        .output()
        .map_err(|e| format!("failed to run sips: {e}"))?;
    if !icon_output.status.success() {
        let stderr = String::from_utf8_lossy(&icon_output.stderr)
            .trim()
            .to_string();
        let stdout = String::from_utf8_lossy(&icon_output.stdout)
            .trim()
            .to_string();
        let details = match (stdout.is_empty(), stderr.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!(": {stdout}"),
            (true, false) => format!(": {stderr}"),
            (false, false) => format!(": stdout={stdout}; stderr={stderr}"),
        };
        return Err(format!(
            "sips returned non-zero exit status while generating {}{}",
            icon_icns.display(),
            details
        ));
    }
    let _ = std::fs::remove_file(icon_png);

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>{name}</string>
  <key>CFBundleExecutable</key>
  <string>{executable}</string>
  <key>CFBundleIdentifier</key>
  <string>{bundle_id}.dev</string>
  <key>CFBundleIconFile</key>
  <string>{name}.icns</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>{name}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundleVersion</key>
  <string>{version}</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
"#,
        name = xml_text(name),
        executable = xml_text(identity.executable_name()),
        bundle_id = identity.macos_bundle_id(),
        version = xml_text(identity.version()),
    );
    std::fs::write(contents.join("Info.plist"), plist)
        .map_err(|e| format!("failed to write Info.plist: {e}"))?;

    ad_hoc_codesign(app_bundle)?;

    Ok(app_exe)
}

#[cfg(feature = "ui-gpui-runtime")]
fn print_startup_crash_report_hint(report: &crashlog::StartupCrashReport) {
    eprintln!(
        "{} detected a crash from a previous run.",
        identity::current().display_name()
    );
    eprintln!(
        "Open this URL to file a prefilled crash report:\n{}",
        report.issue_url
    );
    eprintln!("Crash log: {}", report.crash_log_path.display());
}

#[cfg(feature = "ui-gpui-runtime")]
fn build_backend(
    options: &gitcomet_core::services::RepositoryOptions,
    backend_override: Option<std::sync::Arc<dyn gitcomet_core::services::GitBackend>>,
) -> std::sync::Arc<dyn gitcomet_core::services::GitBackend> {
    #[cfg(feature = "gix")]
    let backend: std::sync::Arc<dyn gitcomet_core::services::GitBackend> =
        std::sync::Arc::new(gitcomet_git_gix::GixBackend);
    #[cfg(not(feature = "gix"))]
    let backend: std::sync::Arc<dyn gitcomet_core::services::GitBackend> =
        std::sync::Arc::new(gitcomet_core::services::UnavailableGitBackend);
    let backend = backend_override.unwrap_or(backend);
    if options.is_default() {
        backend
    } else {
        std::sync::Arc::new(gitcomet_core::services::ConfiguredBackend::new(
            backend,
            options.clone(),
        ))
    }
}

#[cfg(feature = "ui-gpui-runtime")]
fn build_focused_mergetool_gui_config(
    config: &cli::MergetoolConfig,
) -> Result<gitcomet_ui_gpui::FocusedMergetoolConfig, String> {
    let Some(repo_path) = resolve_mergetool_repo_path(&config.merged) else {
        return Err(format!(
            "Failed to locate repository root for merged path {}",
            config.merged.display()
        ));
    };

    Ok(gitcomet_ui_gpui::FocusedMergetoolConfig {
        repo_path,
        conflicted_file_path: config.merged.clone(),
        label_local: config
            .label_local
            .clone()
            .unwrap_or_else(|| path_label(&config.local)),
        label_remote: config
            .label_remote
            .clone()
            .unwrap_or_else(|| path_label(&config.remote)),
        label_base: config.label_base.clone().unwrap_or_else(|| {
            config
                .base
                .as_ref()
                .map(|path| path_label(path))
                .unwrap_or_else(|| "empty tree".to_string())
        }),
    })
}

/// Extract a filename label from a path.
#[cfg(feature = "ui-gpui-runtime")]
fn path_label(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{path:?}"))
}

#[cfg(feature = "ui-gpui-runtime")]
fn resolve_mergetool_repo_path(merged_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let absolute_merged_path = if merged_path.is_absolute() {
        merged_path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(merged_path)
    };
    let absolute_merged_path = absolute_merged_path
        .canonicalize()
        .unwrap_or(absolute_merged_path);

    let mut cursor = if absolute_merged_path.is_dir() {
        absolute_merged_path.as_path()
    } else {
        absolute_merged_path.parent()?
    };

    loop {
        let dot_git = cursor.join(".git");
        if is_git_root_marker(&dot_git) {
            return Some(cursor.to_path_buf());
        }

        cursor = cursor.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::merge::{ConflictStyle, DEFAULT_MARKER_SIZE, DiffAlgorithm};
    #[cfg(feature = "ui-gpui-runtime")]
    use std::fs;
    use std::io::{self, Write};

    #[derive(Default)]
    struct RecordingWriter {
        bytes: Vec<u8>,
        flush_count: usize,
    }

    impl RecordingWriter {
        fn as_text(&self) -> &str {
            std::str::from_utf8(&self.bytes).expect("writer should contain valid utf-8 in tests")
        }
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flush_count += 1;
            Ok(())
        }
    }

    #[derive(Clone)]
    struct TestRunResult {
        stdout: String,
        stderr: String,
        exit_code: i32,
    }

    impl AppRunResult for TestRunResult {
        fn stdout(&self) -> &str {
            &self.stdout
        }

        fn stderr(&self) -> &str {
            &self.stderr
        }

        fn exit_code(&self) -> i32 {
            self.exit_code
        }
    }

    #[cfg(feature = "ui-gpui-runtime")]
    fn mergetool_config(
        repo_root: &std::path::Path,
        merged: std::path::PathBuf,
        base: Option<std::path::PathBuf>,
    ) -> cli::MergetoolConfig {
        cli::MergetoolConfig {
            merged,
            local: repo_root.join("local.txt"),
            remote: repo_root.join("remote.txt"),
            base,
            label_base: None,
            label_local: None,
            label_remote: None,
            conflict_style: ConflictStyle::Merge,
            diff_algorithm: DiffAlgorithm::Myers,
            marker_size: DEFAULT_MARKER_SIZE,
            auto: false,
            gui: true,
        }
    }

    #[cfg(feature = "ui-gpui-runtime")]
    fn create_git_root_marker(repo_root: &std::path::Path) {
        let git_dir = repo_root.join(".git");
        fs::create_dir_all(&git_dir).expect("create git dir marker");
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("write git HEAD marker");
    }

    #[test]
    fn focused_diff_gui_launches_for_success_even_when_diff_output_is_empty() {
        let config = cli::DifftoolConfig {
            local: std::path::PathBuf::from("left.txt"),
            remote: std::path::PathBuf::from("right.txt"),
            display_path: None,
            label_left: None,
            label_right: None,
            gui: true,
        };
        let result = difftool_mode::DifftoolRunResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: exit_code::SUCCESS,
        };

        assert!(should_launch_focused_diff_gui(&config, &result));
    }

    #[test]
    fn focused_diff_gui_does_not_launch_when_not_requested() {
        let config = cli::DifftoolConfig {
            local: std::path::PathBuf::from("left.txt"),
            remote: std::path::PathBuf::from("right.txt"),
            display_path: None,
            label_left: None,
            label_right: None,
            gui: false,
        };
        let result = difftool_mode::DifftoolRunResult {
            stdout: "diff --git".to_string(),
            stderr: String::new(),
            exit_code: exit_code::SUCCESS,
        };

        assert!(!should_launch_focused_diff_gui(&config, &result));
    }

    #[test]
    fn focused_diff_gui_does_not_launch_on_error_exit() {
        let config = cli::DifftoolConfig {
            local: std::path::PathBuf::from("left.txt"),
            remote: std::path::PathBuf::from("right.txt"),
            display_path: None,
            label_left: None,
            label_right: None,
            gui: true,
        };
        let result = difftool_mode::DifftoolRunResult {
            stdout: "diff --git".to_string(),
            stderr: "error".to_string(),
            exit_code: exit_code::ERROR,
        };

        assert!(!should_launch_focused_diff_gui(&config, &result));
    }

    #[test]
    fn configured_git_preference_is_intentionally_browser_only() {
        assert!(mode_uses_configured_git_executable_preference(
            &AppMode::Browser { path: None }
        ));
        assert!(!mode_uses_configured_git_executable_preference(
            &AppMode::Difftool(cli::DifftoolConfig {
                local: std::path::PathBuf::from("left.txt"),
                remote: std::path::PathBuf::from("right.txt"),
                display_path: None,
                label_left: None,
                label_right: None,
                gui: false,
            })
        ));
        assert!(!mode_uses_configured_git_executable_preference(
            &AppMode::Mergetool(cli::MergetoolConfig {
                merged: std::path::PathBuf::from("merged.txt"),
                local: std::path::PathBuf::from("local.txt"),
                remote: std::path::PathBuf::from("remote.txt"),
                base: Some(std::path::PathBuf::from("base.txt")),
                label_base: None,
                label_local: None,
                label_remote: None,
                conflict_style: ConflictStyle::Merge,
                diff_algorithm: DiffAlgorithm::Myers,
                marker_size: DEFAULT_MARKER_SIZE,
                auto: false,
                gui: false,
            })
        ));
        assert!(!mode_uses_configured_git_executable_preference(
            &AppMode::Setup {
                dry_run: false,
                local: false,
            }
        ));
        assert!(!mode_uses_configured_git_executable_preference(
            &AppMode::Uninstall {
                dry_run: false,
                local: false,
            }
        ));
        assert!(!mode_uses_configured_git_executable_preference(
            &AppMode::ExtractMergeFixtures(cli::ExtractMergeFixturesConfig {
                repo: std::path::PathBuf::from("/tmp/repo"),
                output_dir: std::path::PathBuf::from("/tmp/out"),
                max_merges: 10,
                max_files_per_merge: 5,
            })
        ));
    }

    #[test]
    #[cfg(feature = "ui-gpui-runtime")]
    fn build_focused_mergetool_gui_config_uses_default_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("repo");
        let merged = repo_root.join("src/conflicted.txt");
        let base = repo_root.join("base.txt");

        create_git_root_marker(&repo_root);
        fs::create_dir_all(merged.parent().unwrap()).unwrap();

        let config = mergetool_config(&repo_root, merged.clone(), Some(base));
        let gui_config = build_focused_mergetool_gui_config(&config).expect("gui config");

        assert_eq!(gui_config.repo_path, repo_root);
        assert_eq!(gui_config.conflicted_file_path, merged);
        assert_eq!(gui_config.label_local, "local.txt");
        assert_eq!(gui_config.label_remote, "remote.txt");
        assert_eq!(gui_config.label_base, "base.txt");
    }

    #[test]
    #[cfg(feature = "ui-gpui-runtime")]
    fn build_focused_mergetool_gui_config_uses_empty_tree_without_base() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("repo");
        let merged = repo_root.join("src/conflicted.txt");

        create_git_root_marker(&repo_root);
        fs::create_dir_all(merged.parent().unwrap()).unwrap();

        let config = mergetool_config(&repo_root, merged, None);
        let gui_config = build_focused_mergetool_gui_config(&config).expect("gui config");

        assert_eq!(gui_config.label_base, "empty tree");
    }

    #[test]
    #[cfg(feature = "ui-gpui-runtime")]
    fn build_focused_mergetool_gui_config_errors_without_repo_root() {
        let tmp = tempfile::Builder::new()
            .prefix("gitcomet-mergetool-no-repo-")
            .tempdir()
            .expect("create temp dir outside repo");
        let merged = tmp.path().join("outside-repo/merged.txt");

        fs::create_dir_all(merged.parent().unwrap()).unwrap();

        let config = mergetool_config(tmp.path(), merged.clone(), None);
        let err =
            build_focused_mergetool_gui_config(&config).expect_err("expected missing repo root");

        assert!(err.contains("Failed to locate repository root"));
        assert!(err.contains(&merged.display().to_string()));
    }

    #[test]
    #[cfg(feature = "ui-gpui-runtime")]
    fn detects_macos_app_bundle_executable_paths() {
        assert!(is_macos_app_bundle_executable(std::path::Path::new(
            "/tmp/GitComet.app/Contents/MacOS/gitcomet"
        )));
        assert!(!is_macos_app_bundle_executable(std::path::Path::new(
            "/opt/homebrew/bin/gitcomet"
        )));
    }

    #[test]
    #[cfg(all(feature = "ui-gpui-runtime", unix))]
    fn canonicalized_symlink_resolves_to_macos_app_bundle_executable() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let app_exe = temp.path().join("GitComet.app/Contents/MacOS/gitcomet");
        fs::create_dir_all(app_exe.parent().unwrap()).unwrap();
        fs::write(&app_exe, b"#!/bin/sh\n").unwrap();

        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let symlink_path = bin_dir.join("gitcomet");
        symlink(&app_exe, &symlink_path).unwrap();

        assert!(!is_macos_app_bundle_executable(&symlink_path));

        let resolved = resolve_executable_path_for_bundle_detection(&symlink_path);
        let expected = app_exe.canonicalize().unwrap();
        assert_eq!(resolved, expected);
        assert!(is_macos_app_bundle_executable(&resolved));
    }

    #[test]
    #[cfg(all(feature = "ui-gpui-runtime", target_os = "macos"))]
    fn macos_user_app_bundle_path_uses_home_directory_when_available() {
        let home = std::path::Path::new("/Users/example");
        let bundle = macos_user_app_bundle_path_with_home(Some(home));
        assert_eq!(
            bundle,
            std::path::PathBuf::from(
                "/Users/example/Library/Application Support/GitComet/GitComet.app"
            )
        );
    }

    #[test]
    #[cfg(all(feature = "ui-gpui-runtime", target_os = "macos"))]
    fn candidate_macos_app_bundle_paths_include_executable_dir_and_fallback() {
        let exe = std::path::Path::new("/opt/homebrew/bin/gitcomet");
        let candidates = candidate_macos_app_bundle_paths(exe);

        assert_eq!(
            candidates.first(),
            Some(&std::path::PathBuf::from("/opt/homebrew/bin/GitComet.app"))
        );
        assert_eq!(candidates.last(), Some(&macos_user_app_bundle_path()));
    }

    #[test]
    #[cfg(all(feature = "ui-gpui-runtime", target_os = "macos"))]
    fn ensure_macos_dev_app_bundle_replaces_stale_signature_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let app_bundle = temp.path().join("GitComet.app");
        let stale_signature = app_bundle.join("Contents/_CodeSignature/CodeResources");

        fs::create_dir_all(stale_signature.parent().unwrap()).unwrap();
        fs::write(&stale_signature, b"stale-signature").unwrap();

        let current_exe = std::env::current_exe().expect("current test executable path");
        let app_exe =
            ensure_macos_dev_app_bundle(&current_exe, &app_bundle).expect("prepare app bundle");

        assert_eq!(
            app_exe,
            app_bundle.join(std::path::Path::new("Contents/MacOS/gitcomet"))
        );

        let status = std::process::Command::new("codesign")
            .arg("--verify")
            .arg("--strict")
            .arg("--verbose=2")
            .arg(&app_bundle)
            .status()
            .expect("run codesign verification");
        assert!(
            status.success(),
            "expected rebuilt app bundle to pass codesign verification"
        );
    }

    #[test]
    fn emit_result_writes_stdout_stderr_and_flushes() {
        let result = Ok(TestRunResult {
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            exit_code: 7,
        });
        let mut stdout = RecordingWriter::default();
        let mut stderr = RecordingWriter::default();

        let code = emit_result(result, &mut stdout, &mut stderr);

        assert_eq!(code, 7);
        assert_eq!(stdout.as_text(), "out");
        assert_eq!(stderr.as_text(), "err");
        assert_eq!(stdout.flush_count, 1);
        assert_eq!(stderr.flush_count, 1);
    }

    #[test]
    fn emit_result_writes_error_message_to_stderr() {
        let mut stdout = RecordingWriter::default();
        let mut stderr = RecordingWriter::default();

        let code =
            emit_result::<TestRunResult, _, _>(Err("boom".to_string()), &mut stdout, &mut stderr);

        assert_eq!(code, exit_code::ERROR);
        assert_eq!(stdout.as_text(), "");
        assert_eq!(stderr.as_text(), "boom\n");
        assert_eq!(stdout.flush_count, 0);
        assert_eq!(stderr.flush_count, 0);
    }
}
