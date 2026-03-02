mod cli;
#[cfg(feature = "ui")]
mod crashlog;
mod difftool_mode;
mod extract_fixtures_mode;
mod mergetool_mode;
mod setup_mode;

use cli::{AppMode, exit_code};
use std::io::{self, Write};

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
                    "GUI difftool mode is unavailable in this build. Rebuild with `-p gitgpui-app --features ui-gpui`."
                );
                std::process::exit(exit_code::ERROR);
            }

            match difftool_mode::run_difftool(&config) {
                Ok(result) => {
                    // When UI is available and --gui was requested, open a focused
                    // GPUI diff window instead of printing raw text to stdout.
                    #[cfg(feature = "ui-gpui")]
                    if should_launch_focused_diff_gui(&config, &result) {
                        let label_left = config
                            .label_left
                            .clone()
                            .unwrap_or_else(|| path_label(&config.local));
                        let label_right = config
                            .label_right
                            .clone()
                            .unwrap_or_else(|| path_label(&config.remote));

                        let gui_config = gitgpui_ui_gpui::FocusedDiffConfig {
                            label_left,
                            label_right,
                            display_path: config.display_path.clone(),
                            diff_text: result.stdout.clone(),
                        };
                        let code = gitgpui_ui_gpui::run_focused_diff(gui_config);
                        std::process::exit(code);
                    }

                    if !result.stdout.is_empty() {
                        print!("{}", result.stdout);
                    }
                    if !result.stderr.is_empty() {
                        eprint!("{}", result.stderr);
                    }
                    let _ = io::stdout().flush();
                    let _ = io::stderr().flush();
                    std::process::exit(result.exit_code);
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    std::process::exit(exit_code::ERROR);
                }
            }
        }
        AppMode::Browser { path } => {
            #[cfg(feature = "ui")]
            {
                use gitgpui_core::services::GitBackend;
                use std::sync::Arc;

                let backend: Arc<dyn GitBackend> = if cfg!(feature = "gix") {
                    #[cfg(feature = "gix")]
                    {
                        Arc::new(gitgpui_git_gix::GixBackend)
                    }

                    #[cfg(not(feature = "gix"))]
                    {
                        gitgpui_git::default_backend()
                    }
                } else {
                    gitgpui_git::default_backend()
                };

                // Pass path to the UI layer. The existing run() reads
                // std::env::args_os().nth(1) internally, so for now we
                // ignore `path` here — it is parsed for future use.
                let _ = path;

                if cfg!(feature = "ui-gpui") {
                    #[cfg(feature = "ui-gpui")]
                    {
                        gitgpui_ui_gpui::run(backend);
                    }

                    #[cfg(not(feature = "ui-gpui"))]
                    {
                        gitgpui_ui::run(backend);
                    }
                } else {
                    gitgpui_ui::run(backend);
                }
            }

            #[cfg(not(feature = "ui"))]
            {
                let _ = path;
                eprintln!("GitGpui UI is disabled. Build with `-p gitgpui-app --features ui`.");
                std::process::exit(exit_code::ERROR);
            }
        }
        AppMode::Mergetool(config) => {
            #[cfg(not(feature = "ui-gpui"))]
            if config.gui {
                eprintln!(
                    "GUI mergetool mode is unavailable in this build. Rebuild with `-p gitgpui-app --features ui-gpui`."
                );
                std::process::exit(exit_code::ERROR);
            }

            match mergetool_mode::run_mergetool(&config) {
                Ok(result) => {
                    // When UI is available, --gui was requested, and text
                    // conflicts remain unresolved, open the focused GPUI merge
                    // window for interactive resolution.
                    #[cfg(feature = "ui-gpui")]
                    if should_launch_focused_merge_gui(&config, &result)
                        && let Some(ref merge_result) = result.merge_result
                    {
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

                        let gui_config = gitgpui_ui_gpui::FocusedMergeConfig {
                            merged_path: config.merged.clone(),
                            label_local,
                            label_remote,
                            label_base,
                            merged_text: merge_result.output.clone(),
                            is_clean: merge_result.is_clean(),
                            conflict_count: merge_result.conflict_count,
                        };
                        let code = gitgpui_ui_gpui::run_focused_merge(gui_config);
                        std::process::exit(code);
                    }

                    if !result.stdout.is_empty() {
                        print!("{}", result.stdout);
                    }
                    if !result.stderr.is_empty() {
                        eprint!("{}", result.stderr);
                    }
                    let _ = io::stdout().flush();
                    let _ = io::stderr().flush();
                    std::process::exit(result.exit_code);
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    std::process::exit(exit_code::ERROR);
                }
            }
        }
        AppMode::Setup { dry_run, local } => match setup_mode::run_setup(dry_run, local) {
            Ok(result) => {
                if !result.stdout.is_empty() {
                    print!("{}", result.stdout);
                }
                let _ = io::stdout().flush();
                std::process::exit(result.exit_code);
            }
            Err(msg) => {
                eprintln!("{msg}");
                std::process::exit(exit_code::ERROR);
            }
        },
        AppMode::ExtractMergeFixtures(config) => {
            match extract_fixtures_mode::run_extract_merge_fixtures(&config) {
                Ok(result) => {
                    if !result.stdout.is_empty() {
                        print!("{}", result.stdout);
                    }
                    if !result.stderr.is_empty() {
                        eprint!("{}", result.stderr);
                    }
                    let _ = io::stdout().flush();
                    let _ = io::stderr().flush();
                    std::process::exit(result.exit_code);
                }
                Err(msg) => {
                    eprintln!("{msg}");
                    std::process::exit(exit_code::ERROR);
                }
            }
        }
    }
}

/// Extract a filename label from a path.
#[cfg(feature = "ui-gpui")]
fn path_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitgpui_core::merge::{ConflictStyle, DEFAULT_MARKER_SIZE, DiffAlgorithm, MergeResult};

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
}
