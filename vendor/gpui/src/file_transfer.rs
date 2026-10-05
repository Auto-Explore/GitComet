use crate::{ClipboardEntry, ClipboardItem, ExternalPaths};
use std::path::PathBuf;

/// Negotiated operation for a native file transfer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FileTransferOperation {
    /// Duplicate source items at the destination.
    #[default]
    Copy,
    /// Transfer ownership of source items after successful completion.
    Move,
}

/// Native files are distinct from strings that happen to contain paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileTransfer {
    /// Absolute, native source pathnames.
    pub paths: ExternalPaths,
    /// Requested operation before negotiation.
    pub operation: FileTransferOperation,
    /// Application-generated token, preserved only by a compatible owner.
    pub ownership: u64,
}

/// Native file clipboard format identifier.
pub const FILE_TRANSFER_MIME: &str = "application/x-gpui-file-transfer";
/// Native file clipboard format identifier.
pub const COPIED_FILES_MIME: &str = "x-special/gnome-copied-files";
/// Native file clipboard format identifier.
pub const URI_LIST_MIME: &str = "text/uri-list";
/// Native file clipboard format identifier.
pub const KDE_CUT_MIME: &str = "application/x-kde-cutselection";

impl FileTransfer {
    /// Encode local file URLs with native path escaping.
    pub fn uri_list(&self) -> Vec<u8> {
        self.paths
            .paths()
            .iter()
            .filter_map(|path| url::Url::from_file_path(path).ok())
            .map(|url| format!("{url}\r\n"))
            .collect::<String>()
            .into_bytes()
    }

    /// Encode one of the advertised file clipboard formats.
    pub fn encode(&self, mime: &str) -> Option<Vec<u8>> {
        let operation = if self.operation == FileTransferOperation::Move {
            "cut"
        } else {
            "copy"
        };
        let mut out = match mime {
            URI_LIST_MIME => vec![],
            COPIED_FILES_MIME => format!("{operation}\n").into_bytes(),
            FILE_TRANSFER_MIME => format!("{operation}\n{}\n", self.ownership).into_bytes(),
            KDE_CUT_MIME => {
                return Some(
                    if self.operation == FileTransferOperation::Move {
                        b"1"
                    } else {
                        b"0"
                    }
                    .to_vec(),
                );
            }
            _ => return None,
        };
        if mime == COPIED_FILES_MIME {
            // Nautilus splits on LF and rejects empty lines, including a
            // trailing newline. This is distinct from RFC text/uri-list.
            out.extend(
                self.paths
                    .paths()
                    .iter()
                    .filter_map(|path| url::Url::from_file_path(path).ok())
                    .map(|url| url.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .as_bytes(),
            );
        } else {
            out.extend(self.uri_list());
        }
        Some(out)
    }

    /// Decode only local file URLs in a typed native format.
    pub fn decode(bytes: &[u8], mime: &str) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.lines();
        let operation = match mime {
            URI_LIST_MIME => FileTransferOperation::Copy,
            FILE_TRANSFER_MIME | COPIED_FILES_MIME => match lines.next()? {
                "copy" => FileTransferOperation::Copy,
                "cut" => FileTransferOperation::Move,
                _ => return None,
            },
            _ => return None,
        };
        let ownership = if mime == FILE_TRANSFER_MIME {
            lines.next()?.parse().ok()?
        } else {
            0
        };
        let paths = lines
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let url = url::Url::parse(line).ok()?;
                if url.scheme() != "file"
                    || url
                        .host_str()
                        .is_some_and(|h| !h.is_empty() && h != "localhost")
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return None;
                }
                url.to_file_path()
                    .ok()
                    .filter(|p: &PathBuf| p.is_absolute())
            })
            .collect::<Option<Vec<_>>>()?;
        (!paths.is_empty()).then(|| Self {
            paths: ExternalPaths(paths.into_iter().collect()),
            operation,
            ownership,
        })
    }
}

impl ClipboardItem {
    /// Extract typed files, keeping ordinary text distinct.
    pub fn file_transfer(&self) -> Option<FileTransfer> {
        self.entries.iter().find_map(|entry| match entry {
            ClipboardEntry::Files(files) => Some(files.clone()),
            ClipboardEntry::ExternalPaths(paths) => Some(FileTransfer {
                paths: paths.clone(),
                operation: FileTransferOperation::Copy,
                ownership: 0,
            }),
            _ => None,
        })
    }
}

