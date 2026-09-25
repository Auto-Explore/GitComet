//! git-annex keys as they appear in git: a locked file is a symlink into
//! `.git/annex/objects/…/<KEY>/<KEY>`, an unlocked file is a pointer file
//! whose first line is `/annex/objects/<KEY>`.

use std::sync::Arc;

/// Pointer files longer than this are content (git-annex's own rule).
pub const POINTER_MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AnnexKey {
    pub raw: Arc<str>,
    pub backend: Arc<str>,
    /// From the `-s<bytes>` field, when the backend recorded it.
    pub size: Option<u64>,
}

/// `BACKEND[-sSIZE][-mMTIME][-Sn-Cn]--NAME`, e.g. `SHA256E-s10--abc.bin`.
pub fn parse_key(raw: &str) -> Option<AnnexKey> {
    if raw.contains('/') || raw.chars().any(char::is_control) {
        return None;
    }
    let (fields, name) = raw.split_once("--")?;
    if name.is_empty() {
        return None;
    }
    let mut fields = fields.split('-');
    let backend = fields.next()?;
    if backend.is_empty() || !backend.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let mut size = None;
    for field in fields {
        // Each field is a one-letter tag followed by digits (`s10`, `m1700…`).
        let (kind, value) = field.split_at_checked(1)?;
        if !kind.bytes().all(|b| b.is_ascii_alphabetic())
            || value.is_empty()
            || !value.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        if kind == "s" {
            size = Some(value.parse().ok()?);
        }
    }
    Some(AnnexKey {
        raw: Arc::from(raw),
        backend: Arc::from(backend),
        size,
    })
}

/// Key of a locked annexed file from its symlink target.
pub fn key_from_symlink_target(target: &[u8]) -> Option<AnnexKey> {
    let target = std::str::from_utf8(target).ok()?.replace('\\', "/");
    if !target.contains("annex/objects/") {
        return None;
    }
    let mut parts = target.rsplit('/');
    let key = parts.next()?;
    // The object sits in a directory named after its own key.
    if parts.next()? != key {
        return None;
    }
    parse_key(key)
}

