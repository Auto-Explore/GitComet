//! Git LFS pointer files (spec: git-lfs `docs/spec.md`). Parsing is strict so
//! a text file that merely mentions LFS is never mistaken for a pointer.

use std::fmt;
use std::path::{Path, PathBuf};

/// Pointers are at most this long; anything larger is content.
pub const POINTER_MAX_BYTES: usize = 1024;
pub const SPEC_URL: &str = "https://git-lfs.github.com/spec/v1";
/// Pre-1.0 spec URL that git-lfs still accepts.
const LEGACY_SPEC_URL: &str = "https://hawser.github.com/spec/v1";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LfsOid(pub [u8; 32]);

impl LfsOid {
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        crate::hex::decode(hex)?.try_into().ok().map(Self)
    }

    pub fn to_hex(&self) -> String {
        crate::hex::encode(&self.0)
    }
}

impl fmt::Display for LfsOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LfsPointer {
    pub oid: LfsOid,
    pub size: u64,
}

/// Cheap pre-check before reading a whole blob.
pub fn looks_like_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs") || bytes.starts_with(b"version https://hawser")
}

/// Parse a pointer: `version` first, then `key value` lines in ascending key
/// order, LF endings, at most [`POINTER_MAX_BYTES`]. Extension keys are
/// tolerated; `oid sha256:<64 hex>` and `size <n>` are required.
pub fn parse_pointer(bytes: &[u8]) -> Option<LfsPointer> {
    if bytes.len() > POINTER_MAX_BYTES || !looks_like_pointer(bytes) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines = body.split('\n');
    let version = lines.next()?.strip_prefix("version ")?;
    if version != SPEC_URL && version != LEGACY_SPEC_URL {
        return None;
    }
    let (mut oid, mut size, mut previous_key) = (None, None, "");
    for line in lines {
        let (key, value) = line.split_once(' ')?;
        let key_ok = !key.is_empty()
            && key
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'));
        if !key_ok || key <= previous_key || value.is_empty() || value.contains('\r') {
            return None;
        }
        previous_key = key;
        match key {
            "oid" => oid = Some(LfsOid::from_hex(value.strip_prefix("sha256:")?)?),
            "size" => {
                if value.len() > 1 && value.starts_with('0') {
                    return None;
                }
                size = Some(value.parse().ok()?);
            }
            _ => {}
        }
    }
    Some(LfsPointer {
        oid: oid?,
        size: size?,
    })
}

/// Location of an object below the LFS storage directory.
pub fn object_relative_path(oid: &LfsOid) -> PathBuf {
    let hex = oid.to_hex();
    Path::new("objects")
        .join(&hex[0..2])
        .join(&hex[2..4])
        .join(hex)
}

/// `lfs.storage` when set (relative values live under the common git dir,
/// like git-lfs), else `<common git dir>/lfs`.
pub fn storage_dir(lfs_storage: Option<&Path>, common_git_dir: &Path) -> PathBuf {
    match lfs_storage.filter(|path| !path.as_os_str().is_empty()) {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => common_git_dir.join(path),
        None => common_git_dir.join("lfs"),
    }
}

/// Parse one `GIT_LFS_PROGRESS` line:
/// `<direction> <current>/<total files> <downloaded>/<total bytes> <name>`.
pub fn parse_progress_line(line: &str) -> Option<crate::git_operation::TransferProgress> {
    let mut parts = line.trim_end_matches(['\r', '\n']).splitn(4, ' ');
    let direction = parts.next().filter(|d| !d.is_empty())?.to_string();
    let pair = |text: Option<&str>| -> Option<(u64, u64)> {
        let (done, total) = text?.split_once('/')?;
        Some((done.parse().ok()?, total.parse().ok()?))
    };
    let (files_done, files_total) = pair(parts.next())?;
    let (bytes_done, bytes_total) = pair(parts.next())?;
    Some(crate::git_operation::TransferProgress {
        direction,
        files_done,
        files_total,
        bytes_done,
        bytes_total,
        name: parts.next().unwrap_or_default().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        let progress =
            parse_progress_line("download 3/10 12000000/40000000 art/hero psd.psd\n").unwrap();
        assert_eq!(
            (
                progress.direction.as_str(),
                progress.files_done,
                progress.files_total
            ),
            ("download", 3, 10)
        );
        assert_eq!(
            (progress.bytes_done, progress.bytes_total),
            (12_000_000, 40_000_000)
        );
        assert_eq!(progress.name, "art/hero psd.psd");
        assert_eq!(
            progress.summary(),
            "LFS download 3/10 files · 12 MB of 40 MB"
        );
        assert!(parse_progress_line("download 3/x 1/2 a").is_none());
        assert!(parse_progress_line("").is_none());
    }

    const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

    fn pointer(body: &str) -> Vec<u8> {
        format!("version {SPEC_URL}\n{body}").into_bytes()
    }

    #[test]
    fn parses_the_spec_example() {
        let parsed = parse_pointer(&pointer(&format!("oid sha256:{OID}\nsize 12345\n"))).unwrap();
        assert_eq!(parsed.size, 12345);
        assert_eq!(parsed.oid.to_hex(), OID);
    }

    #[test]
    fn tolerates_extension_keys_in_order_and_a_missing_final_newline() {
        let body = format!("ext-0-foo sha256:{OID}\noid sha256:{OID}\nsize 1");
        assert_eq!(parse_pointer(&pointer(&body)).unwrap().size, 1);
    }

    #[test]
    fn rejects_malformed_pointers() {
        let good = format!("oid sha256:{OID}\nsize 5\n");
        let cases = [
            format!("size 5\noid sha256:{OID}\n"),     // keys out of order
            format!("oid sha256:{OID}\n"),             // missing size
            format!("oid sha256:{OID}\r\nsize 5\r\n"), // CRLF
            format!("oid sha256:{}\nsize 5\n", &OID[1..]), // short oid
            format!("oid sha1:{OID}\nsize 5\n"),       // wrong hash method
            format!("oid sha256:{OID}\nsize 05\n"),    // leading zero
            format!("oid sha256:{}\nsize 5\n", OID.to_uppercase()),
        ];
        for body in cases {
            assert!(parse_pointer(&pointer(&body)).is_none(), "{body:?}");
        }
        assert!(parse_pointer(format!("version other\n{good}").as_bytes()).is_none());
        let mut oversized = pointer(&good);
        oversized.extend(std::iter::repeat_n(b'x', POINTER_MAX_BYTES));
        assert!(parse_pointer(&oversized).is_none());
    }

    #[test]
    fn object_path_uses_two_level_fanout() {
        let oid = LfsOid::from_hex(OID).unwrap();
        assert_eq!(
            object_relative_path(&oid),
            Path::new("objects").join("4d").join("7a").join(OID)
        );
    }

    #[test]
    fn storage_dir_follows_lfs_storage_config() {
        let git = Path::new("/repo/.git");
        assert_eq!(storage_dir(None, git), git.join("lfs"));
        assert_eq!(storage_dir(Some(Path::new("big")), git), git.join("big"));
        let absolute = std::env::temp_dir().join("lfs-store");
        assert_eq!(storage_dir(Some(&absolute), git), absolute);
    }
}