/// A completed native transfer. `None` means cancellation or rejection.
#[derive(Clone, Debug)]
pub struct FileTransferCompletion {
    /// The source items supplied when the transfer began.
    pub files: FileTransfer,
    /// The operation actually negotiated with the receiving application.
    pub operation: Option<FileTransferOperation>,
    /// True when the receiver owns source removal (e.g. a Shell optimized move).
    pub source_removed: bool,
}

static COMPLETIONS: std::sync::Mutex<Vec<FileTransferCompletion>> =
    std::sync::Mutex::new(Vec::new());
/// Ownership tokens with a native receiver still extracting them, counted per
/// token: tokens are application-chosen and several transfers may share one
/// (every untagged payload uses 0), so one finishing must not hide the others.
static ACTIVE_TRANSFERS: std::sync::Mutex<std::collections::BTreeMap<u64, usize>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());
impl FileTransfer {
    /// Native adapters mark asynchronous extraction before releasing their drag
    /// loop, and clear it exactly once when that extraction ends. Reporting a
    /// completion does not clear it.
    pub fn set_active(&self, active: bool) {
        let mut transfers = ACTIVE_TRANSFERS.lock().unwrap_or_else(|e| e.into_inner());
        if active {
            *transfers.entry(self.ownership).or_default() += 1;
        } else if let Some(count) = transfers.get_mut(&self.ownership) {
            *count -= 1;
            if *count == 0 {
                transfers.remove(&self.ownership);
            }
        }
    }
}
impl FileTransferCompletion {
    /// A native file transfer whose receiver performs any source removal.
    pub fn receiver_performed(
        files: FileTransfer,
        operation: Option<FileTransferOperation>,
    ) -> Self {
        Self {
            files,
            operation,
            source_removed: operation == Some(FileTransferOperation::Move),
        }
    }

    /// A transfer that was cancelled, rejected, or never started.
    pub fn cancelled(files: FileTransfer) -> Self {
        Self::receiver_performed(files, None)
    }

    /// Native adapters report completion after their protocol's final event.
    pub fn report(self) {
        COMPLETIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(self);
    }
}
/// Whether a transfer with this ownership token is still in flight.
pub(crate) fn transfer_is_active(ownership: u64) -> bool {
    ACTIVE_TRANSFERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&ownership)
}

impl crate::App {
    /// Whether a native receiver is still extracting this clipboard payload.
    pub fn file_transfer_is_active(&self, ownership: u64) -> bool {
        transfer_is_active(ownership)
    }

    /// Includes completed transfers whose source cleanup has not been consumed.
    pub fn has_pending_native_file_transfers(&self) -> bool {
        !ACTIVE_TRANSFERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
            || !COMPLETIONS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
    }
    /// Drain native transfer results. Callers retain ownership tokens to match
    /// results to their source snapshots and perform any required source cleanup.
    pub fn take_file_transfer_completions(&mut self) -> Vec<FileTransferCompletion> {
        std::mem::take(&mut *COMPLETIONS.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Takes the reported completions that `matches` selects and leaves the rest
/// queued. The queue is process-wide and tests run in parallel, so a test must
/// not take or count completions that other tests reported.
#[cfg(test)]
pub(crate) fn take_completions_where(
    matches: impl Fn(&FileTransferCompletion) -> bool,
) -> Vec<FileTransferCompletion> {
    COMPLETIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extract_if(.., |completion| matches(completion))
        .collect()
}

/// A native payload may need metadata prepared on a background worker.
pub enum ExternalDragPayloadResolution {
    /// Available without background work.
    Ready(Option<crate::ExternalDragPayload>),
    /// The pointer gesture remains cancellable while this task runs.
    Pending(crate::Task<Option<crate::ExternalDragPayload>>),
}

/// A lease on the clipboard data object that supplied a paste. Keeping this
/// object alive lets platforms report completion to the original owner even
/// when another application replaces the clipboard during the transfer.
#[derive(Clone)]
pub struct FilePaste(std::rc::Rc<FilePasteState>);

struct FilePasteState {
    completion: std::cell::RefCell<Option<Box<dyn FnOnce(Option<FileTransferOperation>)>>>,
}

impl std::fmt::Debug for FilePaste {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilePaste")
            .field("pending", &self.0.completion.borrow().is_some())
            .finish()
    }
}

impl FilePaste {
    /// Constructed by native clipboard adapters.
    pub fn new(completion: impl FnOnce(Option<FileTransferOperation>) + 'static) -> Self {
        Self(std::rc::Rc::new(FilePasteState {
            completion: std::cell::RefCell::new(Some(Box::new(completion))),
        }))
    }

    /// Report a fully completed paste. `None` preserves a cancelled or partial
    /// transfer. Move means the receiver already removed the source items.
    pub fn complete(self, operation: Option<FileTransferOperation>) {
        let completion = self.0.completion.borrow_mut().take();
        if let Some(completion) = completion {
            completion(operation);
        }
    }
}

impl Drop for FilePasteState {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.get_mut().take() {
            completion(None);
        }
    }
}

/// The native drop remains pending until the receiving application finishes
/// extracting the files. Dropping this lease cancels the transfer.
#[derive(Clone, Debug)]
pub struct FileDropTransfer {
    /// The operation agreed with the source application.
    pub operation: FileTransferOperation,
    /// Whether this particular source guarantees removal after successful Move.
    /// Local file URL transfers normally delegate the move to the receiver;
    /// a protocol completion alone does not imply source-side deletion.
    pub source_owns_move: bool,
    /// A completion tied to the original offer or data object.
    pub completion: FilePaste,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipboardString, FileDragPaths, TestAppContext};
    use std::{cell::RefCell, rc::Rc};

