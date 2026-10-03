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
#[cfg(any(target_os = "linux", test))]
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
    FILE_CLIPBOARD.with(|owned| owned.borrow_mut().take());
    bump_files_revision();
    let backend = clipboard_backend();
    write_copy_diagnostic(source, text.len(), backend);

    match backend {
        ClipboardBackend::Gpui => {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        ClipboardBackend::X11 => write_text_to_x11(&text),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePayload {
    pub paths: Vec<std::path::PathBuf>,
    pub intent: gitcomet_core::filesystem::TransferIntent,
    pub ownership: u64,
}

thread_local! {
    static FILE_CLIPBOARD: std::cell::RefCell<Option<(FilePayload, gpui::ClipboardItem)>> = const { std::cell::RefCell::new(None) };
    /// Bumped whenever this process changes the file clipboard. Views key a
    /// cached cut set on it rather than reading the platform clipboard every
    /// frame -- on X11 that read is a synchronous selection transfer.
    ///
    /// Only our own writes move it, so a cut made in another application is not
    /// reflected until something here touches the clipboard. That is the
    /// deliberate trade: dimming another app's cut is not worth a poll.
    static FILE_CLIPBOARD_REV: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Counts platform clipboard reads so tests can pin that the file-browser
    /// row builder is not doing one per frame.
    pub static FILE_CLIPBOARD_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Changes whenever this process writes or clears the file clipboard.
pub fn files_revision() -> u64 {
    FILE_CLIPBOARD_REV.with(|rev| rev.get())
}

fn bump_files_revision() {
    FILE_CLIPBOARD_REV.with(|rev| rev.set(rev.get().wrapping_add(1)));
}

pub fn write_files<T: 'static>(
    cx: &mut gpui::Context<T>,
    paths: Vec<std::path::PathBuf>,
    intent: gitcomet_core::filesystem::TransferIntent,
) {
    write_files_owned(
        cx,
        paths,
        intent,
        gitcomet_core::filesystem::OperationId::allocate().0,
    );
}

pub fn write_files_owned<T: 'static>(
    cx: &mut gpui::Context<T>,
    paths: Vec<std::path::PathBuf>,
    intent: gitcomet_core::filesystem::TransferIntent,
    ownership: u64,
) {
    let files = gpui::FileTransfer {
        paths: gpui::ExternalPaths(paths.iter().cloned().collect()),
        operation: if intent == gitcomet_core::filesystem::TransferIntent::Move {
            gpui::FileTransferOperation::Move
        } else {
            gpui::FileTransferOperation::Copy
        },
        ownership,
    };
    let item = gpui::ClipboardItem {
        entries: vec![gpui::ClipboardEntry::Files(files.clone())],
    };
    #[cfg(target_os = "linux")]
    if clipboard_backend() == ClipboardBackend::X11 {
        if let Err(error) = gpui_platform::write_files_to_x11_clipboard(&files) {
            eprintln!("Could not copy files: {error}");
            return;
        }
    } else {
        cx.write_to_clipboard(item.clone());
    }
    #[cfg(not(target_os = "linux"))]
    cx.write_to_clipboard(item.clone());
    FILE_CLIPBOARD.with(|owned| {
        *owned.borrow_mut() = Some((
            FilePayload {
                paths,
                intent,
                ownership,
            },
            item,
        ))
    });
    bump_files_revision();
    cx.refresh_windows();
}

pub fn read_files<T: 'static>(cx: &gpui::Context<T>) -> Option<FilePayload> {
    #[cfg(any(test, feature = "test-support"))]
    FILE_CLIPBOARD_READS.with(|reads| reads.set(reads.get() + 1));
    #[cfg(target_os = "linux")]
    let item = if clipboard_backend() == ClipboardBackend::X11 {
        gpui_platform::read_files_from_x11_clipboard().map(|files| gpui::ClipboardItem {
            entries: vec![gpui::ClipboardEntry::Files(files)],
        })
    } else {
        cx.read_from_clipboard()
    };
    #[cfg(not(target_os = "linux"))]
    let item = cx.read_from_clipboard();
    FILE_CLIPBOARD.with(|owned| {
        let mut owned = owned.borrow_mut();
        if let Some((payload, written)) = owned.as_ref()
            && item.as_ref() == Some(written)
        {
            return Some(payload.clone());
        }
        owned.take();
        let item = item?;
        let files = item.file_transfer()?;
        Some(FilePayload {
            paths: files.paths.paths().to_vec(),
            intent: if files.operation == gpui::FileTransferOperation::Move {
                gitcomet_core::filesystem::TransferIntent::Move
            } else {
                gitcomet_core::filesystem::TransferIntent::Copy
            },
            ownership: files.ownership,
        })
    })
}

