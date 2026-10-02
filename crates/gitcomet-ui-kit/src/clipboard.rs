#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CopySource {
    CommitDetailsDiff,
    CommitRangeDiff,
    StagedDiff,
    UnstagedDiff,
    DiffContextMenu,
    FilePathShortcut,
    TextInputShortcut,
    TextInputContextMenu,
    TerminalShortcut,
    TerminalContextMenu,
    HookActivity,
    ErrorDetails,
    ContextMenu,
    EnvironmentDetails,
    /// A copy an extension makes.
    Extension,
}

impl CopySource {
    #[cfg(target_os = "linux")]
    fn as_str(self) -> &'static str {
        match self {
            Self::CommitDetailsDiff => "commit-details-diff",
            Self::CommitRangeDiff => "commit-range-diff",
            Self::StagedDiff => "staged-diff",
            Self::UnstagedDiff => "unstaged-diff",
            Self::DiffContextMenu => "diff-context-menu",
            Self::FilePathShortcut => "file-path-shortcut",
            Self::TextInputShortcut => "text-input-shortcut",
            Self::TextInputContextMenu => "text-input-context-menu",
            Self::TerminalShortcut => "terminal-shortcut",
            Self::TerminalContextMenu => "terminal-context-menu",
            Self::HookActivity => "hook-activity",
            Self::ErrorDetails => "error-details",
            Self::ContextMenu => "context-menu",
            Self::EnvironmentDetails => "environment-details",
            Self::Extension => "extension",
        }
    }
}

/// Live runs only: a dependent's tests must not probe the desktop or write
/// into the real crash directory.
fn copy_diagnostics_enabled() -> bool {
    cfg!(target_os = "linux") && crate::ui_runtime::current().uses_clipboard_diagnostics()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClipboardBackend {
    Gpui,
    /// Only ever selected by `select_clipboard_backend` under WSLg, so on a
    /// non-Linux build nothing constructs it -- the variant stays so
    /// `write_text` keeps one shape across platforms and the selection rules
    /// stay unit-testable everywhere.
    #[allow(dead_code)]
    X11,
}

pub fn write_text<T: 'static>(cx: &mut gpui::Context<T>, text: String, source: CopySource) {
    let backend = clipboard_backend();
    write_copy_diagnostic(source, text.len(), backend);

    match backend {
        ClipboardBackend::Gpui => {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        ClipboardBackend::X11 => write_text_to_x11(&text),
    }
}

pub fn read_text<T: 'static>(cx: &gpui::Context<T>) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// Pure decision function, so it is exercised by this module's tests on every
/// platform; only the Linux `clipboard_backend` actually calls it at runtime.
#[allow(dead_code)]
fn select_clipboard_backend(
    is_wsl: bool,
    wayland_available: bool,
    x11_available: bool,
) -> ClipboardBackend {
    if is_wsl && wayland_available && x11_available {
        // WSLg has disconnected GitComet for both mouse- and keyboard-initiated
        // GPUI Wayland clipboard writes. Route every write through its X11
        // clipboard bridge instead. This is the complete operation, not a
        // second write or a fallback after submitting a Wayland request.
        ClipboardBackend::X11
    } else {
        ClipboardBackend::Gpui
    }
}

#[cfg(target_os = "linux")]
fn clipboard_backend() -> ClipboardBackend {
    if !copy_diagnostics_enabled() {
        return ClipboardBackend::Gpui;
    }
    let environment = crate::linux_gui_env::LinuxGuiEnvironment::detect();
    select_clipboard_backend(
        environment.is_wsl,
        environment.has_wayland,
        environment.has_x11,
    )
}

#[cfg(not(target_os = "linux"))]
fn clipboard_backend() -> ClipboardBackend {
    ClipboardBackend::Gpui
}

#[cfg(target_os = "linux")]
fn write_copy_diagnostic(source: CopySource, text_len: usize, backend: ClipboardBackend) {
    if !copy_diagnostics_enabled() {
        return;
    }
    if let Err(err) = write_copy_diagnostic_inner(source, text_len, backend) {
        eprintln!(
            "Failed to write {} copy crash diagnostics: {err}",
            gitcomet_core::identity::current().display_name()
        );
    }
}

#[cfg(target_os = "linux")]
fn write_copy_diagnostic_inner(
    source: CopySource,
    text_len: usize,
    backend: ClipboardBackend,
) -> std::io::Result<()> {
    let dir = gitcomet_core::platform::dirs::crash_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no per-user state directory is configured",
        )
    })?;

    let text = format!(
        "copy_source={}\ncopy_text_bytes={text_len}\ndisplay={}\nwayland_display={}\n\
         clipboard_backend={}\n",
        source.as_str(),
        env_value("DISPLAY"),
        env_value("WAYLAND_DISPLAY"),
        match backend {
            ClipboardBackend::Gpui => "gpui",
            ClipboardBackend::X11 => "x11",
        },
    );
    gitcomet_core::fs_utils::write_private_file(
        &dir.join(format!("last-operation-{}.log", std::process::id())),
        text.as_bytes(),
    )
}

