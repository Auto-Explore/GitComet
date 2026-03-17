mod cli;
#[cfg(feature = "ui")]
mod crashlog;
mod difftool_mode;
mod extract_fixtures_mode;
mod mergetool_mode;
mod setup_mode;

use cli::{AppMode, exit_code};
use mimalloc::MiMalloc;

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
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

#[cfg(any(feature = "ui-gpui-runtime", test))]
fn should_launch_focused_diff_gui(
    config: &cli::DifftoolConfig,
    result: &difftool_mode::DifftoolRunResult,
) -> bool {
    config.gui && result.exit_code == exit_code::SUCCESS
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
            #[cfg(not(feature = "ui-gpui-runtime"))]
            if config.gui {
                eprintln!(
                    "GUI difftool mode is unavailable in this build. Rebuild with `-p gitcomet-app --features ui-gpui`."
                );
                std::process::exit(exit_code::ERROR);
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
                let code = gitcomet_ui_gpui::run_focused_diff(gui_config);
                std::process::exit(code);
            }

            run_and_exit(result);
        }
        AppMode::Browser { path } => {
            #[cfg(feature = "ui")]
            {
                #[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
                if maybe_relaunch_browser_from_macos_app_bundle() {
                    std::process::exit(exit_code::SUCCESS);
                }

                let startup_crash_report = crashlog::take_startup_report();
                let backend = build_backend();

                if cfg!(feature = "ui-gpui-runtime") {
                    #[cfg(feature = "ui-gpui-runtime")]
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

                    #[cfg(not(feature = "ui-gpui-runtime"))]
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
            #[cfg(not(feature = "ui-gpui-runtime"))]
            if config.gui {
                eprintln!(
                    "GUI mergetool mode is unavailable in this build. Rebuild with `-p gitcomet-app --features ui-gpui`."
                );
                std::process::exit(exit_code::ERROR);
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
                        std::process::exit(exit_code::ERROR);
                    }
                };
                let backend = build_backend();
                let code = gitcomet_ui_gpui::run_focused_mergetool(backend, gui_config);
                std::process::exit(code);
            }

            let result = mergetool_mode::run_mergetool(&config);

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

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
const MACOS_BUNDLE_RELAUNCH_ENV: &str = "GITCOMET_SKIP_APP_BUNDLE_RELAUNCH";
#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
const MACOS_APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/gitcomet-512.png");

#[cfg(all(target_os = "macos", feature = "ui-gpui-runtime"))]
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
        if dot_git.is_dir() || dot_git.is_file() {
            return Some(cursor.to_path_buf());
        }

        cursor = cursor.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::merge::{ConflictStyle, DEFAULT_MARKER_SIZE, DiffAlgorithm};
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
    #[cfg(feature = "ui-gpui-runtime")]
    fn build_focused_mergetool_gui_config_uses_default_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("repo");
        let merged = repo_root.join("src/conflicted.txt");
        let base = repo_root.join("base.txt");

        fs::create_dir_all(repo_root.join(".git")).unwrap();
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

        fs::create_dir_all(repo_root.join(".git")).unwrap();
        fs::create_dir_all(merged.parent().unwrap()).unwrap();

        let config = mergetool_config(&repo_root, merged, None);
        let gui_config = build_focused_mergetool_gui_config(&config).expect("gui config");

        assert_eq!(gui_config.label_base, "empty tree");
    }

    #[test]
    #[cfg(feature = "ui-gpui-runtime")]
    fn build_focused_mergetool_gui_config_errors_without_repo_root() {
        let tmp = tempfile::tempdir().unwrap();
        let merged = tmp.path().join("outside-repo/merged.txt");

        fs::create_dir_all(merged.parent().unwrap()).unwrap();

        let config = mergetool_config(tmp.path(), merged.clone(), None);
        let err =
            build_focused_mergetool_gui_config(&config).expect_err("expected missing repo root");

        assert!(err.contains("Failed to locate repository root"));
        assert!(err.contains(&merged.display().to_string()));
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