/// Key of an unlocked annexed file from its pointer bytes.
pub fn key_from_pointer(bytes: &[u8]) -> Option<AnnexKey> {
    if bytes.len() > POINTER_MAX_BYTES || !bytes.starts_with(b"/annex/objects/") {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    // Further lines are allowed only if they look like annex paths.
    if lines.any(|line| !line.contains("/annex/") || !line.ends_with('\n')) {
        return None;
    }
    let key = first
        .strip_prefix("/annex/objects/")?
        .trim_end_matches('\n')
        .trim_end_matches('\r');
    parse_key(key)
}

/// Where git-annex may keep a key's content, relative to `.git/annex/objects`:
/// `hashdirmixed` (non-bare repos) first, then `hashdirlower` (bare and
/// crippled-filesystem repos), as git-annex itself checks both. `levels` is 2,
/// or 1 under `annex.tune.objecthash1`.
pub fn object_paths(key: &str, levels: usize) -> [std::path::PathBuf; 2] {
    use md5::{Digest as _, Md5};
    let digest = Md5::digest(key.as_bytes());
    // hashDirMixed: the digest's first four bytes as a little-endian word,
    // six base-32 digits in swapped pairs, two characters per level.
    const CHARS: &[u8; 32] = b"0123456789zqjxkmvwgpfZQJXKMVWGPF";
    let word = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    let digits: Vec<u8> = (0..6)
        .map(|i| CHARS[((word >> (6 * i)) & 31) as usize])
        .collect();
    let mixed: Vec<u8> = digits
        .chunks(2)
        .flat_map(|pair| [pair[1], pair[0]])
        .collect();
    // hashDirLower: hex digest, three characters per level.
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let levels = levels.clamp(1, 2);
    let mixed = std::str::from_utf8(&mixed).unwrap_or_default();
    let path = |width: usize, digits: &str| {
        (0..levels)
            .map(|level| &digits[level * width..(level + 1) * width])
            .chain([key, key])
            .collect()
    };
    [path(2, mixed), path(3, &hex)]
}

/// Split `adjusted/<base>(<mode>)` into base branch and mode.
pub fn adjusted_branch(head: &str) -> Option<(&str, &str)> {
    let rest = head.strip_prefix("adjusted/")?.strip_suffix(')')?;
    let (base, mode) = rest.rsplit_once('(')?;
    (!base.is_empty() && !mode.is_empty()).then_some((base, mode))
}

/// Branches git-annex maintains for itself: the location-tracking branch and
/// the `synced/*` staging refs. Pass the branch name without any remote
/// prefix; a local `feature/git-annex` is an ordinary branch.
pub fn is_annex_ref(branch_name: &str) -> bool {
    branch_name == "git-annex" || branch_name.starts_with("synced/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "SHA256E-s10--5f0e8b51a6a5.bin";

    #[test]
    fn parses_keys_with_and_without_size() {
        let key = parse_key(KEY).unwrap();
        assert_eq!((&*key.backend, key.size), ("SHA256E", Some(10)));
        let key = parse_key("URL--https&c%%example.com%file").unwrap();
        assert_eq!((&*key.backend, key.size), ("URL", None));
        let key = parse_key("WORM-s3-m1700000000--name.txt").unwrap();
        assert_eq!(key.size, Some(3));
        for bad in [
            "",
            "SHA256E",
            "SHA256E-s10--",
            "SHA-256-s1--x",
            "SHA256E-sx--a",
            "a/b--c",
        ] {
            assert!(parse_key(bad).is_none(), "{bad}");
        }
    }

    /// Directories real git-annex 10.20260901 chose for these keys.
    #[test]
    fn object_paths_match_git_annex() {
        for (key, mixed, lower) in [
            (
                "SHA256E-s2000--44536ca869d7a09269b7205eeff9347d96cf7869234e830046e58e04559a9f85.bin",
                "2w/Fk",
                "91e/87e",
            ),
            (
                "SHA256E-s1500--0e34e6739f87fa817b7ce94598c5831dd6d8827c28863417e15479365fec4b95.bin",
                "p3/W1",
                "e31/cf2",
            ),
        ] {
            let [first, second] = object_paths(key, 2);
            assert_eq!(first, std::path::Path::new(mixed).join(key).join(key));
            assert_eq!(second, std::path::Path::new(lower).join(key).join(key));
        }
        let [one_level, _] = object_paths(
            "SHA256E-s2000--44536ca869d7a09269b7205eeff9347d96cf7869234e830046e58e04559a9f85.bin",
            1,
        );
        assert!(one_level.starts_with("2w"));
        assert_eq!(one_level.components().count(), 3);
    }

    #[test]
    fn reads_keys_from_locked_symlinks() {
        let target = format!("../../.git/annex/objects/Xk/Wq/{KEY}/{KEY}");
        assert_eq!(
            &*key_from_symlink_target(target.as_bytes()).unwrap().raw,
            KEY
        );
        let windows = format!("..\\.git\\annex\\objects\\Xk\\Wq\\{KEY}\\{KEY}");
        assert!(key_from_symlink_target(windows.as_bytes()).is_some());
        assert!(key_from_symlink_target(b"a.txt").is_none());
        let mismatched = format!(".git/annex/objects/Xk/Wq/other/{KEY}");
        assert!(key_from_symlink_target(mismatched.as_bytes()).is_none());
    }

    #[test]
    fn reads_keys_from_unlocked_pointers() {
        let pointer = format!("/annex/objects/{KEY}\n");
        assert_eq!(&*key_from_pointer(pointer.as_bytes()).unwrap().raw, KEY);
        assert!(key_from_pointer(format!("/annex/objects/{KEY}").as_bytes()).is_some());
        assert!(key_from_pointer(format!("/annex/objects/{KEY}\r\n").as_bytes()).is_some());
        let appended = format!("/annex/objects/{KEY}\nuser text\n");
        assert!(key_from_pointer(appended.as_bytes()).is_none());
        assert!(key_from_pointer(b"plain file\n").is_none());
    }

    #[test]
    fn splits_adjusted_branch_names() {
        assert_eq!(
            adjusted_branch("adjusted/main(unlocked)"),
            Some(("main", "unlocked"))
        );
        assert_eq!(
            adjusted_branch("adjusted/feat/x(hidemissing-unlocked)"),
            Some(("feat/x", "hidemissing-unlocked"))
        );
        assert_eq!(adjusted_branch("main"), None);
        assert_eq!(adjusted_branch("adjusted/(unlocked)"), None);
    }

    #[test]
    fn recognises_annex_bookkeeping_refs() {
        for name in ["git-annex", "synced/main", "synced/feat/x"] {
            assert!(is_annex_ref(name), "{name}");
        }
        for name in [
            "main",
            "feature/git-annex",
            "git-annex-docs",
            "unsynced/main",
        ] {
            assert!(!is_annex_ref(name), "{name}");
        }
    }
}
