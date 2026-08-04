#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CopySource {
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
    TerminalProtocol,
    ContextMenu,
}

impl CopySource {
    #[cfg(all(target_os = "linux", not(test)))]
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
            Self::TerminalProtocol => "terminal-protocol",
            Self::ContextMenu => "context-menu",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClipboardBackend {
    Gpui,
    X11,
}

pub(crate) fn write_text<T: 'static>(cx: &mut gpui::Context<T>, text: String, source: CopySource) {
    let backend = clipboard_backend();
    write_copy_diagnostic(source, text.len(), backend);

    match backend {
        ClipboardBackend::Gpui => {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        ClipboardBackend::X11 => write_text_to_x11(&text),
    }
}

pub(crate) fn read_text<T: 'static>(cx: &gpui::Context<T>) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

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

#[cfg(all(target_os = "linux", not(test)))]
fn clipboard_backend() -> ClipboardBackend {
    let environment = crate::linux_gui_env::LinuxGuiEnvironment::detect();
    select_clipboard_backend(
        environment.is_wsl,
        environment.has_wayland,
        environment.has_x11,
    )
}

#[cfg(not(all(target_os = "linux", not(test))))]
fn clipboard_backend() -> ClipboardBackend {
    ClipboardBackend::Gpui
}

#[cfg(all(target_os = "linux", not(test)))]
fn write_copy_diagnostic(source: CopySource, text_len: usize, backend: ClipboardBackend) {
    if let Err(err) = write_copy_diagnostic_inner(source, text_len, backend) {
        eprintln!("Failed to write GitComet copy crash diagnostics: {err}");
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn write_copy_diagnostic_inner(
    source: CopySource,
    text_len: usize,
    backend: ClipboardBackend,
) -> std::io::Result<()> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| home.join(".local/state").into_os_string())
        })
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "XDG_STATE_HOME and HOME are unset",
            )
        })?;
    let dir = std::path::PathBuf::from(base)
        .join("gitcomet")
        .join("crashes");
    std::fs::create_dir_all(&dir)?;

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
    let mut file =
        std::fs::File::create(dir.join(format!("last-operation-{}.log", std::process::id())))?;
    use std::io::Write as _;
    file.write_all(text.as_bytes())?;
    file.sync_data()
}

#[cfg(all(target_os = "linux", not(test)))]
fn env_value(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "<unset>".to_string())
}

#[cfg(not(all(target_os = "linux", not(test))))]
fn write_copy_diagnostic(_source: CopySource, _text_len: usize, _backend: ClipboardBackend) {}

#[cfg(all(target_os = "linux", not(test)))]
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

#[cfg(not(all(target_os = "linux", not(test))))]
fn write_text_to_x11(_text: &str) {
    unreachable!("the X11 clipboard backend is only selected on Linux")
}

#[cfg(test)]
mod tests {
    use super::{ClipboardBackend, replace_clipboard_owner, select_clipboard_backend};

    fn rust_sources_under(dir: &std::path::Path, sources: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read source directory") {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                rust_sources_under(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
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
        let mut sources = Vec::new();
        rust_sources_under(&src_dir, &mut sources);

        for path in sources {
            let relative = path.strip_prefix(&src_dir).expect("source below src");
            let is_test_source = relative
                .components()
                .any(|component| component.as_os_str() == "tests")
                || relative
                    .file_name()
                    .is_some_and(|name| name == "smoke_tests.rs" || name == "test_support.rs");
            if relative == std::path::Path::new("clipboard.rs") || is_test_source {
                continue;
            }

            let source = std::fs::read_to_string(&path).expect("read Rust source");
            for forbidden in [".write_to_clipboard(", ".read_from_clipboard("] {
                assert!(
                    !source.contains(forbidden),
                    "{} accesses GPUI clipboard directly; use crate::clipboard instead",
                    relative.display()
                );
            }
        }
    }
}
