//! git-annex keys as they appear in git: a locked file is a symlink into
//! `.git/annex/objects/…/<KEY>/<KEY>`, an unlocked file is a pointer file
//! whose first line is `/annex/objects/<KEY>`.

use std::{borrow::Cow, sync::Arc};

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
    if raw.chars().any(char::is_control) || (cfg!(windows) && raw.contains('\\')) {
        return None;
    }
    let (fields, name) = raw.split_once("--")?;
    if name.is_empty() {
        return None;
    }
    let mut fields = fields.split('-');
    let backend = fields.next()?;
    if backend.is_empty()
        || !backend
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
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
    let target = std::str::from_utf8(target).ok()?;
    // Preserve literal backslashes in Unix key filenames. Only normalize a
    // Windows-style target when its directory separators require it.
    let target = if target.contains("annex/objects/") {
        Cow::Borrowed(target)
    } else {
        Cow::Owned(target.replace('\\', "/"))
    };
    if !target.contains("annex/objects/") {
        return None;
    }
    let mut parts = target.rsplit('/');
    let key = parts.next()?;
    // The object sits in a directory named after its own key.
    if parts.next()? != key {
        return None;
    }
    key_from_filename(key)
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
    key_from_filename(key)
}

/// git-annex's keyFile escaping is separate from the raw key accepted by
/// --key and hashed for object directories. Decode in one pass so escaped
/// ampersands cannot introduce another escape.
fn key_from_filename(filename: &str) -> Option<AnnexKey> {
    if filename.contains('/') {
        return None;
    }
    if !filename.contains(['&', '%']) {
        return parse_key(filename);
    }
    let mut key = String::with_capacity(filename.len());
    let mut chars = filename.chars().peekable();
    while let Some(ch) = chars.next() {
        let decoded = match (ch, chars.peek()) {
            ('%', _) => '/',
            ('&', Some('a')) => {
                chars.next();
                '&'
            }
            ('&', Some('c')) => {
                chars.next();
                ':'
            }
            ('&', Some('s')) => {
                chars.next();
                '%'
            }
            _ => ch,
        };
        key.push(decoded);
    }
    parse_key(&key)
}

fn key_filename(key: &str) -> Cow<'_, str> {
    if !key.contains(['&', '%', ':', '/']) {
        return Cow::Borrowed(key);
    }
    let mut filename = String::with_capacity(key.len());
    for ch in key.chars() {
        match ch {
            '&' => filename.push_str("&a"),
            '%' => filename.push_str("&s"),
            ':' => filename.push_str("&c"),
            '/' => filename.push('%'),
            _ => filename.push(ch),
        }
    }
    Cow::Owned(filename)
}

/// Where git-annex may keep a key's content, relative to `.git/annex/objects`:
/// `hashdirmixed` (non-bare repos) first, then `hashdirlower` (bare and
/// crippled-filesystem repos), as git-annex itself checks both. `levels` is 2,
/// or 1 under `annex.tune.objecthash1`.
pub fn object_paths(key: &str, levels: usize) -> [std::path::PathBuf; 2] {
    use md5::{Digest as _, Md5};
    let digest = Md5::digest(key.as_bytes());
    // hashDirMixed uses git-annex's own alphabet and skips every sixth bit;
    // standard base32 cannot reproduce it. Swap digit pairs, two per level.
    const CHARS: &[u8; 32] = b"0123456789zqjxkmvwgpfZQJXKMVWGPF";
    let word = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    let mixed: [u8; 4] = std::array::from_fn(|i| CHARS[((word >> (6 * (i ^ 1))) & 31) as usize]);
    // hashDirLower: hex digest, three characters per level.
    let mut hex = [0u8; 6];
    let hex = faster_hex::hex_encode(&digest[..3], &mut hex).expect("six hex digits fit");
    let levels = levels.clamp(1, 2);
    let mixed = std::str::from_utf8(&mixed).unwrap_or_default();
    let filename = key_filename(key);
    let path = |width: usize, digits: &str| {
        (0..levels)
            .map(|level| &digits[level * width..(level + 1) * width])
            .chain([filename.as_ref(), filename.as_ref()])
            .collect()
    };
    [path(2, mixed), path(3, hex)]
}

