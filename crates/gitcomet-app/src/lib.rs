//! Process launch for GitComet and applications built on its crates.
//!
//! [`AppLaunch`] installs the product identity, then crash logging, parses the
//! command line, runs the caller's preparation hook, and runs the selected
//! mode: the repository browser, the focused difftool or mergetool, or Git
//! tool setup. The executable keeps only its allocator, platform resources,
//! and `main`.

#[cfg(feature = "ui-gpui-runtime")]
mod browser_instance;
pub mod cli;
#[cfg(feature = "ui-gpui-runtime")]
mod crashlog;
mod difftool_mode;
mod extract_fixtures_mode;
mod git_root;
mod launch;
#[cfg(any(
    all(target_os = "linux", feature = "ui-gpui-runtime"),
    all(test, feature = "ui-gpui-runtime")
))]
mod linux_wayland_fallback;
mod mergetool_mode;
mod setup_mode;

pub use cli::{AppMode, CliOutcome, exit_code};
pub use gitcomet_core::identity::ProductIdentity;
/// The extension API this launch accepts, re-exported so a product names
/// one revision of it.
#[cfg(feature = "ui-gpui-runtime")]
pub use gitcomet_extension_api as extension_api;

pub(crate) use gitcomet_core::hex::encode as hex_encode;

use std::ffi::OsString;
use std::io::Write;

type PrepareHook = Box<dyn FnOnce(&AppMode) -> Result<(), String>>;

/// The compiled-in extensions, in registration order.
#[cfg(feature = "ui-gpui-runtime")]
pub(crate) type Extensions = Vec<Box<dyn gitcomet_extension_api::Extension>>;
#[cfg(not(feature = "ui-gpui-runtime"))]
pub(crate) type Extensions = ();

/// Everything a product decides about its process launch.
pub struct AppLaunch {
    identity: ProductIdentity,
    about: String,
    on_prepare: Option<PrepareHook>,
    extensions: Extensions,
    repository_options: gitcomet_core::services::RepositoryOptions,
    repository_options_customized: bool,
    backend: Option<std::sync::Arc<dyn gitcomet_core::services::GitBackend>>,
}

impl AppLaunch {
    pub fn new(identity: ProductIdentity) -> Self {
        let repository_options = gitcomet_core::services::RepositoryOptions::default()
            .with_history_ref_filter(gitcomet_core::services::HistoryRefFilter::excluding(
                identity.hidden_ref_prefixes().iter().copied(),
            ));
        Self {
            identity,
            about: cli::DEFAULT_ABOUT.to_string(),
            on_prepare: None,
            extensions: Extensions::default(),
            repository_options,
            repository_options_customized: false,
            backend: None,
        }
    }

    /// GitComet's own launch.
    pub fn gitcomet() -> Self {
        Self::new(ProductIdentity::gitcomet())
    }

    /// Installs a product's identity before logging and any path resolution.
    pub fn identity(mut self, identity: ProductIdentity) -> Self {
        if !self.repository_options_customized {
            self.repository_options = gitcomet_core::services::RepositoryOptions::default()
                .with_history_ref_filter(gitcomet_core::services::HistoryRefFilter::excluding(
                    identity.hidden_ref_prefixes().iter().copied(),
                ));
        }
        self.identity = identity;
        self
    }

    /// The one-line description in `--help`.
    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = about.into();
        self
    }

    /// Runs after the command line parsed into a mode and before the mode
    /// runs; `--help` and `--version` skip it. An error is printed and the
    /// process exits with [`exit_code::ERROR`].
    pub fn on_prepare(
        mut self,
        prepare: impl FnOnce(&AppMode) -> Result<(), String> + 'static,
    ) -> Self {
        self.on_prepare = Some(Box::new(prepare));
        self
    }

    /// Compiles in `extension`. Extensions register in the order added; the
    /// set is validated once the command line has parsed, before any window
    /// opens, and a registration error exits with [`exit_code::ERROR`].
    #[cfg(feature = "ui-gpui-runtime")]
    pub fn extension(mut self, extension: impl gitcomet_extension_api::Extension) -> Self {
        self.extensions.push(Box::new(extension));
        self
    }

    /// Options every repository opens with, such as refs History leaves out.
    /// Defaults to the product identity's hidden ref prefixes.
    pub fn repository_options(
        mut self,
        options: gitcomet_core::services::RepositoryOptions,
    ) -> Self {
        self.repository_options_customized = true;
        self.repository_options = options;
        self
    }

    /// Uses this backend for repository operations in every UI window.
    /// Repository options apply to custom backends just as to the default.
    pub fn backend(
        mut self,
        backend: std::sync::Arc<dyn gitcomet_core::services::GitBackend>,
    ) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Runs with the process arguments and returns the exit code.
    pub fn run(self) -> i32 {
        self.run_with_args(std::env::args_os().collect())
    }

    /// Runs with `args` (including the program name) and returns the exit code.
    pub fn run_with_args(self, args: Vec<OsString>) -> i32 {
        let Self {
            identity,
            about,
            on_prepare,
            extensions,
            repository_options,
            backend,
            repository_options_customized: _,
        } = self;
        if let Err(message) = install_identity(identity) {
            eprintln!("{message}");
            return exit_code::ERROR;
        }

        #[cfg(feature = "ui-gpui-runtime")]
        crashlog::install();

        dispatch(cli::parse_cli(args, &about), on_prepare, move |mode| {
            launch::run_mode(mode, extensions, repository_options, backend)
        })
    }
}

