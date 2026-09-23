//! Short-lived identities for memoized, already verified file contents.
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, Prefix};
use std::time::SystemTime;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_NAME_OPENED, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FileBasicInfo, FileIdInfo, GetDriveTypeW, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, GetVolumeInformationByHandleW, VOLUME_NAME_GUID,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_FILE_USN_DATA, FSCTL_WRITE_USN_CLOSE_RECORD,
    READ_FILE_USN_DATA, USN_JOURNAL_DATA_V0,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    pub volume: u64,
    pub file_id: u128,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub change_unix_nanos: i128,
    pub journal_id: u64,
    pub usn: i64,
}

fn journal_id(handle: HANDLE) -> Option<u64> {
    let mut journal = USN_JOURNAL_DATA_V0::default();
    let mut returned = 0;
    // A file handle supports this query on supported Windows versions. Do not
    // require elevation or enable a journal: unsupported access disables reuse.
    (unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            std::ptr::null(),
            0,
            (&mut journal as *mut USN_JOURNAL_DATA_V0).cast(),
            size_of::<USN_JOURNAL_DATA_V0>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    } != 0
        && returned as usize >= size_of::<USN_JOURNAL_DATA_V0>()
        && journal.UsnJournalID != 0)
        .then_some(journal.UsnJournalID)
}

fn file_usn(handle: HANDLE) -> Option<i64> {
    let versions = READ_FILE_USN_DATA {
        MinMajorVersion: 2,
        MaxMajorVersion: 2,
    };
    let mut record = [0u8; 1024];
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            handle,
            FSCTL_READ_FILE_USN_DATA,
            (&versions as *const READ_FILE_USN_DATA).cast(),
            size_of::<READ_FILE_USN_DATA>() as u32,
            record.as_mut_ptr().cast(),
            record.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
        || returned < 60
        || record[4..6] != 2u16.to_le_bytes()
    {
        return None;
    }
    let length = u32::from_le_bytes(record[..4].try_into().ok()?);
    if length < 60 || length > returned {
        return None;
    }
    let usn = i64::from_le_bytes(record[24..32].try_into().ok()?);
    (usn > 0).then_some(usn)
}

/// Keeps in-place writers excluded until content verification finishes:
/// Windows may defer timestamps until the last writer closes. Rename-over and
/// delete stay allowed, so a user's editor save or `git checkout` never fails
/// on this handle; a replaced path has a new file ID, which stamps detect.
/// Acquisition never waits for a sharing violation; callers use their normal
/// uncached path when this optimization is unavailable.
pub struct FileIdentityGuard {
    _file: File,
    identity: FileIdentity,
}

