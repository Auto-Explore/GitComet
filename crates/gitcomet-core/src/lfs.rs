//! Git LFS pointer files (spec: git-lfs `docs/spec.md`). [`parse_pointer`] is
//! strict, so a text file that merely mentions LFS never shows as a pointer;
//! [`parse_pointer_like_git_lfs`] takes what git-lfs itself uploads.

use std::fmt;
use std::path::{Path, PathBuf};

/// git-lfs reads only blobs shorter than this as pointers.
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
        crate::hex::decode_array(hex).map(Self)
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
/// order, LF endings, shorter than [`POINTER_MAX_BYTES`]. Extension keys are
/// tolerated; `oid sha256:<64 hex>` and `size <n>` are required.
pub fn parse_pointer(bytes: &[u8]) -> Option<LfsPointer> {
    if bytes.len() >= POINTER_MAX_BYTES || !looks_like_pointer(bytes) {
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

/// Spec URLs git-lfs reads, its alpha one included.
const GIT_LFS_READ_VERSIONS: [&str; 3] = ["http://git-media.io/v/2", LEGACY_SPEC_URL, SPEC_URL];
/// git-lfs only parses text that contains one of these.
const GIT_LFS_MARKERS: [&[u8]; 3] = [b"git-media", b"hawser", b"git-lfs"];
const GIT_LFS_KEYS: [&[u8]; 3] = [b"version", b"oid", b"size"];

/// Parse a pointer as git-lfs reads one (`lfs/pointer.go`, `DecodeFrom`),
/// which takes more than it writes: white space around the text, CRLF, blank
/// lines, extension lines before `version`, signed or zero-padded sizes and
/// the alpha spec URL. Its `pre-push` hook uploads what this accepts.
pub fn parse_pointer_like_git_lfs(bytes: &[u8]) -> Option<LfsPointer> {
    if bytes.len() >= POINTER_MAX_BYTES {
        return None;
    }
    let text = trim_space(bytes);
    if !GIT_LFS_MARKERS
        .iter()
        .any(|marker| memchr::memmem::find(text, marker).is_some())
    {
        return None;
    }
    let mut values: [Option<&[u8]>; 3] = [None; 3];
    let mut found = 0;
    let mut extensions: Vec<(&[u8], &[u8])> = Vec::new();
    for line in text.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let space = memchr::memchr(b' ', line)?;
        let (key, value) = (&line[..space], &line[space + 1..]);
        // Nothing follows `size`, extensions included.
        if found == GIT_LFS_KEYS.len() {
            return None;
        }
        if key == GIT_LFS_KEYS[found] {
            values[found] = Some(value);
            found += 1;
            continue;
        }
        extension_priority(key)?;
        extensions.retain(|(seen, _)| *seen != key);
        extensions.push((key, value));
    }
    let [Some(version), Some(oid), Some(size)] = values else {
        return None;
    };
    if !GIT_LFS_READ_VERSIONS
        .iter()
        .any(|known| known.as_bytes() == version)
    {
        return None;
    }
    let oid = sha256_oid(oid)?;
    let size: i64 = std::str::from_utf8(size).ok()?.parse().ok()?;
    let mut priorities = [false; 10];
    for (key, value) in extensions {
        sha256_oid(value)?;
        if std::mem::replace(&mut priorities[extension_priority(key)?], true) {
            return None;
        }
    }
    Some(LfsPointer {
        oid,
        size: u64::try_from(size).ok()?,
    })
}

fn sha256_oid(value: &[u8]) -> Option<LfsOid> {
    LfsOid::from_hex(std::str::from_utf8(value.strip_prefix(b"sha256:")?).ok()?)
}

/// The digit of an `ext-<digit>-<name>` key, prefix-matched as git-lfs's
/// `\Aext-\d{1}-\w+` is.
fn extension_priority(key: &[u8]) -> Option<usize> {
    match key.strip_prefix(b"ext-")? {
        [digit @ b'0'..=b'9', b'-', word, ..] if word.is_ascii_alphanumeric() || *word == b'_' => {
            Some(usize::from(digit - b'0'))
        }
        _ => None,
    }
}

/// Go's `bytes.TrimSpace`: Unicode white space off both ends, stopping at
/// invalid UTF-8.
fn trim_space(bytes: &[u8]) -> &[u8] {
    let leading = bytes.utf8_chunks().next().map_or(0, |chunk| {
        chunk.valid().len() - chunk.valid().trim_start().len()
    });
    let bytes = &bytes[leading..];
    let trailing = bytes
        .utf8_chunks()
        .last()
        .filter(|chunk| chunk.invalid().is_empty())
        .map_or(0, |chunk| {
            chunk.valid().len() - chunk.valid().trim_end().len()
        });
    &bytes[..bytes.len() - trailing]
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
        // git-lfs treats a 1024-byte blob as content.
        let padded = |length: usize| {
            let short = pointer(&format!("ext-0-pad x\n{good}"));
            let pad = "x".repeat(length - short.len() + 1);
            pointer(&format!("ext-0-pad {pad}\n{good}"))
        };
        assert_eq!(padded(1023).len(), 1023);
        assert!(parse_pointer(&padded(1023)).is_some());
        assert!(parse_pointer(&padded(1024)).is_none());
    }

    /// Each case matches what `git lfs push --dry-run` (git-lfs 3.8.1) lists.
    #[test]
    fn reads_pointers_as_git_lfs_does() {
        let v = format!("version {SPEC_URL}");
        let oid = format!("oid sha256:{OID}");
        let padded = |length: usize| {
            let mut text = format!("{v}\n{oid}\nsize 9\n").into_bytes();
            text.resize(length, b' ');
            text
        };
        let accepted = [
            format!("{v}\r\n{oid}\r\nsize 9\r\n"),
            format!("{v}\n{oid}\nsize 0009\n"),
            format!("{v}\n{oid}\nsize +9\n"),
            format!("{v}\n{oid}\nsize -0\n"),
            format!("\n{v}\n\n{oid}\nsize 9\n\n"),
            format!("\u{a0}{v}\n{oid}\nsize 9 \n"),
            format!("ext-0-foo sha256:{OID}\n{v}\n{oid}\nsize 9\n"),
            format!("{v}\next-1-a sha256:{OID}\n{oid}\nsize 9\n"),
            format!("version http://git-media.io/v/2\n{oid}\nsize 9\n"),
            format!("version {LEGACY_SPEC_URL}\n{oid}\nsize 9"),
        ];
        for text in accepted
            .iter()
            .map(String::as_bytes)
            .chain([&padded(1023)[..]])
        {
            let parsed = parse_pointer_like_git_lfs(text);
            assert_eq!(
                parsed.map(|pointer| pointer.oid.to_hex()).as_deref(),
                Some(OID),
                "{:?}",
                String::from_utf8_lossy(text)
            );
        }
        assert_eq!(
            parse_pointer_like_git_lfs(accepted[1].as_bytes()).map(|pointer| pointer.size),
            Some(9)
        );

        let rejected = [
            String::new(),
            " \n\t\n".to_string(),
            format!("{v}\n{oid}\nsize 9\nfoo bar\n"),
            format!("{v}\n{oid}\nsize 9\next-1-a sha256:{OID}\n"),
            format!("{v}\next-1-a sha256:{OID}\next-1-b sha256:{OID}\n{oid}\nsize 9\n"),
            format!("{v}\n{oid}\nsize\t9\n"),
            format!("{v}\n{oid}\nsize -9\n"),
            format!("{v}\n{oid}\nsize 9223372036854775808\n"),
            format!("{v}\noid sha256:{}\nsize 9\n", OID.to_uppercase()),
            format!("{oid}\n{v}\nsize 9\n"),
        ];
        for text in rejected
            .iter()
            .map(String::as_bytes)
            .chain([&padded(1024)[..]])
        {
            assert!(
                parse_pointer_like_git_lfs(text).is_none(),
                "{:?}",
                String::from_utf8_lossy(text)
            );
        }
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
