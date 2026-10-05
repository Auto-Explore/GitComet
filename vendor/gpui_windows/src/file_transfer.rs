//! Shell data object wrapper retaining transfer-result formats for the caller.
use crate::bindings::Windows::Win32::*;
use gpui::{FileTransfer, FileTransferCompletion, FileTransferOperation};
use std::{cell::RefCell, os::windows::ffi::OsStrExt, path::Path, rc::Rc, sync::LazyLock};
use windows_core::{BOOL, Error, GUID, HRESULT, Interface, PCWSTR, Ref, Result, implement};

const EFFECT_COPY: u32 = DROPEFFECT_COPY as u32;
const EFFECT_MOVE: u32 = DROPEFFECT_MOVE as u32;

fn effect_of(operation: FileTransferOperation) -> u32 {
    if operation == FileTransferOperation::Move {
        EFFECT_MOVE
    } else {
        EFFECT_COPY
    }
}

fn transfer_operation(effect: u32) -> Option<FileTransferOperation> {
    if effect == EFFECT_MOVE {
        Some(FileTransferOperation::Move)
    } else if effect == EFFECT_COPY {
        Some(FileTransferOperation::Copy)
    } else {
        None
    }
}

/// The operation an effect reports. A drop into the Recycle Bin is a move
/// whatever effect the Shell reports for it, as long as it reports one.
fn performed_operation(effect: u32, recycle_bin: bool) -> Option<FileTransferOperation> {
    if recycle_bin && effect != 0 {
        Some(FileTransferOperation::Move)
    } else {
        transfer_operation(effect)
    }
}

static PREFERRED: LazyLock<u16> = LazyLock::new(|| unsafe {
    RegisterClipboardFormatW(windows_core::w!("Preferred DropEffect")) as u16
});
static PERFORMED: LazyLock<u16> = LazyLock::new(|| unsafe {
    RegisterClipboardFormatW(windows_core::w!("Performed DropEffect")) as u16
});
static LOGICAL: LazyLock<u16> = LazyLock::new(|| unsafe {
    RegisterClipboardFormatW(windows_core::w!("Logical Performed DropEffect")) as u16
});
static PASTED: LazyLock<u16> = LazyLock::new(|| unsafe {
    RegisterClipboardFormatW(windows_core::w!("Paste Succeeded")) as u16
});
static PRIVATE: LazyLock<u16> = LazyLock::new(|| unsafe {
    RegisterClipboardFormatW(windows_core::w!("application/x-gpui-file-transfer")) as u16
});
static TARGET_CLSID: LazyLock<u16> =
    LazyLock::new(|| unsafe { RegisterClipboardFormatW(windows_core::w!("TargetCLSID")) as u16 });

#[derive(Default)]
struct TransferResult {
    performed: Option<u32>,
    logical: Option<u32>,
    reported: bool,
    async_mode: bool,
    in_operation: bool,
    pasted: Option<u32>,
    drag_effect: Option<u32>,
    recycle_bin: bool,
}

impl TransferResult {
    fn reset(&mut self) {
        *self = Self {
            async_mode: self.async_mode,
            ..Default::default()
        };
    }
}

// Both the shared result and the wrapped Shell object belong to the UI apartment.
#[implement(IDataObject, IDataObjectAsyncCapability, Agile = false)]
struct FileDataObject {
    inner: IDataObject,
    files: FileTransfer,
    result: Rc<RefCell<TransferResult>>,
    clipboard: bool,
}

/// Forwards an inner result unchanged. `HRESULT::ok` would collapse success
/// codes such as `S_FALSE` or `DATA_S_SAMEFORMATETC` into `S_OK`.
fn forward(result: HRESULT) -> Result<()> {
    if result == S_OK {
        Ok(())
    } else {
        Err(Error::from_hresult(result))
    }
}

