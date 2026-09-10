use super::TrashReceipt;
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use windows::{
    Win32::{Foundation::E_ABORT, Storage::FileSystem::MoveFileW, System::Com::*, UI::Shell::*},
    core::{HRESULT, PCWSTR, Ref, implement},
};

fn native(path: &Path) -> io::Result<Vec<u16>> {
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in file name",
        ));
    }
    name.push(0);
    Ok(name)
}

pub fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    let from = native(from)?;
    let to = native(to)?;
    // SAFETY: both strings are NUL-terminated and alive for the synchronous call.
    // MoveFileW never replaces an existing destination.
    unsafe { MoveFileW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr())) }
        .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xffff))
}

/// Read an entry's volume and file ID without following a reparse point.
pub fn file_identity(path: &Path) -> io::Result<(u64, u64)> {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Storage::FileSystem::*,
    };
    let path = native(path)?;
    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    // SAFETY: the native path remains alive, no data access is requested, and
    // the handle is closed on every path. Sharing permits editor/transfer I/O.
    let handle = OwnedHandle(
        unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(io::Error::other)?,
    );
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle.0, &mut info) }.map_err(io::Error::other)?;
    Ok((
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}

#[implement(IFileOperationProgressSink)]
struct Sink {
    receipt: Arc<Mutex<Option<PathBuf>>>,
}
#[allow(non_snake_case, unused_variables)]
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn PreDeleteItem(&self, flags: u32, _: Ref<IShellItem>) -> windows::core::Result<()> {
        // A permanent deletion callback is never authorized by Trash.
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
            return Err(E_ABORT.into());
        }
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        result: HRESULT,
        recycled: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        result.ok()?;
        let item = recycled
            .as_ref()
            .ok_or_else(|| windows::core::Error::from(E_ABORT))?;
        // SAFETY: COM owns item; GetDisplayName allocates with CoTaskMemAlloc.
        let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        let path = unsafe { OsString::from_wide(name.as_wide()) };
        unsafe { CoTaskMemFree(Some(name.as_ptr().cast())) };
        *self.receipt.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.into());
        Ok(())
    }
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hrresult: windows::core::HRESULT) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreRenameItem(
        &self,
        _dwflags: u32,
        _psiitem: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _dwflags: u32,
        _psiitem: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
        _hrrename: windows::core::HRESULT,
        _psinewlycreated: windows::core::Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _dwflags: u32,
        _psiitem: windows::core::Ref<IShellItem>,
        _psidestinationfolder: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _dwflags: u32,
        _psiitem: windows::core::Ref<IShellItem>,
        _psidestinationfolder: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
        _hrmove: windows::core::HRESULT,
        _psinewlycreated: windows::core::Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _dwflags: u32,
        _psiitem: windows::core::Ref<IShellItem>,
        _psidestinationfolder: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _dwflags: u32,
        _psiitem: windows::core::Ref<IShellItem>,
        _psidestinationfolder: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
        _hrcopy: windows::core::HRESULT,
        _psinewlycreated: windows::core::Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: windows::core::Ref<IShellItem>,
        _psznewname: &windows::core::PCWSTR,
        _psztemplatename: &windows::core::PCWSTR,
        _dwfileattributes: u32,
        _hrnew: windows::core::HRESULT,
        _psinewitem: windows::core::Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _iworktotal: u32, _iworksofar: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

pub fn trash_item(path: &Path) -> io::Result<TrashReceipt> {
    let path = native(path)?;
    let receipt = Arc::new(Mutex::new(None));
    // Use a dedicated STA: the filesystem worker may already belong to another
    // apartment. No COM interface escapes the thread.
    std::thread::spawn(move || -> io::Result<TrashReceipt> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(io::Error::other)?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        let _apartment = Apartment;
        let result = (|| -> windows::core::Result<()> {
            let sink: IFileOperationProgressSink = Sink {
                receipt: receipt.clone(),
            }
            .into();
            unsafe {
                let operation: IFileOperation =
                    CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
                operation.SetOperationFlags(
                    FOFX_RECYCLEONDELETE
                        | FOFX_EARLYFAILURE
                        | FOF_NOERRORUI
                        | FOF_SILENT
                        | FOF_NOCONFIRMATION
                        | FOF_NO_CONNECTED_ELEMENTS,
                )?;
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None)?;
                operation.DeleteItem(&item, &sink)?;
                operation.PerformOperations()?;
                if operation.GetAnyOperationsAborted()?.as_bool() {
                    return Err(E_ABORT.into());
                }
            }
            Ok(())
        })();
        result.map_err(io::Error::other)?;
        let item = receipt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| io::Error::other("Recycle Bin did not return an exact restore entry"))?;
        let name = item
            .file_name()
            .ok_or_else(|| io::Error::other("Missing recycled item name"))?
            .to_string_lossy();
        let info = name
            .strip_prefix("$R")
            .map(|tail| item.with_file_name(format!("$I{tail}")))
            .filter(|p| p.is_file());
        Ok(TrashReceipt { item, info })
    })
    .join()
    .map_err(|_| io::Error::other("Recycle Bin worker failed"))?
}
