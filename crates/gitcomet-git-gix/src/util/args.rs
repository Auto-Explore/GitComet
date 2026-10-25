//! Argument and path helpers: validation, path decoding, and stable hashing.

use super::*;

pub(crate) fn validate_ref_like_arg(value: &str, kind: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid {kind}: value is empty"
        ))));
    }
    if value.starts_with('-') {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid {kind}: values starting with '-' are not allowed"
        ))));
    }
    Ok(())
}

pub(crate) fn validate_hex_commit_id(id: &CommitId) -> Result<()> {
    let value = id.as_ref();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::new(ErrorKind::Backend(
            "invalid commit id: must contain only hexadecimal characters".to_string(),
        )));
    }
    Ok(())
}

pub(crate) fn path_buf_from_git_bytes(path_bytes: &[u8], context: &str) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let _ = context;
        Ok(PathBuf::from(OsStr::from_bytes(path_bytes)))
    }

    #[cfg(windows)]
    {
        let path_text = std::str::from_utf8(path_bytes).map_err(|_| {
            Error::new(ErrorKind::Backend(format!(
                "{context}: non-UTF-8 git path bytes are not representable on Windows",
            )))
        })?;
        Ok(PathBuf::from(path_text))
    }
}

/// Renders a path as a platform-stable byte sequence for hashing config keys.
///
/// Raw `OsStr` bytes on Unix, UTF-16 LE units on Windows, so the same
/// repository path yields the same key across processes and reboots.
pub(crate) fn stable_path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        path.as_os_str().as_bytes().to_vec()
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let mut bytes = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[cfg(not(any(unix, windows)))]
    {
        path.to_str()
            .map(|text| text.as_bytes().to_vec())
            .unwrap_or_else(|| format!("{path:?}").into_bytes())
    }
}

pub(crate) fn fnv1a_64(bytes: &[u8]) -> u64 {
    use std::hash::Hasher as _;
    let mut hash = fnv::FnvHasher::default();
    hash.write(bytes);
    hash.finish()
}

// Test helper: constructs a git stage:path blob spec for index stage testing.
#[cfg(test)]
pub(crate) fn git_stage_blob_spec(stage: u8, path: &Path) -> Result<OsString> {
    git_revision_with_path(&format!(":{stage}:"), path, "build conflict stage revision")
}

#[cfg(test)]
pub(super) fn git_revision_with_path(prefix: &str, path: &Path, context: &str) -> Result<OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        let _ = context;
        let path_bytes = path.as_os_str().as_bytes();
        let mut rev = Vec::with_capacity(prefix.len().saturating_add(path_bytes.len()));
        rev.extend_from_slice(prefix.as_bytes());
        rev.extend_from_slice(path_bytes);
        Ok(OsString::from_vec(rev))
    }

    #[cfg(windows)]
    {
        let path_text = path.to_str().ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "{context}: non-Unicode path cannot be represented for git command arguments",
            )))
        })?;
        Ok(OsString::from(format!(
            "{prefix}{}",
            path_text.replace('\\', "/")
        )))
    }
}