#[allow(non_snake_case)]
impl IDataObject_Impl for FileDataObject_Impl {
    fn GetData(&self, f: *const FORMATETC) -> Result<STGMEDIUM> {
        unsafe { self.inner.GetData(f) }
    }
    fn GetDataHere(&self, f: *const FORMATETC, m: *mut STGMEDIUM) -> Result<()> {
        forward(unsafe { self.inner.GetDataHere(f, m) })
    }
    fn QueryGetData(&self, f: *const FORMATETC) -> Result<()> {
        forward(unsafe { self.inner.QueryGetData(f) })
    }
    fn GetCanonicalFormatEtc(&self, f: *const FORMATETC, out: *mut FORMATETC) -> Result<()> {
        forward(unsafe { self.inner.GetCanonicalFormatEtc(f, out) })
    }
    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> {
        unsafe { self.inner.EnumFormatEtc(direction) }
    }
    fn DAdvise(&self, f: *const FORMATETC, flags: u32, sink: Ref<IAdviseSink>) -> Result<u32> {
        unsafe { self.inner.DAdvise(f, flags, sink.as_ref()) }
    }
    fn DUnadvise(&self, connection: u32) -> Result<()> {
        forward(unsafe { self.inner.DUnadvise(connection) })
    }
    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        unsafe { self.inner.EnumDAdvise() }
    }
    fn SetData(&self, f: *const FORMATETC, medium: *const STGMEDIUM, release: BOOL) -> Result<()> {
        if f.is_null() || medium.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // The caller owns valid FORMATETC/STGMEDIUM arguments for this COM call.
        // Read before forwarding: the underlying object may consume the medium.
        let format = unsafe { (*f).cfFormat.0 };
        let recycle_bin = unsafe {
            if format == *TARGET_CLSID
                && (*medium).tymed == TYMED_HGLOBAL as u32
                && GlobalSize((*medium).Anonymous.hGlobal) >= std::mem::size_of::<GUID>()
            {
                let ptr = GlobalLock((*medium).Anonymous.hGlobal);
                if ptr.is_null() {
                    false
                } else {
                    let clsid = std::ptr::read_unaligned(ptr.cast::<GUID>());
                    let _ = GlobalUnlock((*medium).Anonymous.hGlobal);
                    clsid == CLSID_RecycleBin
                }
            } else {
                false
            }
        };
        let value = unsafe {
            if (*medium).tymed == TYMED_HGLOBAL as u32
                && GlobalSize((*medium).Anonymous.hGlobal) >= 4
            {
                let ptr = GlobalLock((*medium).Anonymous.hGlobal);
                if ptr.is_null() {
                    None
                } else {
                    let value = std::ptr::read_unaligned(ptr.cast::<u32>());
                    let _ = GlobalUnlock((*medium).Anonymous.hGlobal);
                    Some(value)
                }
            } else {
                None
            }
        };
        unsafe {
            forward(self.inner.SetData(f, medium, release.as_bool()))?;
        }
        if let Some(value) = value {
            let mut state = self.result.borrow_mut();
            if self.clipboard
                && state.reported
                && !state.in_operation
                && [*PERFORMED, *LOGICAL, *TARGET_CLSID].contains(&format)
            {
                // Synchronous pastes have no StartOperation. Their first result
                // metadata begins a fresh attempt after the previous PasteSucceeded.
                // Repeated PasteSucceeded notifications still finish only once.
                state.reset();
            }
            state.recycle_bin |= recycle_bin;
            if format == *PERFORMED {
                state.performed = Some(value);
            }
            if format == *LOGICAL {
                state.logical = Some(value);
            }
            if format == *PASTED {
                state.pasted = Some(value);
            }
            if self.clipboard && format == *PASTED && !state.reported && !state.in_operation {
                state.reported = true;
                // Both PasteSucceeded=MOVE and Performed=MOVE are required
                // before a clipboard source must delete its originals.
                FileTransferCompletion {
                    files: self.files.clone(),
                    operation: performed_operation(value, state.recycle_bin),
                    source_removed: !state.recycle_bin && state.performed != Some(EFFECT_MOVE),
                }
                .report();
            }
        }
        Ok(())
    }
}

