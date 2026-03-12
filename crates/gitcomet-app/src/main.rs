mod cli;
#[cfg(feature = "ui")]
mod crashlog;
mod difftool_mode;
mod extract_fixtures_mode;
mod mergetool_mode;
mod setup_mode;

use cli::{AppMode, exit_code};
use mimalloc::MiMalloc;
use std::io::{self, Write};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

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

fn run_and_exit<R: AppRunResult>(result: Result<R, String>) -> ! {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    std::process::exit(emit_result(result, &mut stdout, &mut stderr));
}

#[cfg(any(feature = "ui-gpui", test))]
fn should_launch_focused_diff_gui(
    config: &cli::DifftoolConfig,
    result: &difftool_mode::DifftoolRunResult,
) -> bool {
    config.gui && result.exit_code == exit_code::SUCCESS
}

#[cfg(any(feature = "ui-gpui", test))]
fn should_launch_focused_merge_gui(
    config: &cli::MergetoolConfig,
    result: &mergetool_mode::MergetoolRunResult,
) -> bool {
    config.gui && result.exit_code == exit_code::CANCELED && result.merge_result.is_some()
}

fn main() {
    let mode = match cli::parse_app_mode() {
        Ok(mode) => mode,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(exit_code::ERROR);
        }
    };

    #[cfg(feature = "ui")]
    crashlog::install();

    match mode {
        AppMode::Difftool(config) => {
            #[cfg(not(feature = "ui-gpui"))]
            if config.gui {
                eprintln!(
                    "GUI difftool mode is unavailable in this build. Rebuild with `-p gitcomet-app --features ui-gpui`."
                );
                std::process::exit(exit_code::ERROR);
            }

            let result = difftool_mode::run_difftool(&config);

            // When UI is available and --gui was requested, open a focused
            // GPUI diff window instead of printing raw text to stdout.
            #[cfg(feature = "ui-gpui")]
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
                let code = gitcomet_ui_gpui::run_focused_diff(gui_config);
                std::process::exit(code);
            }

            run_and_exit(result);
        }
        AppMode::Browser { path } => {
            #[cfg(feature = "ui")]
            {
                #[cfg(all(target_os = "macos", feature = "ui-gpui"))]
                if maybe_relaunch_browser_from_macos_app_bundle() {
                    std::process::exit(exit_code::SUCCESS);
                }

                let startup_crash_report = crashlog::take_startup_report();
                let backend = build_backend();

                if cfg!(feature = "ui-gpui") {
                    #[cfg(feature = "ui-gpui")]
                    {
                        let startup_report = startup_crash_report.clone().map(|report| {
                            gitcomet_ui_gpui::StartupCrashReport {
                                issue_url: report.issue_url,
                                summary: report.summary,
                                crash_log_path: report.crash_log_path,
                            }
                        });
                        if let Err(err) = gitcomet_ui_gpui::run_with_startup_crash_report(
                            backend.clone(),
                            path.clone(),
                            startup_report,
                        ) {
                            eprintln!("Failed to launch GPUI browser UI: {err}");
                            if let Some(report) = startup_crash_report.as_ref() {
                                print_startup_crash_report_hint(report);
                            }
                            std::process::exit(exit_code::ERROR);
                        }
                    }

                    #[cfg(not(feature = "ui-gpui"))]
                    {
                        if let Some(report) = startup_crash_report.as_ref() {
                            print_startup_crash_report_hint(report);
                        }
                        gitcomet_ui::run(backend);
                    }
                } else {
                    if let Some(report) = startup_crash_report.as_ref() {
                        print_startup_crash_report_hint(report);
                    }
                    gitcomet_ui::run(backend);
                }
            }

            #[cfg(not(feature = "ui"))]
            {
                let _ = path;
                eprintln!("GitComet UI is disabled. Build with `-p gitcomet-app --features ui`.");
                std::process::exit(exit_code::ERROR);
            }
        }
        AppMode::Mergetool(config) => {
            #[cfg(not(feature = "ui-gpui"))]
            if config.gui {
                eprintln!(
                    "GUI mergetool mode is unavailable in this build. Rebuild with `-p gitcomet-app --features ui-gpui`."
                );
                std::process::exit(exit_code::ERROR);
            }

            let result = mergetool_mode::run_mergetool(&config);

            // When UI is available, --gui was requested, and text
            // conflicts remain unresolved, open the focused GPUI merge
            // window for interactive resolution.
            #[cfg(feature = "ui-gpui")]
            if let Ok(result) = &result
                && should_launch_focused_merge_gui(&config, result)
            {
                let Some(repo_path) = resolve_mergetool_repo_path(&config.merged) else {
                    eprintln!(
                        "Failed to locate repository root for merged path {}",
                        config.merged.display()
                    );
                    std::process::exit(exit_code::ERROR);
                };

                // Determine labels for display.
                let label_local = config
                    .label_local
                    .clone()
                    .unwrap_or_else(|| path_label(&config.local));
                let label_remote = config
                    .label_remote
                    .clone()
                    .unwrap_or_else(|| path_label(&config.remote));
                let label_base = config.label_base.clone().unwrap_or_else(|| {
                    config
                        .base
                        .as_ref()
                        .map(|p| path_label(p))
                        .unwrap_or_else(|| "empty tree".to_string())
                });

                let gui_config = gitcomet_ui_gpui::FocusedMergetoolConfig {
                    repo_path,
                    conflicted_file_path: config.merged.clone(),
                    label_local,
                    label_remote,
                    label_base,
                };
                let backend = build_backend();
                let code = gitcomet_ui_gpui::run_focused_mergetool(backend, gui_config);
                std::process::exit(code);
            }

            run_and_exit(result);
        }
        AppMode::Setup { dry_run, local } => run_and_exit(setup_mode::run_setup(dry_run, local)),
        AppMode::Uninstall { dry_run, local } => {
            run_and_exit(setup_mode::run_uninstall(dry_run, local))
        }
        AppMode::ExtractMergeFixtures(config) => {
            run_and_exit(extract_fixtures_mode::run_extract_merge_fixtures(&config))
        }
    }
}

