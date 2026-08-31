use super::*;

pub fn path_storage_key(path: &Path) -> String {
    if let Some(text) = path.to_str() {
        return text.to_string();
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        let bytes = path.as_os_str().as_bytes();
        let mut out = String::with_capacity(SESSION_PATH_BYTES_PREFIX.len() + bytes.len() * 2);
        out.push_str(SESSION_PATH_BYTES_PREFIX);
        out.push_str(&hex_encode(bytes));
        out
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let mut raw = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            raw.extend_from_slice(&unit.to_le_bytes());
        }
        let mut out = String::with_capacity(SESSION_PATH_WIDE_PREFIX.len() + raw.len() * 2);
        out.push_str(SESSION_PATH_WIDE_PREFIX);
        out.push_str(&hex_encode(&raw));
        out
    }

    #[cfg(not(any(unix, windows)))]
    {
        path.display().to_string()
    }
}

pub fn path_storage_key_shared(path: &Path) -> Arc<str> {
    if let Some(text) = path.to_str() {
        return Arc::from(text);
    }

    Arc::from(path_storage_key(path))
}

pub fn path_from_storage_key(raw: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        if let Some(hex) = raw.strip_prefix(SESSION_PATH_BYTES_PREFIX)
            && let Some(bytes) = hex_decode(hex)
        {
            return PathBuf::from(OsString::from_vec(bytes));
        }
    }

    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt as _;

        if let Some(hex) = raw.strip_prefix(SESSION_PATH_WIDE_PREFIX)
            && let Some(bytes) = hex_decode(hex)
            && bytes.len() % 2 == 0
        {
            let mut wide = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.as_chunks::<2>().0 {
                wide.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            return PathBuf::from(OsString::from_wide(&wide));
        }
    }

    PathBuf::from(raw)
}

pub(super) use gitcomet_core::hex::decode as hex_decode;
pub(super) use gitcomet_core::hex::encode as hex_encode;
