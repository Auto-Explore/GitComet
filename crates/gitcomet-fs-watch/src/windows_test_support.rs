//! Safe, deliberately conservative eligibility check for NTFS test cookies.
use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, GetDriveTypeW,
    GetFinalPathNameByHandleW, GetVolumeInformationByHandleW, GetVolumePathNameW,
};

/// False (or an error) means keep guarded settling. This does not establish
/// notification ordering or flush any data; callers still need closed writers.
pub fn is_local_ntfs(path: &Path) -> io::Result<bool> {
    for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        if fs::symlink_metadata(ancestor)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Ok(false);
        }
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let handle = directory.as_raw_handle();
    let mut filesystem = [0u16; 32];
    let mut resolved = vec![0u16; 32768];
    let mut volume = vec![0u16; 32768];
    // SAFETY: the directory owns a live handle throughout the calls. All output
    // buffers have the stated capacity; optional output pointers may be null.
    unsafe {
        if GetVolumeInformationByHandleW(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let len =
            GetFinalPathNameByHandleW(handle, resolved.as_mut_ptr(), resolved.len() as u32, 0);
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len as usize >= resolved.len() {
            return Ok(false);
        }
        if GetVolumePathNameW(resolved.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) == 0 {
            return Err(io::Error::last_os_error());
        }
        // DRIVE_FIXED = 3. Remote and removable media are not validated here.
        Ok(supported_volume(
            GetDriveTypeW(volume.as_ptr()),
            &filesystem,
        ))
    }
}

fn supported_volume(drive_type: u32, filesystem: &[u16]) -> bool {
    drive_type == 3 && filesystem.starts_with(&[78, 84, 70, 83, 0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_volume_is_not_eligible() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!is_local_ntfs(&temp.path().join("missing")).unwrap_or(false));
    }

    #[test]
    fn native_sync_rejects_remote_removable_and_unknown_filesystems() {
        let ntfs: Vec<_> = "NTFS\0".encode_utf16().collect();
        assert!(supported_volume(3, &ntfs));
        for drive_type in [0, 1, 2, 4, 5, 6] {
            assert!(!supported_volume(drive_type, &ntfs));
        }
        for name in ["", "NTFS", "ReFS\0", "FAT32\0", "unknown\0"] {
            assert!(!supported_volume(
                3,
                &name.encode_utf16().collect::<Vec<_>>()
            ));
        }
    }
}