    /// An absolute path in a temporary directory on every platform.
    fn path(name: &str) -> PathBuf {
        PathBuf::from(if cfg!(windows) { "C:/tmp" } else { "/tmp" }).join(name)
    }

    /// The file URL that names `path(name)`.
    fn file_url(name: &str) -> String {
        url::Url::from_file_path(path(name)).unwrap().to_string()
    }

    fn files(names: &[&str], operation: FileTransferOperation, ownership: u64) -> FileTransfer {
        FileTransfer {
            paths: ExternalPaths(names.iter().map(|name| path(name)).collect()),
            operation,
            ownership,
        }
    }

    fn transfer(name: &str, ownership: u64) -> FileTransfer {
        files(&[name], FileTransferOperation::Move, ownership)
    }

    /// Records every outcome a paste or drop lease reports.
    fn recorded_paste() -> (FilePaste, Rc<RefCell<Vec<Option<FileTransferOperation>>>>) {
        let outcomes = Rc::new(RefCell::new(Vec::new()));
        let paste = FilePaste::new({
            let outcomes = outcomes.clone();
            move |operation| outcomes.borrow_mut().push(operation)
        });
        (paste, outcomes)
    }

    #[test]
    fn nautilus_payload_has_lf_separators_and_no_empty_lines() {
        let files = files(&["a", "b"], FileTransferOperation::Copy, 7);
        assert_eq!(
            files.encode(COPIED_FILES_MIME).unwrap(),
            format!("copy\n{}\n{}", file_url("a"), file_url("b")).as_bytes()
        );
        assert_eq!(
            files.uri_list(),
            format!("{}\r\n{}\r\n", file_url("a"), file_url("b")).as_bytes()
        );
    }

    #[test]
    fn typed_formats_roundtrip_operation_and_paths() {
        for operation in [FileTransferOperation::Copy, FileTransferOperation::Move] {
            let original = files(&["a b#%.txt", "日本語", "folder"], operation, 0xdead_beef);
            let decode = |mime| FileTransfer::decode(&original.encode(mime).unwrap(), mime);
            assert_eq!(decode(FILE_TRANSFER_MIME), Some(original.clone()));
            // GNOME's format carries the operation but no ownership token.
            assert_eq!(
                decode(COPIED_FILES_MIME),
                Some(FileTransfer {
                    ownership: 0,
                    ..original.clone()
                })
            );
            // A plain URI list always offers untagged copies.
            assert_eq!(
                decode(URI_LIST_MIME),
                Some(FileTransfer {
                    operation: FileTransferOperation::Copy,
                    ownership: 0,
                    ..original.clone()
                })
            );
        }
    }

    #[test]
    fn kde_cut_marker_follows_the_operation() {
        let marker = |operation| files(&["a"], operation, 1).encode(KDE_CUT_MIME);
        assert_eq!(marker(FileTransferOperation::Move), Some(b"1".to_vec()));
        assert_eq!(marker(FileTransferOperation::Copy), Some(b"0".to_vec()));
    }