#[cfg(all(target_os = "macos", feature = "ui-gpui"))]
const MACOS_BUNDLE_RELAUNCH_ENV: &str = "GITCOMET_SKIP_APP_BUNDLE_RELAUNCH";
#[cfg(all(target_os = "macos", feature = "ui-gpui"))]
const MACOS_APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/gitcomet-512.png");

#[cfg(all(target_os = "macos", feature = "ui-gpui"))]
fn maybe_relaunch_browser_from_macos_app_bundle() -> bool {
    if std::env::var_os(MACOS_BUNDLE_RELAUNCH_ENV).is_some() {
        return false;
    }

    let Ok(current_exe) = std::env::current_exe() else {
        return false;
    };
    if current_exe
        .to_string_lossy()
        .contains(".app/Contents/MacOS/")
    {
        return false;
    }

    let Some(bin_dir) = current_exe.parent() else {
        return false;
    };
    let app_bundle = bin_dir.join("GitComet.app");
    let app_exe = match ensure_macos_dev_app_bundle(&current_exe, &app_bundle) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("Failed to prepare macOS app bundle icon: {err}");
            return false;
        }
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

#[cfg(all(target_os = "macos", feature = "ui-gpui"))]
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

    let app_exe = macos.join("gitcomet-app");
    std::fs::copy(current_exe, &app_exe)
        .map_err(|e| format!("failed to copy executable into bundle: {e}"))?;

    let icon_png = resources.join("GitComet.png");
    let icon_icns = resources.join("GitComet.icns");
    std::fs::write(&icon_png, MACOS_APP_ICON_PNG)
        .map_err(|e| format!("failed to write icon PNG: {e}"))?;

    let icon_status = std::process::Command::new("sips")
        .arg("-s")
        .arg("format")
        .arg("icns")
        .arg(&icon_png)
        .arg("--out")
        .arg(&icon_icns)
        .status()
        .map_err(|e| format!("failed to run sips: {e}"))?;
    if !icon_status.success() {
        return Err(format!(
            "sips returned non-zero exit status while generating {}",
            icon_icns.display()
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
  <string>GitComet</string>
  <key>CFBundleExecutable</key>
  <string>gitcomet-app</string>
  <key>CFBundleIdentifier</key>
  <string>ai.autoexplore.gitcomet.dev</string>
  <key>CFBundleIconFile</key>
  <string>GitComet.icns</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>GitComet</string>
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
        version = env!("CARGO_PKG_VERSION")
    );
    std::fs::write(contents.join("Info.plist"), plist)
        .map_err(|e| format!("failed to write Info.plist: {e}"))?;

    Ok(app_exe)
}

#[cfg(feature = "ui")]
fn print_startup_crash_report_hint(report: &crashlog::StartupCrashReport) {
    eprintln!("GitComet detected a crash from a previous run.");
    eprintln!(
        "Open this URL to file a prefilled crash report:\n{}",
        report.issue_url
    );
    eprintln!("Crash log: {}", report.crash_log_path.display());
}

#[cfg(feature = "ui")]
fn build_backend() -> std::sync::Arc<dyn gitcomet_core::services::GitBackend> {
    if cfg!(feature = "gix") {
        #[cfg(feature = "gix")]
        {
            std::sync::Arc::new(gitcomet_git_gix::GixBackend)
        }

        #[cfg(not(feature = "gix"))]
        {
            gitcomet_git::default_backend()
        }
    } else {
        gitcomet_git::default_backend()
    }
}

/// Extract a filename label from a path.
#[cfg(feature = "ui-gpui")]
fn path_label(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{path:?}"))
}

#[cfg(feature = "ui-gpui")]
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
        if dot_git.is_dir() || dot_git.is_file() {
            return Some(cursor.to_path_buf());
        }

        cursor = cursor.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::merge::{ConflictStyle, DEFAULT_MARKER_SIZE, DiffAlgorithm, MergeResult};
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

    fn mergetool_config(gui: bool, auto: bool) -> cli::MergetoolConfig {
        cli::MergetoolConfig {
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
            auto,
            gui,
        }
    }

    fn unresolved_merge_result() -> MergeResult {
        MergeResult {
            output: "<<<<<<< ours\nleft\n=======\nright\n>>>>>>> theirs\n".to_string(),
            conflict_count: 1,
        }
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
    fn focused_merge_gui_launches_for_unresolved_text_conflict() {
        let config = mergetool_config(true, false);
        let result = mergetool_mode::MergetoolRunResult {
            stdout: String::new(),
            stderr: "conflict".to_string(),
            exit_code: exit_code::CANCELED,
            merge_result: Some(unresolved_merge_result()),
        };

        assert!(should_launch_focused_merge_gui(&config, &result));
    }

    #[test]
    fn focused_merge_gui_launches_after_auto_mode_when_unresolved_conflicts_remain() {
        let config = mergetool_config(true, true);
        let result = mergetool_mode::MergetoolRunResult {
            stdout: String::new(),
            stderr: "auto could not resolve all conflicts".to_string(),
            exit_code: exit_code::CANCELED,
            merge_result: Some(unresolved_merge_result()),
        };

        assert!(should_launch_focused_merge_gui(&config, &result));
    }

    #[test]
    fn focused_merge_gui_does_not_launch_when_not_requested() {
        let config = mergetool_config(false, false);
        let result = mergetool_mode::MergetoolRunResult {
            stdout: String::new(),
            stderr: "conflict".to_string(),
            exit_code: exit_code::CANCELED,
            merge_result: Some(unresolved_merge_result()),
        };

        assert!(!should_launch_focused_merge_gui(&config, &result));
    }

    #[test]
    fn focused_merge_gui_does_not_launch_on_success_exit() {
        let config = mergetool_config(true, false);
        let result = mergetool_mode::MergetoolRunResult {
            stdout: String::new(),
            stderr: "clean merge".to_string(),
            exit_code: exit_code::SUCCESS,
            merge_result: Some(MergeResult {
                output: "clean\n".to_string(),
                conflict_count: 0,
            }),
        };

        assert!(!should_launch_focused_merge_gui(&config, &result));
    }

    #[test]
    fn focused_merge_gui_does_not_launch_for_binary_conflict_without_merge_result() {
        let config = mergetool_config(true, false);
        let result = mergetool_mode::MergetoolRunResult {
            stdout: String::new(),
            stderr: "binary conflict".to_string(),
            exit_code: exit_code::CANCELED,
            merge_result: None,
        };

        assert!(!should_launch_focused_merge_gui(&config, &result));
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