/// Split `adjusted/<base>(<mode>)` into base branch and mode.
pub fn adjusted_branch(head: &str) -> Option<(&str, &str)> {
    let rest = head.strip_prefix("adjusted/")?.strip_suffix(')')?;
    let (base, mode) = rest.rsplit_once('(')?;
    (!base.is_empty() && !mode.is_empty()).then_some((base, mode))
}

/// Bulky directories below a Git directory that the file watcher never walks.
pub const WATCH_PRIVATE_DIRS: [&str; 5] = [
    "annex/objects",
    "annex/keysdb",
    "annex/transfer",
    "annex/tmp",
    "annex/othertmp",
];

/// Logs the support summary reads from the git-annex branch and journals.
pub const SUPPORT_LOGS: [&str; 4] = ["uuid.log", "trust.log", "remote.log", "numcopies.log"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchPath {
    /// Locks, databases, temp files and location logs git-annex rewrites on
    /// every command. Never a reason to refresh.
    Private,
    /// A directory holding support metadata; only its creation or removal matters.
    Directory,
    /// A file the support summary reads.
    Support,
}

/// How the file watcher treats `relative` (to a Git directory), or `None`
/// outside `annex/`. Anything not named here is private, so new git-annex
/// bookkeeping files can never start a refresh loop.
pub fn watch_path(relative: &std::path::Path) -> Option<WatchPath> {
    // Non-UTF-8 names match nothing below, so they stay private.
    let mut parts = relative
        .components()
        .map(|part| part.as_os_str().to_str().unwrap_or_default());
    if parts.next()? != "annex" {
        return None;
    }
    let journal = |dir: &str| dir == "journal" || dir == "journal-private";
    Some(match (parts.next(), parts.next(), parts.next()) {
        (None, ..) => WatchPath::Directory,
        (Some(dir), None, _) if journal(dir) => WatchPath::Directory,
        (Some("restage.log" | "daemon.pid"), None, _) => WatchPath::Support,
        (Some(dir), Some(log), None) if journal(dir) && SUPPORT_LOGS.contains(&log) => {
            WatchPath::Support
        }
        _ => WatchPath::Private,
    })
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
    fn filesystem_escapes_are_decoded_once_and_encoded_for_object_paths() {
        // Raw keys, filenames and hash directories verified with examinekey
        // and fromkey, including escapes that could be decoded twice.
        for (raw, filename, mixed) in [
            (
                "WORM-s5-m1700000000--a:b.txt",
                "WORM-s5-m1700000000--a&cb.txt",
                "f4/6x",
            ),
            ("URL--a&b:c%d/e", "URL--a&ab&cc&sd%e", "x5/2p"),
        ] {
            let pointer = format!("/annex/objects/{filename}\n");
            let target = format!(".git/annex/objects/{mixed}/{filename}/{filename}");
            assert_eq!(
                key_from_pointer(pointer.as_bytes()).unwrap().raw.as_ref(),
                raw
            );
            assert_eq!(
                key_from_symlink_target(target.as_bytes())
                    .unwrap()
                    .raw
                    .as_ref(),
                raw
            );
            assert_eq!(
                object_paths(raw, 2)[0],
                std::path::Path::new(mixed).join(filename).join(filename)
            );
            assert_eq!(key_filename(raw), filename);
        }
        let raw = "URL--literal&c&s%:雪";
        assert_eq!(
            key_from_filename(&key_filename(raw)).unwrap().raw.as_ref(),
            raw
        );
        assert!(key_from_pointer(b"/annex/objects/URL--../outside\n").is_none());
    }

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

    #[test]
    fn sha3_backends_are_recognised_in_keys_and_both_pointer_forms() {
        for backend in ["SHA3_224", "SHA3_256", "SHA3_384", "SHA3_512", "SHA3_512E"] {
            let raw = format!("{backend}-s5--abcdef.bin");
            let expected = AnnexKey {
                raw: Arc::from(raw.as_str()),
                backend: Arc::from(backend),
                size: Some(5),
            };
            assert_eq!(parse_key(&raw), Some(expected.clone()));
            let pointer = format!("/annex/objects/{raw}\n");
            assert_eq!(key_from_pointer(pointer.as_bytes()), Some(expected.clone()));
            let target = format!("../.git/annex/objects/Xk/Wq/{raw}/{raw}");
            assert_eq!(key_from_symlink_target(target.as_bytes()), Some(expected));
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