impl FileDataObject {
    /// Ends the extraction `StartOperation` began, exactly once.
    fn end_operation(&self, state: &mut TransferResult) {
        if state.in_operation {
            state.in_operation = false;
            self.files.set_active(false);
        }
    }
}

#[allow(non_snake_case)]
impl IDataObjectAsyncCapability_Impl for FileDataObject_Impl {
    fn SetAsyncMode(&self, enabled: BOOL) -> Result<()> {
        self.result.borrow_mut().async_mode = enabled.as_bool();
        Ok(())
    }
    fn GetAsyncMode(&self) -> Result<BOOL> {
        Ok(self.result.borrow().async_mode.into())
    }
    fn StartOperation(&self, _: Ref<IBindCtx>) -> Result<()> {
        let mut state = self.result.borrow_mut();
        if !state.in_operation {
            if self.clipboard {
                state.reset();
            }
            state.in_operation = true;
            self.files.set_active(true);
        }
        Ok(())
    }
    fn InOperation(&self) -> Result<BOOL> {
        Ok(self.result.borrow().in_operation.into())
    }
    fn EndOperation(&self, result: HRESULT, _: Ref<IBindCtx>, effects: u32) -> Result<()> {
        let mut state = self.result.borrow_mut();
        self.end_operation(&mut state);
        if !state.reported {
            state.reported = true;
            let logical = if self.clipboard {
                state.pasted
            } else {
                state.logical.or(state.drag_effect).or(Some(effects))
            };
            let operation = if result.is_ok() {
                logical.and_then(|effect| performed_operation(effect, state.recycle_bin))
            } else {
                None
            };
            FileTransferCompletion {
                files: self.files.clone(),
                operation,
                source_removed: !state.recycle_bin && effects != EFFECT_MOVE,
            }
            .report();
        }
        Ok(())
    }
}

impl Drop for FileDataObject {
    fn drop(&mut self) {
        let mut state = self.result.borrow_mut();
        // A target that began extracting and vanished never calls EndOperation.
        self.end_operation(&mut state);
        if !state.reported {
            FileTransferCompletion::cancelled(self.files.clone()).report();
        }
    }
}

fn set_bytes(object: &IDataObject, format: u16, bytes: &[u8]) -> Result<()> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE as u32, bytes.len());
        if memory.is_invalid() {
            return Err(Error::from_thread());
        }
        let ptr = GlobalLock(memory);
        if ptr.is_null() {
            let _ = GlobalFree(memory);
            return Err(E_OUTOFMEMORY.into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
        let _ = GlobalUnlock(memory);
        let format = FORMATETC {
            cfFormat: CLIPFORMAT(format),
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT as u32,
            lindex: -1,
            tymed: TYMED_HGLOBAL as u32,
        };
        let medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL as u32,
            Anonymous: uSTGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        };
        if let Err(error) = object.SetData(&format, &medium, true).ok() {
            let _ = GlobalFree(memory);
            return Err(error);
        }
    }
    Ok(())
}

fn data_object(
    files: FileTransfer,
    clipboard: bool,
) -> Result<(IDataObject, Rc<RefCell<TransferResult>>)> {
    struct IdLists(Vec<LPITEMIDLIST>);
    impl Drop for IdLists {
        fn drop(&mut self) {
            for list in &self.0 {
                unsafe { CoTaskMemFree(list.cast()) };
            }
        }
    }
    let mut lists = IdLists(Vec::new());
    for path in files.paths.paths() {
        let native = shell_path(path)?;
        let mut pidl = std::ptr::null_mut();
        unsafe {
            SHParseDisplayName(
                PCWSTR(native.as_ptr()),
                None::<&IBindCtx>,
                &mut pidl,
                SFGAOF(0),
                None,
            )
            .ok()?;
        }
        lists.0.push(pidl);
    }
    let items = unsafe {
        SHCreateShellItemArrayFromIDLists(
            &lists.0.iter().map(|p| p.cast_const()).collect::<Vec<_>>(),
        )?
    };
    let inner: IDataObject = unsafe { items.BindToHandler(None::<&IBindCtx>, &BHID_DataObject)? };
    set_bytes(
        &inner,
        *PREFERRED,
        &effect_of(files.operation).to_le_bytes(),
    )?;
    if let Some(bytes) = files.encode(gpui::FILE_TRANSFER_MIME) {
        set_bytes(&inner, *PRIVATE, &bytes)?;
    }
    let result = Rc::new(RefCell::new(TransferResult {
        async_mode: true,
        ..Default::default()
    }));
    let object = FileDataObject {
        inner,
        files,
        result: result.clone(),
        clipboard,
    }
    .into();
    Ok((object, result))
}