impl FileIdentityGuard {
    pub fn try_open(path: &Path) -> io::Result<Option<Self>> {
        if !path.is_absolute() {
            return Ok(None);
        }
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return Ok(None);
        };
        let drive = match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Ok(None),
        };
        let root = [u16::from(drive), b':' as u16, b'\\' as u16, 0];
        // DRIVE_FIXED: exclude network shares and mapped remote drives.
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
            return Ok(None);
        }
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let handle = file.as_raw_handle();
        // A local-looking parent can redirect into a network share. SMB has
        // no volume GUID, so require one from the opened handle itself. Very
        // long/unsupported names conservatively use content verification.
        let mut final_path = [0u16; 512];
        let final_len = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                final_path.as_mut_ptr(),
                final_path.len() as u32,
                FILE_NAME_OPENED | VOLUME_NAME_GUID,
            )
        };
        if final_len == 0 || final_len as usize >= final_path.len() {
            return Ok(None);
        }
        let mut filesystem = [0u16; 32];
        let local_ntfs = unsafe {
            GetVolumeInformationByHandleW(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            )
        } != 0
            && filesystem[..5] == [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0];
        if !local_ntfs {
            return Ok(None);
        }
        let mut basic = FILE_BASIC_INFO::default();
        let mut id = FILE_ID_INFO::default();
        // Both buffers have their documented size and remain alive for the call.
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
            || unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    FileIdInfo,
                    (&mut id as *mut FILE_ID_INFO).cast(),
                    size_of::<FILE_ID_INFO>() as u32,
                )
            } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if basic.FileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return Ok(None);
        }
        let metadata = file.metadata()?;
        let Some(journal) = journal_id(handle) else {
            return Ok(None);
        };
        let Some(usn) = file_usn(handle) else {
            return Ok(None);
        };
        if journal_id(handle) != Some(journal) {
            return Ok(None);
        }
        let identity = FileIdentity {
            volume: id.VolumeSerialNumber,
            file_id: u128::from_le_bytes(id.FileId.Identifier),
            len: metadata.len(),
            modified: metadata.modified().ok(),
            change_unix_nanos: (i128::from(basic.ChangeTime) - 116_444_736_000_000_000) * 100,
            journal_id: journal,
            usn,
        };
        Ok(Some(Self {
            _file: file,
            identity,
        }))
    }

    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Establish a new journal boundary before verifying bytes for reuse.
    /// USN reasons coalesce while other readers keep a file open. A close
    /// record resets that accumulation, so the next mapped write changes its
    /// USN even when neither timestamp changes. Only cache misses call this;
    /// unchanged cache hits query the journal without writing a record.
    pub fn seal_for_reuse(&mut self) -> bool {
        let handle = self._file.as_raw_handle();
        if journal_id(handle) != Some(self.identity.journal_id) {
            return false;
        }
        let mut usn = 0i64;
        let mut returned = 0;
        if unsafe {
            DeviceIoControl(
                handle,
                FSCTL_WRITE_USN_CLOSE_RECORD,
                std::ptr::null(),
                0,
                (&mut usn as *mut i64).cast(),
                size_of::<i64>() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
            || returned != size_of::<i64>() as u32
            || usn <= 0
            || journal_id(handle) != Some(self.identity.journal_id)
        {
            return false;
        }
        self.identity.usn = usn;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_WRITE, FlushViewOfFile, MapViewOfFile, PAGE_READWRITE,
        UnmapViewOfFile,
    };

    fn write_mapped(path: &Path, byte: u8) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        unsafe {
            let mapping = CreateFileMappingW(
                file.as_raw_handle(),
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                0,
                std::ptr::null(),
            );
            assert!(!mapping.is_null());
            let view = MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, 0);
            assert!(!view.Value.is_null());
            drop(file);
            let guard_excluded = FileIdentityGuard::try_open(path).is_err();
            *view.Value.cast::<u8>() = byte;
            let flushed = FlushViewOfFile(view.Value, 1) != 0;
            let unmapped = UnmapViewOfFile(view) != 0;
            let closed = CloseHandle(mapping) != 0;
            assert!(
                guard_excluded,
                "an existing writable mapping disables reuse"
            );
            assert!(flushed && unmapped && closed);
        }
    }

    #[test]
    fn held_identity_allows_rename_over_and_delete_but_not_writers() {
        let root = std::env::temp_dir().join(format!(
            "gitcomet-identity-sharing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("user.txt");
        std::fs::write(&path, b"before").unwrap();
        let Some(original) = FileIdentityGuard::try_open(&path).unwrap() else {
            // Unsupported filesystems/journal access deliberately disable reuse.
            std::fs::remove_dir_all(root).unwrap();
            return;
        };
        assert!(
            OpenOptions::new().write(true).open(&path).is_err(),
            "in-place writers stay excluded while the identity is read"
        );
        // An editor's atomic save renames a new file over the original.
        let replacement = root.join("replacement.txt");
        std::fs::write(&replacement, b"after").unwrap();
        std::fs::rename(&replacement, &path).expect("rename over a held file");
        assert_eq!(std::fs::read(&path).unwrap(), b"after");
        let replaced = FileIdentityGuard::try_open(&path).unwrap().unwrap();
        assert_ne!(replaced.identity().file_id, original.identity().file_id);
        // `git checkout` unlinks a file before writing its new version.
        std::fs::remove_file(&path).expect("delete a held file");
        drop(replaced);
        drop(original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sealed_identity_detects_repeated_mapped_writes_with_an_open_reader() {
        let root = std::env::temp_dir().join(format!(
            "gitcomet-mapped-identity-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("mapped.bin");
        std::fs::write(&path, [b'a'; 4096]).unwrap();
        let reader = File::open(&path).unwrap();
        let Some(mut guard) = FileIdentityGuard::try_open(&path).unwrap() else {
            // Unsupported filesystems/journal access deliberately disable reuse.
            drop(reader);
            std::fs::remove_dir_all(root).unwrap();
            return;
        };
        for byte in *b"bcd" {
            assert!(guard.seal_for_reuse());
            let before = guard.identity();
            drop(guard);
            assert_eq!(
                FileIdentityGuard::try_open(&path)
                    .unwrap()
                    .unwrap()
                    .identity(),
                before,
                "unchanged reads do not write another close record"
            );
            write_mapped(&path, byte);
            guard = FileIdentityGuard::try_open(&path).unwrap().unwrap();
            let after = guard.identity();
            assert_eq!(before.file_id, after.file_id);
            assert_eq!(before.len, after.len);
            assert_ne!(
                before.usn, after.usn,
                "each mapped write must invalidate reuse"
            );
            assert_eq!(std::fs::read(&path).unwrap()[0], byte);
        }
        drop(guard);
        drop(reader);
        std::fs::remove_dir_all(root).unwrap();
    }
}