    #[test]
    fn unknown_formats_are_neither_encoded_nor_decoded() {
        let files = files(&["a"], FileTransferOperation::Copy, 1);
        assert_eq!(files.encode("text/plain"), None);
        assert_eq!(
            FileTransfer::decode(file_url("a").as_bytes(), "text/plain"),
            None
        );
        // KDE's marker names no files, so it cannot be read on its own.
        assert_eq!(FileTransfer::decode(b"1", KDE_CUT_MIME), None);
    }

    #[test]
    fn malformed_typed_payloads_decode_to_none() {
        let url = file_url("a");
        for (payload, mime) in [
            (String::new(), FILE_TRANSFER_MIME),
            (format!("move\n1\n{url}"), FILE_TRANSFER_MIME),
            (format!("copy\n{url}"), FILE_TRANSFER_MIME),
            (format!("copy\n-1\n{url}"), FILE_TRANSFER_MIME),
            ("copy\n1\n".to_string(), FILE_TRANSFER_MIME),
            (String::new(), COPIED_FILES_MIME),
            (format!("paste\n{url}"), COPIED_FILES_MIME),
            ("cut".to_string(), COPIED_FILES_MIME),
            (String::new(), URI_LIST_MIME),
            ("# only a comment\r\n".to_string(), URI_LIST_MIME),
        ] {
            assert_eq!(
                FileTransfer::decode(payload.as_bytes(), mime),
                None,
                "{mime}: {payload:?}"
            );
        }
        assert_eq!(FileTransfer::decode(&[0xff, 0xfe], URI_LIST_MIME), None);
    }

    #[test]
    fn uri_lists_tolerate_crlf_comments_blank_lines_and_localhost() {
        let localhost = file_url("b").replacen("file://", "file://localhost", 1);
        let payload = format!(
            "# copied by a test\r\n{}\r\n\r\n{localhost}\r\n",
            file_url("a")
        );
        let decoded = FileTransfer::decode(payload.as_bytes(), URI_LIST_MIME).unwrap();
        assert_eq!(decoded.paths.paths(), [path("a"), path("b")]);
    }

    #[test]
    fn file_urls_with_queries_or_fragments_are_rejected() {
        for suffix in ["?version=2", "#section"] {
            let payload = format!("{}{suffix}", file_url("a"));
            assert_eq!(
                FileTransfer::decode(payload.as_bytes(), URI_LIST_MIME),
                None,
                "{payload}"
            );
        }
    }

    #[test]
    fn special_characters_are_percent_encoded() {
        let list = files(&["a b#%.txt", "日本"], FileTransferOperation::Copy, 0).uri_list();
        let list = String::from_utf8(list).unwrap();
        assert!(list.contains("/a%20b%23%25.txt\r\n"), "{list}");
        assert!(list.contains("/%E6%97%A5%E6%9C%AC\r\n"), "{list}");
    }

    #[test]
    fn paths_without_a_file_url_are_left_out() {
        // A relative path names no file on its own, so no receiver could open it.
        let mut files = files(&["a"], FileTransferOperation::Copy, 0);
        files.paths.0.insert(0, PathBuf::from("relative/b"));
        assert_eq!(
            files.uri_list(),
            format!("{}\r\n", file_url("a")).as_bytes()
        );
    }

    #[test]
    fn paste_receipt_reports_completion_once_and_drop_reports_cancellation() {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = events.clone();
        let receipt = FilePaste::new(move |operation| observed.borrow_mut().push(operation));
        let retained = receipt.clone();
        drop(receipt);
        assert!(events.borrow().is_empty());
        retained.complete(Some(FileTransferOperation::Move));
        let observed = events.clone();
        drop(FilePaste::new(move |operation| {
            observed.borrow_mut().push(operation)
        }));
        assert_eq!(*events.borrow(), [Some(FileTransferOperation::Move), None]);
    }