/// Keeps the native owner alive across a paste and its conflict continuations.
pub struct PasteReceipt {
    native: Option<gpui::FilePaste>,
    payload: FilePayload,
    remaining: Vec<std::path::PathBuf>,
    intent: gitcomet_core::filesystem::TransferIntent,
}
impl PasteReceipt {
    pub fn from_drop(
        paths: Vec<std::path::PathBuf>,
        transfer: gpui::FileDropTransfer,
        intent: gitcomet_core::filesystem::TransferIntent,
    ) -> Self {
        Self {
            native: Some(transfer.completion),
            remaining: service_identities(&paths),
            payload: FilePayload {
                paths,
                intent,
                ownership: 0,
            },
            intent,
        }
    }
    /// `paths` are the filesystem service's canonical sources.
    pub fn completed(&mut self, paths: &[std::path::PathBuf]) {
        self.remaining
            .retain(|path| !paths.iter().any(|completed| path.starts_with(completed)));
    }
    pub fn finish<T: 'static>(self, cx: &mut gpui::Context<T>) {
        let complete = self.remaining.is_empty();
        let operation = complete.then_some(
            if self.intent == gitcomet_core::filesystem::TransferIntent::Move {
                gpui::FileTransferOperation::Move
            } else {
                gpui::FileTransferOperation::Copy
            },
        );
        if let Some(native) = self.native {
            native.complete(operation);
        } else if complete
            && self.intent == gitcomet_core::filesystem::TransferIntent::Move
            && read_files(cx).as_ref() == Some(&self.payload)
        {
            write_text(cx, String::new(), CopySource::ContextMenu);
        }
    }
}

/// Spelled as the filesystem service reports completed sources, so a path
/// reached through a symlink or a Windows 8.3 name still matches.
fn service_identities(paths: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    paths
        .iter()
        .map(|path| {
            gitcomet_core::filesystem::absolute_identity(path).unwrap_or_else(|_| path.clone())
        })
        .collect()
}

pub fn capture_paste<T: 'static>(
    cx: &gpui::Context<T>,
    paths: &[std::path::PathBuf],
    intent: gitcomet_core::filesystem::TransferIntent,
) -> Option<PasteReceipt> {
    let payload = read_files(cx)?;
    if payload.paths != paths {
        return None;
    }
    let files = gpui::FileTransfer {
        paths: gpui::ExternalPaths(payload.paths.iter().cloned().collect()),
        operation: if payload.intent == gitcomet_core::filesystem::TransferIntent::Move {
            gpui::FileTransferOperation::Move
        } else {
            gpui::FileTransferOperation::Copy
        },
        ownership: payload.ownership,
    };
    let native = cx.capture_file_paste(&files);
    Some(PasteReceipt {
        remaining: service_identities(&payload.paths),
        native,
        payload,
        intent,
    })
}

pub fn cancel_cut<T: 'static>(cx: &mut gpui::Context<T>) {
    let current = read_files(cx);
    if let Some(payload) = current
        && payload.intent == gitcomet_core::filesystem::TransferIntent::Move
        && payload.ownership != 0
    {
        write_files(
            cx,
            payload.paths,
            gitcomet_core::filesystem::TransferIntent::Copy,
        );
    }
}

pub fn complete_file_move<T: 'static>(
    cx: &mut gpui::Context<T>,
    ownership: u64,
    completed: &[std::path::PathBuf],
) {
    let Some(mut current) = read_files(cx).filter(|p| p.ownership == ownership && ownership != 0)
    else {
        return;
    };
    current
        .paths
        .retain(|path| !completed.iter().any(|done| path.starts_with(done)));
    if current.paths.is_empty() {
        FILE_CLIPBOARD.with(|owned| owned.borrow_mut().take());
        write_text(cx, String::new(), CopySource::ContextMenu);
    } else {
        write_files_owned(cx, current.paths, current.intent, ownership);
    }
}

#[cfg(target_os = "windows")]
pub fn snapshot<T: 'static>(cx: &gpui::Context<T>) -> Option<gpui::ClipboardItem> {
    cx.read_from_clipboard()
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
        ClipboardBackend, PasteReceipt, clipboard_backend, copy_diagnostics_enabled,
        replace_clipboard_owner, select_clipboard_backend,
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

    #[cfg(unix)]
    #[test]
    fn drop_receipts_match_completions_reported_under_the_canonical_parent() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("a.txt"), "a").unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut receipt = PasteReceipt::from_drop(
            vec![link.join("a.txt")],
            gpui::FileDropTransfer {
                operation: gpui::FileTransferOperation::Move,
                source_owns_move: false,
                completion: gpui::FilePaste::new(|_| {}),
            },
            gitcomet_core::filesystem::TransferIntent::Move,
        );
        receipt.completed(&[std::fs::canonicalize(&real).unwrap().join("a.txt")]);
        assert!(
            receipt.remaining.is_empty(),
            "a moved source dropped through a symlink still completes the drop"
        );
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