/// The process builder used by product binaries. Identity is installed only
/// when running, before crash logging, argument handling, or path resolution.
///
/// ```no_run
/// fn main() -> ! { gitcomet_app::App::new().run() }
/// ```
pub struct App(AppLaunch);

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
impl App {
    pub fn new() -> Self {
        Self(AppLaunch::gitcomet())
    }
    pub fn identity(self, identity: ProductIdentity) -> Self {
        Self(self.0.identity(identity))
    }
    pub fn on_prepare(
        self,
        prepare: impl FnOnce(&AppMode) -> Result<(), String> + 'static,
    ) -> Self {
        Self(self.0.on_prepare(prepare))
    }
    pub fn backend(self, backend: std::sync::Arc<dyn gitcomet_core::services::GitBackend>) -> Self {
        Self(self.0.backend(backend))
    }
    pub fn repository_options(self, options: gitcomet_core::services::RepositoryOptions) -> Self {
        Self(self.0.repository_options(options))
    }
    pub fn about(self, about: impl Into<String>) -> Self {
        Self(self.0.about(about))
    }
    #[cfg(feature = "ui-gpui-runtime")]
    pub fn extension(self, extension: impl gitcomet_extension_api::Extension) -> Self {
        Self(self.0.extension(extension))
    }
    pub fn run(self) -> ! {
        std::process::exit(self.0.run())
    }
}

/// Prints help or version, or prepares and runs the parsed mode.
fn dispatch(
    parsed: Result<CliOutcome, String>,
    on_prepare: Option<PrepareHook>,
    run_mode: impl FnOnce(AppMode) -> i32,
) -> i32 {
    let mode = match parsed {
        Ok(CliOutcome::Run(mode)) => mode,
        Ok(CliOutcome::Inform(info)) => {
            let _ = info.print();
            let _ = std::io::stdout().flush();
            return info.exit_code();
        }
        Err(message) => {
            eprintln!("{message}");
            return exit_code::ERROR;
        }
    };
    if let Some(prepare) = on_prepare
        && let Err(message) = prepare(&mode)
    {
        eprintln!("{message}");
        return exit_code::ERROR;
    }
    run_mode(mode)
}

/// Installs `identity` unless the same identity is already in place.
fn install_identity(identity: ProductIdentity) -> Result<(), String> {
    match gitcomet_core::identity::install(identity) {
        Ok(()) => Ok(()),
        Err(identity) if gitcomet_core::identity::current() == &*identity => Ok(()),
        Err(identity) => Err(format!(
            "Cannot launch {}: the process identity was resolved before launch as {}.",
            identity.display_name(),
            gitcomet_core::identity::current().display_name()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn parse(args: &[&str]) -> Result<CliOutcome, String> {
        cli::parse_cli(
            args.iter().map(OsString::from).collect(),
            cli::DEFAULT_ABOUT,
        )
    }

    #[test]
    fn product_builder_uses_identity_defaults_and_preserves_explicit_repository_options() {
        use gitcomet_core::services::{HistoryRefFilter, RepositoryOptions};
        const IDENTITY: ProductIdentity =
            ProductIdentity::new("Example", "example", "com.example.app")
                .with_hidden_ref_prefixes(&["refs/example/"]);
        let default = App::new().identity(IDENTITY.clone());
        assert!(
            default
                .0
                .repository_options
                .history_ref_filter
                .excludes("refs/example/1")
        );
        assert!(
            !default
                .0
                .repository_options
                .history_ref_filter
                .excludes("refs/pull/1")
        );
        let explicit = RepositoryOptions::default()
            .with_history_ref_filter(HistoryRefFilter::excluding(["refs/custom/"]));
        let customized = App::new()
            .repository_options(explicit.clone())
            .identity(IDENTITY);
        assert_eq!(customized.0.repository_options, explicit);
    }

    #[test]
    fn help_and_version_print_without_preparing_or_running() {
        for flag in ["--help", "--version"] {
            let outcome = parse(&["gitcomet", flag]).unwrap();
            let CliOutcome::Inform(info) = &outcome else {
                panic!("{flag} must be informational");
            };
            assert!(info.text().contains("gitcomet"), "{}", info.text());
            let code = dispatch(
                Ok(outcome),
                Some(Box::new(move |_: &AppMode| {
                    panic!("{flag} must bypass on_prepare")
                })),
                move |_| panic!("{flag} must not run a mode"),
            );
            assert_eq!(code, exit_code::SUCCESS);
        }
    }

    #[test]
    fn a_failed_preparation_reports_an_error_and_skips_the_mode() {
        let prepared = Rc::new(Cell::new(false));
        let seen = Rc::clone(&prepared);
        let code = dispatch(
            parse(&["gitcomet", "setup", "--dry-run"]),
            Some(Box::new(move |mode: &AppMode| {
                seen.set(matches!(mode, AppMode::Setup { dry_run: true, .. }));
                Err("extension registration failed".to_string())
            })),
            |_| panic!("the mode must not run after a failed preparation"),
        );
        assert!(prepared.get(), "on_prepare must see the parsed mode");
        assert_eq!(code, exit_code::ERROR);
    }

    #[test]
    fn a_successful_preparation_runs_the_mode_and_returns_its_code() {
        let code = dispatch(
            parse(&["gitcomet", "setup", "--dry-run"]),
            Some(Box::new(|_: &AppMode| Ok(()))),
            |mode| {
                assert!(matches!(mode, AppMode::Setup { dry_run: true, .. }));
                7
            },
        );
        assert_eq!(code, 7);
    }

    #[test]
    fn a_parse_error_exits_with_the_error_code_before_preparing() {
        let code = dispatch(
            parse(&["gitcomet", "--no-such-flag"]),
            Some(Box::new(|_: &AppMode| {
                panic!("a parse error must not prepare")
            })),
            |_| panic!("a parse error must not run a mode"),
        );
        assert_eq!(code, exit_code::ERROR);
    }
}
