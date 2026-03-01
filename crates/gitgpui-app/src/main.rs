mod cli;
mod crashlog;
mod difftool_mode;

use cli::{AppMode, exit_code};
use std::io::{self, Write};

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
        AppMode::Difftool(config) => match difftool_mode::run_difftool(&config) {
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
        },
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
            #[cfg(feature = "ui")]
            {
                // TODO: launch focused merge view via GPUI
                eprintln!(
                    "mergetool mode: merged={} local={} remote={}",
                    config.merged.display(),
                    config.local.display(),
                    config.remote.display()
                );
                if let Some(ref b) = config.base {
                    eprintln!("  base: {}", b.display());
                }
                eprintln!(
                    "Focused mergetool UI is not yet implemented. \
                     Use the full browser for now."
                );
                std::process::exit(exit_code::ERROR);
            }

            #[cfg(not(feature = "ui"))]
            {
                let _ = config;
                eprintln!("GitGpui UI is disabled. Build with `-p gitgpui-app --features ui`.");
                std::process::exit(exit_code::ERROR);
            }
        }
    }
}
