use super::TrashReceipt;
use objc2_foundation::{NSFileManager, NSURL};
use std::{
    ffi::{CStr, CString, OsString},
    io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::Path,
    ptr::NonNull,
};

pub fn trash_item(path: &Path) -> io::Result<TrashReceipt> {
    let path_c = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    // SAFETY: the owned, NUL-terminated native pathname outlives NSURL creation.
    // Foundation copies it. The returned NSURL owns its filesystem representation.
    let url = unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
            NonNull::new(path_c.as_ptr().cast_mut()).unwrap(),
            path.is_dir(),
            None,
        )
    };
    let mut resulting = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
        .map_err(|e| io::Error::other(e.to_string()))?;
    let resulting =
        resulting.ok_or_else(|| io::Error::other("Trash did not return a restore URL"))?;
    let native = unsafe { CStr::from_ptr(resulting.fileSystemRepresentation().as_ptr()) };
    Ok(TrashReceipt {
        item: OsString::from_vec(native.to_bytes().to_vec()).into(),
        info: None,
    })
}