    #[test]
    fn paste_receipt_keeps_the_first_outcome() {
        let (paste, outcomes) = recorded_paste();
        let other = paste.clone();
        let observer = paste.clone();
        assert!(format!("{observer:?}").contains("pending: true"));
        paste.complete(Some(FileTransferOperation::Copy));
        other.complete(Some(FileTransferOperation::Move));
        assert!(format!("{observer:?}").contains("pending: false"));
        drop(observer);
        assert_eq!(*outcomes.borrow(), [Some(FileTransferOperation::Copy)]);

        let (paste, outcomes) = recorded_paste();
        paste.complete(None);
        assert_eq!(
            *outcomes.borrow(),
            [None],
            "an explicit cancel is reported once"
        );
    }

    #[test]
    fn native_names_roundtrip_without_treating_text_as_files() {
        let files = FileTransfer {
            paths: ExternalPaths(
                [PathBuf::from(if cfg!(windows) {
                    "C:/tmp/a b#%.txt"
                } else {
                    "/tmp/a b\n#%.txt"
                })]
                .into_iter()
                .collect(),
            ),
            operation: FileTransferOperation::Move,
            ownership: 19,
        };
        assert_eq!(
            FileTransfer::decode(
                &files.encode(FILE_TRANSFER_MIME).unwrap(),
                FILE_TRANSFER_MIME
            ),
            Some(files)
        );
        assert!(FileTransfer::decode(b"/tmp/file", URI_LIST_MIME).is_none());
        assert!(FileTransfer::decode(b"file://host/remote", URI_LIST_MIME).is_none());
        assert!(
            ClipboardItem::new_string("/tmp/file".into())
                .file_transfer()
                .is_none()
        );
    }

    #[test]
    fn clipboard_items_yield_their_first_file_entry() {
        let tagged = files(&["tagged"], FileTransferOperation::Move, 5);
        let plain = ExternalPaths([path("plain")].into_iter().collect());
        let text = ClipboardEntry::String(ClipboardString::new("text".into()));
        let item = |entries| ClipboardItem { entries };
        assert_eq!(
            item(vec![
                text.clone(),
                ClipboardEntry::Files(tagged.clone()),
                ClipboardEntry::ExternalPaths(plain.clone()),
            ])
            .file_transfer(),
            Some(tagged.clone())
        );
        // Paths that other applications offered are untagged copies.
        assert_eq!(
            item(vec![
                ClipboardEntry::ExternalPaths(plain.clone()),
                ClipboardEntry::Files(tagged),
            ])
            .file_transfer(),
            Some(FileTransfer {
                paths: plain,
                operation: FileTransferOperation::Copy,
                ownership: 0,
            })
        );
        assert_eq!(item(vec![text]).file_transfer(), None);
        assert_eq!(item(vec![]).file_transfer(), None);
    }

    #[test]
    fn copied_files_still_paste_as_text() {
        // Regression: platforms now read file clipboards as `ClipboardEntry::Files`
        // instead of `ExternalPaths`. `ClipboardItem::text()` is what every
        // text consumer uses (including Wayland's `text/plain` send path), so
        // both entry kinds must render identically, one path per line.
        let files = files(&["a", "b"], FileTransferOperation::Move, 3);
        let as_external = ClipboardItem {
            entries: vec![ClipboardEntry::ExternalPaths(files.paths.clone())],
        };
        let as_files = ClipboardItem {
            entries: vec![ClipboardEntry::Files(files)],
        };
        assert_eq!(
            as_external.text(),
            Some(format!("{}\n{}", path("a").display(), path("b").display()))
        );
        assert_eq!(as_files.text(), as_external.text());
    }

    #[test]
    fn text_entries_take_precedence_over_paths() {
        let item = ClipboardItem {
            entries: vec![
                ClipboardEntry::Files(files(&["a"], FileTransferOperation::Copy, 0)),
                ClipboardEntry::String(ClipboardString::new("label".into())),
            ],
        };
        assert_eq!(item.text(), Some("label".to_string()));
    }

    #[test]
    fn drag_paths_carry_the_requested_transfer() {
        let drag = FileDragPaths::new([(path("file"), false), (path("folder"), true)]);
        assert_eq!(drag.operation, FileTransferOperation::Copy);
        assert_eq!(drag.ownership, 0);
        assert_eq!(
            drag.entries(),
            [(path("file"), false), (path("folder"), true)]
        );
        assert_eq!(
            drag.with_transfer(FileTransferOperation::Move, 9)
                .transfer(),
            files(&["file", "folder"], FileTransferOperation::Move, 9)
        );
    }