#[cfg(target_os = "linux")]
fn env_value(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "<unset>".to_string())
}

#[cfg(not(target_os = "linux"))]
fn write_copy_diagnostic(_source: CopySource, _text_len: usize, _backend: ClipboardBackend) {}

#[cfg(target_os = "linux")]
fn write_text_to_x11(text: &str) {
    thread_local! {
        static X11_CLIPBOARD: std::cell::RefCell<Option<x11_clipboard::Clipboard>> =
            const { std::cell::RefCell::new(None) };
    }

    let result = X11_CLIPBOARD.with(|clipboard| {
        let mut clipboard = clipboard.borrow_mut();
        replace_clipboard_owner(
            &mut clipboard,
            || x11_clipboard::Clipboard::new().map_err(|err| err.to_string()),
            |next| {
                let atoms = &next.setter.atoms;
                next.store(atoms.clipboard, atoms.utf8_string, text.as_bytes().to_vec())
                    .map_err(|err| err.to_string())
            },
        )
    });

    if let Err(err) = result {
        // Calling the Wayland setter as a fallback here would reintroduce the
        // process-terminating WSLg failure this path exists to avoid.
        eprintln!("Failed to copy text through the X11 clipboard bridge: {err}");
    }
}

/// Pure over its `create`/`store` callbacks, so it is exercised by this
/// module's tests on every platform; only `write_text_to_x11` calls it at
/// runtime, and that exists on Linux alone.
#[allow(dead_code)]
fn replace_clipboard_owner<Clipboard, Error>(
    active: &mut Option<Clipboard>,
    create: impl FnOnce() -> Result<Clipboard, Error>,
    store: impl FnOnce(&Clipboard) -> Result<(), Error>,
) -> Result<(), Error> {
    let next = create()?;
    store(&next)?;
    *active = Some(next);
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn write_text_to_x11(_text: &str) {
    unreachable!("the X11 clipboard backend is only selected on Linux")
}

#[cfg(test)]
mod tests {
    use super::{
        ClipboardBackend, clipboard_backend, copy_diagnostics_enabled, replace_clipboard_owner,
        select_clipboard_backend,
    };
    use crate::ui_runtime::{UiRuntime, with_override};

    /// The switch is the runtime, not `cfg(test)`, which a dependent's tests
    /// never see: deterministic runs neither probe the desktop nor log.
    #[test]
    fn only_live_runs_probe_the_platform_clipboard_or_log_copies() {
        with_override(UiRuntime::deterministic(), || {
            assert!(!copy_diagnostics_enabled());
            assert_eq!(clipboard_backend(), ClipboardBackend::Gpui);
        });
        with_override(UiRuntime::live(), || {
            assert_eq!(copy_diagnostics_enabled(), cfg!(target_os = "linux"));
        });
    }

    #[test]
    fn wslg_all_copy_paths_exclusively_use_x11() {
        assert_eq!(
            select_clipboard_backend(true, true, true),
            ClipboardBackend::X11
        );
    }

    #[test]
    fn non_wsl_and_non_hybrid_copies_keep_using_gpui() {
        assert_eq!(
            select_clipboard_backend(false, true, true),
            ClipboardBackend::Gpui
        );
        assert_eq!(
            select_clipboard_backend(true, true, false),
            ClipboardBackend::Gpui
        );
        assert_eq!(
            select_clipboard_backend(true, false, true),
            ClipboardBackend::Gpui
        );
    }

    #[test]
    fn successive_x11_writes_replace_the_selection_owner() {
        #[derive(Debug, Eq, PartialEq)]
        struct FakeClipboard(u8);

        let mut active = None;
        let mut served = Vec::new();
        replace_clipboard_owner(
            &mut active,
            || Ok::<_, ()>(FakeClipboard(1)),
            |owner| {
                served.push((owner.0, "first"));
                Ok(())
            },
        )
        .expect("store first selection");
        replace_clipboard_owner(
            &mut active,
            || Ok::<_, ()>(FakeClipboard(2)),
            |owner| {
                served.push((owner.0, "second"));
                Ok(())
            },
        )
        .expect("store second selection");

        assert_eq!(served, vec![(1, "first"), (2, "second")]);
        assert_eq!(active, Some(FakeClipboard(2)));
    }

    #[test]
    fn production_clipboard_access_is_centralized_in_this_module() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let offenders = crate::test_support::source_guards::direct_clipboard_access(
            &src_dir,
            &["clipboard.rs"],
        );
        assert!(
            offenders.is_empty(),
            "these access the GPUI clipboard directly; use the kit's clipboard module: {offenders:?}"
        );
    }
}