/// Shell parsing names use backslashes and DOS/UNC prefixes, even when a
/// filesystem path came from `canonicalize`. Work in UTF-16 to retain native names.
fn shell_path(path: &Path) -> Result<Vec<u16>> {
    let mut native: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .map(|c| if c == b'/' as u16 { b'\\' as u16 } else { c })
        .collect();
    if native.contains(&0) {
        return Err(E_INVALIDARG.into());
    }
    match native.as_slice() {
        // \\?\UNC\server\share -> \\server\share
        [92, 92, 63, 92, 85, 78, 67, 92, ..] => {
            native.drain(2..8);
        }
        // \\?\C:\path -> C:\path; other device namespaces stay intact.
        [92, 92, 63, 92, drive, 58, 92, ..]
            if (b'A' as u16..=b'Z' as u16).contains(drive)
                || (b'a' as u16..=b'z' as u16).contains(drive) =>
        {
            native.drain(..4);
        }
        _ => {}
    }
    native.push(0);
    Ok(native)
}

/// Places the files on the clipboard, with `text` for plain-text consumers.
pub(crate) fn write_files(files: FileTransfer, text: Option<String>) -> Result<()> {
    let (object, _) = data_object(files, true)?;
    if let Some(text) = text {
        let wide: Vec<u8> = text
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        set_bytes(&object, CF_UNICODETEXT as u16, &wide)?;
    }
    unsafe { OleSetClipboard(&object).ok() }
}

/// Runs the modal drag loop. `None` means the target extracts asynchronously
/// and the data object reports the result when that ends.
pub(crate) fn drag_files(window: HWND, files: FileTransfer) -> Option<FileTransferCompletion> {
    let Ok((object, result)) = data_object(files.clone(), false) else {
        return Some(FileTransferCompletion::cancelled(files));
    };
    let allowed = EFFECT_COPY | effect_of(files.operation);
    let effect = unsafe { SHDoDragDrop(Some(window), &object, None::<&IDropSource>, allowed) }.ok();
    let mut state = result.borrow_mut();
    state.drag_effect = effect;
    if state.in_operation || state.reported {
        return None;
    }
    // Reported here: releasing the data object must not add a cancellation.
    state.reported = true;
    let Some(effect) = effect else {
        return Some(FileTransferCompletion::cancelled(files));
    };
    let logical = state.logical.unwrap_or(effect);
    Some(FileTransferCompletion {
        files,
        operation: performed_operation(logical, state.recycle_bin),
        source_removed: !state.recycle_bin
            && (effect != EFFECT_MOVE || state.performed.is_some_and(|p| p != EFFECT_MOVE)),
    })
}

/// Shell completion is delivered to the captured IDataObject, never to whichever
/// application happens to own the clipboard after the filesystem worker ends.
pub(crate) fn capture_paste(files: &FileTransfer) -> Option<gpui::FilePaste> {
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let object = unsafe { OleGetClipboard() }.ok()?;
    if crate::clipboard::read_from_clipboard()?
        .file_transfer()
        .as_ref()
        != Some(files)
        || unsafe { GetClipboardSequenceNumber() } != sequence
    {
        return None;
    }
    Some(gpui::FilePaste::new(move |operation| {
        let Some(operation) = operation else {
            return;
        };
        let logical = effect_of(operation);
        // Our filesystem service has already moved the originals. Reporting
        // an optimized move prevents the source from deleting them again.
        let performed = if operation == FileTransferOperation::Move {
            0u32
        } else {
            EFFECT_COPY
        };
        let result = set_bytes(&object, *PERFORMED, &performed.to_le_bytes())
            .and_then(|_| set_bytes(&object, *LOGICAL, &logical.to_le_bytes()))
            .and_then(|_| set_bytes(&object, *PASTED, &logical.to_le_bytes()));
        if let Err(error) = result {
            log::error!("Could not report file paste completion: {error}");
        }
    }))
}