    #[test]
    fn cancelled_completions_keep_their_files() {
        let files = transfer("cancelled", 11);
        let completion = FileTransferCompletion::cancelled(files.clone());
        assert_eq!(completion.files, files);
        assert_eq!(completion.operation, None);
        assert!(!completion.source_removed);
    }

    #[test]
    fn receiver_owned_moves_never_request_source_cleanup() {
        for operation in [
            None,
            Some(FileTransferOperation::Copy),
            Some(FileTransferOperation::Move),
        ] {
            let files = transfer("receiver-owned", 12);
            let completion = FileTransferCompletion::receiver_performed(files.clone(), operation);
            assert_eq!(completion.files, files);
            assert_eq!(completion.operation, operation);
            assert_eq!(
                completion.source_removed,
                operation == Some(FileTransferOperation::Move)
            );
        }
    }

    #[test]
    fn transfer_is_active_until_its_extraction_ends() {
        let files = transfer("a", 0x5f1a_2b3c_4d5e_6f70);
        files.set_active(true);
        assert!(transfer_is_active(files.ownership));
        // Completion is reported separately from activity: a Windows paste can
        // report before `EndOperation`, and a drag that never started reports
        // without ever having been active.
        FileTransferCompletion {
            files: files.clone(),
            operation: Some(FileTransferOperation::Move),
            source_removed: false,
        }
        .report();
        assert!(transfer_is_active(files.ownership));
        files.set_active(false);
        assert!(!transfer_is_active(files.ownership));
    }

    #[test]
    fn overlapping_transfers_sharing_a_token_stay_active_until_the_last_one_ends() {
        // Untagged payloads all use token 0. Two pastes of such payloads can
        // overlap on Windows, where each one runs `StartOperation` and
        // `EndOperation`; the first ending must not report the second finished.
        let first = transfer("first", 0);
        let second = transfer("second", 0);
        first.set_active(true);
        second.set_active(true);
        first.set_active(false);
        assert!(transfer_is_active(0), "second transfer is still extracting");
        // Completions of transfers that were never active do not end it either.
        FileTransferCompletion::cancelled(transfer("third", 0)).report();
        assert!(transfer_is_active(0));
        second.set_active(false);
        assert!(!transfer_is_active(0));
        // Ending an inactive transfer is harmless.
        second.set_active(false);
        assert!(!transfer_is_active(0));
    }

    #[test]
    fn activity_is_tracked_per_token() {
        let first = transfer("first-token", 0x7e57_0001);
        let second = transfer("second-token", 0x7e57_0002);
        first.set_active(true);
        assert!(transfer_is_active(first.ownership));
        assert!(!transfer_is_active(second.ownership));
        first.set_active(false);
        assert!(!transfer_is_active(first.ownership));
    }

    #[crate::test]
    fn pending_transfers_include_extractions_and_unconsumed_results(cx: &mut TestAppContext) {
        let files = transfer("pending", 0x7e57_0003);
        files.set_active(true);
        assert!(cx.update(|cx| cx.file_transfer_is_active(files.ownership)));
        assert!(cx.update(|cx| cx.has_pending_native_file_transfers()));
        files.set_active(false);
        assert!(!cx.update(|cx| cx.file_transfer_is_active(files.ownership)));

        FileTransferCompletion::cancelled(files.clone()).report();
        assert!(cx.update(|cx| cx.has_pending_native_file_transfers()));
        let taken = take_completions_where(|completion| completion.files == files);
        assert_eq!(taken.len(), 1);
        assert!(take_completions_where(|completion| completion.files == files).is_empty());
    }

    #[test]
    fn non_file_uri_lists_decode_to_none_so_callers_can_fall_back() {
        // Browsers advertise `text/uri-list` for ordinary links. Clipboard
        // backends must be able to tell "not a file payload" apart from
        // "clipboard is broken", so decode returns None rather than erroring.
        for payload in [
            b"https://example.com/page".as_slice(),
            b"mailto:someone@example.com".as_slice(),
            b"file:///tmp/ok\r\nhttps://example.com/mixed".as_slice(),
        ] {
            assert!(
                FileTransfer::decode(payload, URI_LIST_MIME).is_none(),
                "expected {:?} to be rejected as a file payload",
                std::str::from_utf8(payload),
            );
        }
        assert!(FileTransfer::decode(file_url("ok").as_bytes(), URI_LIST_MIME).is_some());
    }
}