/// Retain the Shell source until asynchronous file extraction finishes. Our
/// receiver performs an optimized move and must never ask it to delete again.
pub(crate) fn capture_drop(
    object: &IDataObject,
    operation: FileTransferOperation,
) -> gpui::FileDropTransfer {
    let object = object.clone();
    let asynchronous =
        object
            .cast::<IDataObjectAsyncCapability>()
            .ok()
            .filter(|capability| unsafe {
                capability
                    .GetAsyncMode()
                    .is_ok_and(|enabled| enabled.as_bool())
                    && capability.StartOperation(None::<&IBindCtx>).is_ok()
            });
    gpui::FileDropTransfer {
        operation,
        source_owns_move: false,
        completion: gpui::FilePaste::new(move |completed| {
            let logical = completed.map_or(0, effect_of);
            let performed = if completed == Some(FileTransferOperation::Copy) {
                EFFECT_COPY
            } else {
                0
            };
            let result = set_bytes(&object, *PERFORMED, &performed.to_le_bytes())
                .and_then(|_| set_bytes(&object, *LOGICAL, &logical.to_le_bytes()));
            if let Err(error) = result {
                log::error!("Could not report file drop completion: {error}");
            }
            if let Some(capability) = asynchronous {
                let result = if completed.is_some() { S_OK } else { E_ABORT };
                if let Err(error) =
                    unsafe { capability.EndOperation(result, None::<&IBindCtx>, performed) }.ok()
                {
                    log::error!("Could not finish asynchronous file drop: {error}");
                }
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

    struct Apartment;

    impl Apartment {
        fn new() -> Self {
            unsafe { OleInitialize(std::ptr::null()).ok().unwrap() };
            Self
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { OleUninitialize() };
        }
    }

    fn files(path: PathBuf, ownership: u64) -> FileTransfer {
        FileTransfer {
            paths: gpui::ExternalPaths([path].into_iter().collect()),
            operation: FileTransferOperation::Move,
            ownership,
        }
    }

    #[test]
    fn shell_accepts_forward_slashes_and_canonicalized_paths() {
        let _apartment = Apartment::new();
        let executable = std::env::current_exe().unwrap();
        for path in [
            executable.clone(),
            PathBuf::from(executable.to_str().unwrap().replace('\\', "/")),
            std::fs::canonicalize(&executable).unwrap(),
        ] {
            assert!(
                data_object(files(path.clone(), 0), false).is_ok(),
                "{path:?}"
            );
        }
    }

    #[test]
    fn shell_paths_preserve_native_names_and_unc_shares() {
        for (input, expected) in [
            (r"C:/dir/file", r"C:\dir\file"),
            (r"\\?\C:\dir\file", r"C:\dir\file"),
            (r"\\?\UNC\server\share\file", r"\\server\share\file"),
            (r"\\server/share/file", r"\\server\share\file"),
        ] {
            let mut input: Vec<u16> = input.encode_utf16().collect();
            let mut expected: Vec<u16> = expected.encode_utf16().collect();
            // NTFS filenames can contain an unpaired UTF-16 surrogate.
            input.push(0xd800);
            expected.extend([0xd800, 0]);
            assert_eq!(
                shell_path(Path::new(&OsString::from_wide(&input))).unwrap(),
                expected
            );
        }
        assert!(shell_path(Path::new(&OsString::from_wide(&[65, 0, 66]))).is_err());
    }

    #[test]
    fn data_object_uses_apartment_marshaling() {
        let _apartment = Apartment::new();
        let (object, _) = data_object(files(std::env::current_exe().unwrap(), 0), false).unwrap();
        // Neither IAgileObject nor the free-threaded IMarshal may be exposed.
        for iid in [
            GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90),
            GUID::from_u128(0x00000003_0000_0000_c000_000000000046),
        ] {
            let mut interface = std::ptr::null_mut();
            assert_eq!(
                unsafe { object.query(&iid, &mut interface) },
                HRESULT(0x80004002u32 as i32)
            );
            assert!(interface.is_null());
        }
    }

    #[gpui::test]
    fn clipboard_reports_each_paste_with_fresh_effects(cx: &mut gpui::TestAppContext) {
        let _apartment = Apartment::new();
        let files = files(std::env::current_exe().unwrap(), 0x7e57_1001);
        let (object, state) = data_object(files.clone(), true).unwrap();
        let asynchronous = object.cast::<IDataObjectAsyncCapability>().unwrap();
        let expect = |operation, source_removed| {
            let completions = cx
                .update(|cx| cx.take_file_transfer_completions())
                .into_iter()
                .filter(|completion| completion.files == files)
                .collect::<Vec<_>>();
            assert_eq!(completions.len(), 1);
            assert_eq!(completions[0].operation, operation);
            assert_eq!(completions[0].source_removed, source_removed);
        };
        for (result, effect, pasted, operation, removed) in [
            (
                S_OK,
                EFFECT_COPY,
                Some(EFFECT_COPY),
                Some(FileTransferOperation::Copy),
                true,
            ),
            (E_ABORT, EFFECT_MOVE, Some(EFFECT_MOVE), None, false),
            (
                S_OK,
                EFFECT_MOVE,
                Some(EFFECT_MOVE),
                Some(FileTransferOperation::Move),
                false,
            ),
            (
                S_OK,
                0,
                Some(EFFECT_MOVE),
                Some(FileTransferOperation::Move),
                true,
            ),
            // No PasteSucceeded in this attempt: the previous move must not leak.
            (S_OK, EFFECT_MOVE, None, None, false),
        ] {
            unsafe { asynchronous.StartOperation(None::<&IBindCtx>).ok().unwrap() };
            assert!(state.borrow().performed.is_none());
            assert!(state.borrow().pasted.is_none());
            assert!(!state.borrow().recycle_bin);
            // A repeated StartOperation must not count another extraction.
            unsafe { asynchronous.StartOperation(None::<&IBindCtx>).ok().unwrap() };
            set_bytes(&object, *PERFORMED, &effect.to_le_bytes()).unwrap();
            if let Some(pasted) = pasted {
                set_bytes(&object, *PASTED, &pasted.to_le_bytes()).unwrap();
            }
            unsafe {
                asynchronous
                    .EndOperation(result, None::<&IBindCtx>, effect)
                    .ok()
                    .unwrap();
                asynchronous
                    .EndOperation(result, None::<&IBindCtx>, effect)
                    .ok()
                    .unwrap();
                assert!(asynchronous.GetAsyncMode().unwrap().as_bool());
            }
            expect(operation, removed);
        }
        assert!(!cx.update(|cx| cx.file_transfer_is_active(files.ownership)));

        // Synchronous targets report through SetData without StartOperation.
        for (effect, pasted) in [
            (EFFECT_COPY, EFFECT_COPY),
            (EFFECT_MOVE, 0),
            (EFFECT_MOVE, EFFECT_MOVE),
            (0, EFFECT_MOVE),
        ] {
            set_bytes(&object, *PERFORMED, &effect.to_le_bytes()).unwrap();
            set_bytes(&object, *PASTED, &pasted.to_le_bytes()).unwrap();
            set_bytes(&object, *PASTED, &pasted.to_le_bytes()).unwrap();
            expect(transfer_operation(pasted), effect != EFFECT_MOVE);
        }
    }
}
