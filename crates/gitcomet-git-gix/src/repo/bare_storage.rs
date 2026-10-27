use super::preflight::{LinkedWorktree, Preflight, read_head};
use super::{canonical_path, head_commit, native_mode, native_path, refusal};
#[cfg(test)]
use crate::repo::bare_lifecycle::OperationLock;
use crate::repo::bare_lifecycle::{
    ConversionLock, DirectoryIdentity, coordination_path, sibling_path,
};
use crate::util::{
    git_command_failed_error, git_workdir_cmd_for, run_git_capture_bytes_cancellable,
    run_git_raw_output,
};
use gitcomet_core::domain::RepositoryMode;
use gitcomet_core::error::{BareConversionRefusal, Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const JOURNAL_VERSION: u32 = 1;
const CONFIG_REPAIR: &[u8] = b"\n[core]\n\tbare = false\n";

pub(super) fn journal_path(root: &Path) -> Result<PathBuf> {
    sibling_path(root, ".bare-conversion.json")
}

pub(super) fn tree_digest(root: &Path) -> Result<String> {
    let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha256);
    digest_tree(root, Path::new(""), &mut hasher)?;
    Ok(hasher
        .try_finalize()
        .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))?
        .to_string())
}

pub(super) fn checkout_metadata_digest(root: &Path) -> Result<String> {
    metadata_digest(root, true)
}

fn metadata_digest(root: &Path, include_index: bool) -> Result<String> {
    let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha256);
    for relative in [
        "HEAD",
        "index",
        "config",
        "config.worktree",
        "commondir",
        "packed-refs",
        "refs",
        "worktrees",
    ] {
        if relative == "index" && !include_index {
            continue;
        }
        match root.join(relative).symlink_metadata() {
            Ok(_) => digest_tree(root, Path::new(relative), &mut hasher)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(relative.as_bytes());
                hasher.update(b"missing");
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(hasher
        .try_finalize()
        .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))?
        .to_string())
}

fn digest_tree(root: &Path, relative: &Path, hasher: &mut gix::hash::Hasher) -> Result<()> {
    let path = root.join(relative);
    let metadata = path.symlink_metadata()?;
    let encoded = relative.as_os_str().as_encoded_bytes();
    hasher.update(&(encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        hasher.update(&metadata.permissions().mode().to_le_bytes());
    }
    #[cfg(not(unix))]
    hasher.update(&[u8::from(metadata.permissions().readonly())]);
    if metadata.file_type().is_symlink() {
        hasher.update(b"symlink");
        hasher.update(fs::read_link(&path)?.as_os_str().as_encoded_bytes());
        return Ok(());
    }
    if metadata.is_file() {
        hasher.update(b"file");
        hasher.update(&metadata.len().to_le_bytes());
        let mut file = File::open(path)?;
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(refusal(BareConversionRefusal::PathCollision { path }));
    }
    hasher.update(b"directory");
    let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        digest_tree(root, &relative.join(entry.file_name()), hasher)?;
    }
    Ok(())
}

pub(super) fn verify_relocation_tree(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let metadata = path.symlink_metadata()?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(refusal(BareConversionRefusal::PathCollision { path }));
        }
        if metadata.is_dir() {
            verify_relocation_tree(&path)?;
        }
    }
    Ok(())
}

pub(super) fn repair_config(source: &Preflight) -> Result<()> {
    let written = replace_bare_config(&source.config_path, source.config_before.as_deref())?;
    let verified = verify_checkout(&source.preview.root, source);
    if let Err(error) = verified {
        restore_config(
            &source.config_path,
            &written,
            source.config_before.as_deref(),
        )
        .map_err(|recovery| {
            recovery_error(
                &source.preview.root,
                format!("{error}; config rollback failed: {recovery}"),
            )
        })?;
        return Err(error);
    }
    Ok(())
}

fn replace_bare_config(path: &Path, expected: Option<&[u8]>) -> Result<Vec<u8>> {
    let parent = path.parent().ok_or_else(|| {
        refusal(BareConversionRefusal::UnsupportedLayout {
            path: path.to_path_buf(),
        })
    })?;
    let mut scratch = tempfile::NamedTempFile::new_in(parent)?;
    scratch.write_all(expected.unwrap_or_default())?;
    let mut command = git_workdir_cmd_for(parent);
    command
        .arg("config")
        .arg("--file")
        .arg(scratch.path())
        .args(["--unset-all", "core.bare"]);
    let output = run_git_raw_output(command, "git config remove previous bare assignments")?;
    if !output.status.success() && output.status.code() != Some(5) {
        return Err(git_command_failed_error(
            "git config remove previous bare assignments",
            output,
        ));
    }
    let mut written = fs::read(scratch.path())?;
    written.extend_from_slice(CONFIG_REPAIR);
    replace_config(path, expected, Some(&written))?;
    Ok(written)
}

fn restore_config(path: &Path, expected: &[u8], original: Option<&[u8]>) -> Result<()> {
    replace_config(path, Some(expected), original)
}

fn replace_config(path: &Path, expected: Option<&[u8]>, replacement: Option<&[u8]>) -> Result<()> {
    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock_path = PathBuf::from(lock_name);
    let mut lock = File::create_new(&lock_path)?;
    let result = (|| {
        let actual = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(Error::from(error)),
        };
        if actual.as_deref() != expected {
            return Err(refusal(BareConversionRefusal::StalePreview));
        }
        if let Some(replacement) = replacement {
            lock.write_all(replacement)?;
            if let Ok(metadata) = fs::metadata(path) {
                lock.set_permissions(metadata.permissions())?;
            }
            lock.sync_all()?;
            drop(lock);
            fs::rename(&lock_path, path)?;
        } else {
            drop(lock);
            fs::remove_file(path)?;
            fs::remove_file(&lock_path)?;
        }
        sync_directory(path.parent().ok_or_else(|| {
            refusal(BareConversionRefusal::UnsupportedLayout {
                path: path.to_path_buf(),
            })
        })?)?;
        Ok(())
    })();
    if result.is_err() && lock_path.exists() {
        fs::remove_file(&lock_path).map_err(|cleanup| {
            recovery_error(path, format!("config lock cleanup failed: {cleanup}"))
        })?;
    }
    result
}

fn verify_checkout(root: &Path, source: &Preflight) -> Result<()> {
    if native_mode(root)? != RepositoryMode::WorkingTree
        || native_path(root, "--show-toplevel")? != root
        || read_head(root)? != source.preview.head
    {
        return Err(refusal(BareConversionRefusal::StalePreview));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPath {
    #[cfg(unix)]
    bytes: Vec<u8>,
    #[cfg(windows)]
    wide: Vec<u16>,
    #[cfg(not(any(unix, windows)))]
    text: String,
}

impl StoredPath {
    fn from_path(path: &Path) -> Self {
        #[cfg(unix)]
        {
            Self {
                bytes: path.as_os_str().as_encoded_bytes().to_vec(),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self {
                wide: path.as_os_str().encode_wide().collect(),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self {
                text: path.to_string_lossy().into_owned(),
            }
        }
    }

    fn path(&self) -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(OsString::from_vec(self.bytes.clone()))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(OsString::from_wide(&self.wide))
        }
        #[cfg(not(any(unix, windows)))]
        {
            PathBuf::from(&self.text)
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Phase {
    Prepared,
    OriginalMoved,
    Installed,
    Committed,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalLinkedWorktree {
    root: StoredPath,
    registration: StoredPath,
    original_pointer: Vec<u8>,
    installed_pointer: Vec<u8>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    root: StoredPath,
    staging: StoredPath,
    backup: StoredPath,
    original_identity: DirectoryIdentity,
    installed_identity: DirectoryIdentity,
    source_digest: String,
    installed_digest: String,
    phase: Phase,
    linked: Vec<JournalLinkedWorktree>,
}

pub(super) fn relocate(source: &Preflight, cancellation: &CancellationToken) -> Result<()> {
    let root = &source.preview.root;
    let parent = root
        .parent()
        .ok_or_else(|| refusal(BareConversionRefusal::UnsupportedLayout { path: root.clone() }))?;
    let staged = tempfile::Builder::new()
        .prefix(".bare-checkout-")
        .tempdir_in(parent)?;
    fs::create_dir(staged.path().join(".git"))?;
    copy_metadata(
        &source.common_dir,
        &staged.path().join(".git"),
        Path::new(""),
        cancellation,
    )?;
    let relative_config = source
        .config_path
        .strip_prefix(&source.common_dir)
        .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))?;
    replace_bare_config(
        &staged.path().join(".git").join(relative_config),
        source.config_before.as_deref(),
    )?;
    let staged_metadata = metadata_digest(&staged.path().join(".git"), false)?;
    checkout_staged(staged.path(), source, cancellation)?;
    if metadata_digest(&staged.path().join(".git"), false)? != staged_metadata
        || read_head(staged.path())? != source.preview.head
    {
        return Err(refusal(BareConversionRefusal::StalePreview));
    }
    fs::set_permissions(staged.path(), fs::metadata(root)?.permissions())?;
    source.revalidate()?;
    cancellation.check_cancelled()?;
    sync_tree(staged.path())?;
    let backup = tempfile::Builder::new()
        .prefix(".bare-backup-")
        .tempdir_in(parent)?;
    let backup_path = backup.path().to_path_buf();
    backup.close()?;
    let mut journal = Journal {
        version: JOURNAL_VERSION,
        root: StoredPath::from_path(root),
        staging: StoredPath::from_path(staged.path()),
        backup: StoredPath::from_path(&backup_path),
        original_identity: DirectoryIdentity::read(root)?,
        installed_identity: DirectoryIdentity::read(staged.path())?,
        source_digest: source.source_digest.clone(),
        installed_digest: tree_digest(staged.path())?,
        phase: Phase::Prepared,
        linked: source
            .linked
            .iter()
            .map(|linked| journal_linked(root, linked))
            .collect::<Result<Vec<_>>>()?,
    };
    write_journal(&journal)?;
    let staged = staged.keep();
    let result = install(source, &mut journal, cancellation);
    if let Err(error) = result {
        return match recover_journal(&journal) {
            Ok(()) => Err(error),
            Err(recovery) => Err(recovery_error(root, format!("{error}; {recovery}"))),
        };
    }
    if staged.exists() {
        return Err(recovery_error(
            root,
            "staging directory unexpectedly remains after install",
        ));
    }
    Ok(())
}

fn copy_metadata(
    source: &Path,
    destination: &Path,
    relative: &Path,
    cancellation: &CancellationToken,
) -> Result<()> {
    cancellation.check_cancelled()?;
    let path = source.join(relative);
    let target = destination.join(relative);
    let metadata = path.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(refusal(BareConversionRefusal::PathCollision { path }));
    }
    if metadata.is_dir() {
        if !relative.as_os_str().is_empty() {
            fs::create_dir(&target)?;
        }
        for entry in fs::read_dir(path)? {
            copy_metadata(
                source,
                destination,
                &relative.join(entry?.file_name()),
                cancellation,
            )?;
        }
        fs::set_permissions(target, metadata.permissions())?;
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(refusal(BareConversionRefusal::PathCollision { path }));
    }
    if cfg!(unix) && immutable_object(relative) && fs::hard_link(&path, &target).is_ok() {
        return Ok(());
    }
    fs::copy(path, target)?;
    Ok(())
}

fn immutable_object(relative: &Path) -> bool {
    let parts = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "objects" {
        return false;
    }
    if parts[1] == "pack" {
        return [".pack", ".idx", ".rev"]
            .iter()
            .any(|suffix| parts[2].ends_with(suffix));
    }
    parts[1].len() == 2
        && parts[1].bytes().all(|byte| byte.is_ascii_hexdigit())
        && parts[2].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn checkout_staged(
    root: &Path,
    source: &Preflight,
    cancellation: &CancellationToken,
) -> Result<()> {
    let mut cmd = git_workdir_cmd_for(root);
    cmd.arg("--git-dir")
        .arg(root.join(".git"))
        .arg("--work-tree")
        .arg(root);
    cmd.env("GIT_INDEX_FILE", root.join(".git/index"));
    cmd.args(["-c", "core.bare=false", "read-tree"]);
    match head_commit(&source.preview.head) {
        Some(commit) => {
            cmd.args(["-m", "-u", commit.as_ref()]);
        }
        None => {
            cmd.arg("--empty");
        }
    }
    run_git_capture_bytes_cancellable(cmd, "git read-tree staged checkout", cancellation)?;
    let mut verify = git_workdir_cmd_for(root);
    verify
        .arg("--git-dir")
        .arg(root.join(".git"))
        .arg("--work-tree")
        .arg(root);
    verify.args(["-c", "core.bare=false", "diff-files", "--quiet"]);
    run_git_capture_bytes_cancellable(verify, "git diff-files staged checkout", cancellation)?;
    if let Some(commit) = head_commit(&source.preview.head) {
        let mut verify = git_workdir_cmd_for(root);
        verify
            .arg("--git-dir")
            .arg(root.join(".git"))
            .arg("--work-tree")
            .arg(root);
        verify.args([
            "-c",
            "core.bare=false",
            "diff-index",
            "--cached",
            "--quiet",
            commit.as_ref(),
        ]);
        run_git_capture_bytes_cancellable(verify, "git diff-index staged checkout", cancellation)?;
    }
    Ok(())
}

fn journal_linked(root: &Path, linked: &LinkedWorktree) -> Result<JournalLinkedWorktree> {
    let target = root.join(".git").join(&linked.registration);
    let target = gix::path::into_bstr(target)
        .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())).with_source(error))?;
    let target = gix::path::to_unix_separators_on_windows(target);
    let mut installed_pointer = b"gitdir: ".to_vec();
    installed_pointer.extend_from_slice(&target);
    installed_pointer.push(b'\n');
    Ok(JournalLinkedWorktree {
        root: StoredPath::from_path(&linked.root),
        registration: StoredPath::from_path(&linked.registration),
        original_pointer: linked.pointer.clone(),
        installed_pointer,
    })
}

fn install(
    source: &Preflight,
    journal: &mut Journal,
    cancellation: &CancellationToken,
) -> Result<()> {
    let root = journal.root.path();
    let parent = root
        .parent()
        .ok_or_else(|| recovery_error(&root, "repository has no parent"))?;
    source.revalidate()?;
    cancellation.check_cancelled()?;
    rename_directory(&root, &journal.backup.path())?;
    sync_directory(parent)?;
    journal.phase = Phase::OriginalMoved;
    write_journal(journal)?;
    failpoint(Failpoint::OriginalMoved)?;
    rename_directory(&journal.staging.path(), &root)?;
    sync_directory(parent)?;
    journal.phase = Phase::Installed;
    write_journal(journal)?;
    failpoint(Failpoint::Installed)?;
    if !journal.linked.is_empty() {
        let mut cmd = git_workdir_cmd_for(&root);
        cmd.args(["worktree", "repair", "--"]);
        cmd.args(journal.linked.iter().map(|linked| linked.root.path()));
        run_git_capture_bytes_cancellable(
            cmd,
            "git worktree repair relocated repository",
            cancellation,
        )?;
    }
    verify_checkout(&root, source)?;
    for linked in &journal.linked {
        if fs::read(linked.root.path().join(".git"))? != linked.installed_pointer {
            return Err(recovery_error(
                &root,
                "linked worktree repair produced an unexpected pointer",
            ));
        }
    }
    if tree_digest(&root)? != journal.installed_digest {
        return Err(recovery_error(
            &root,
            "the installed checkout changed during verification",
        ));
    }
    failpoint(Failpoint::Repaired)?;
    journal.phase = Phase::Committed;
    write_journal(journal)?;
    failpoint(Failpoint::Committed)?;
    cleanup_committed(journal)
}

fn write_journal(journal: &Journal) -> Result<()> {
    let root = journal.root.path();
    let bytes =
        serde_json::to_vec(journal).map_err(|error| recovery_error(&root, error.to_string()))?;
    write_journal_copy(&trusted_journal_path(&root)?, &bytes)?;
    write_journal_copy(&journal_path(&root)?, &bytes)
}

fn trusted_journal_path(root: &Path) -> Result<PathBuf> {
    let coordination = coordination_path(root)?;
    let parent = coordination
        .parent()
        .ok_or_else(|| recovery_error(root, "coordination record has no parent"))?;
    let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha256);
    hasher.update(root.as_os_str().as_encoded_bytes());
    let key = hasher
        .try_finalize()
        .map_err(|error| recovery_error(root, error.to_string()))?;
    Ok(parent.join(format!("{key}.conversion.json")))
}

fn write_journal_copy(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| recovery_error(path, "journal has no parent"))?;
    let mut pending = tempfile::NamedTempFile::new_in(parent)?;
    pending.write_all(bytes)?;
    pending.as_file().sync_all()?;
    pending
        .persist(path)
        .map_err(|error| Error::from(error.error))?;
    sync_directory(parent)
}

fn read_trusted_journal(root: &Path) -> Result<Journal> {
    let trusted_path = trusted_journal_path(root)?;
    let bytes = fs::read(&trusted_path).map_err(|error| {
        recovery_error(
            root,
            format!("the trusted local conversion record is unavailable: {error}"),
        )
    })?;
    let trusted: Journal = serde_json::from_slice(&bytes).map_err(|error| {
        recovery_error(root, format!("invalid trusted conversion record: {error}"))
    })?;
    validate_journal_paths(root, &trusted)?;
    match fs::read(journal_path(root)?) {
        Ok(bytes) => {
            let mut sibling: Journal = serde_json::from_slice(&bytes).map_err(|error| {
                recovery_error(root, format!("invalid sibling conversion record: {error}"))
            })?;
            sibling.phase = trusted.phase;
            if sibling != trusted {
                return Err(recovery_error(
                    root,
                    "the sibling journal does not match the trusted local conversion record",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(trusted)
}

fn remove_journals(root: &Path) -> Result<()> {
    for path in [journal_path(root)?, trusted_journal_path(root)?] {
        match fs::remove_file(&path) {
            Ok(()) => sync_directory(
                path.parent()
                    .ok_or_else(|| recovery_error(root, "journal has no parent"))?,
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn recovery_error(path: &Path, reason: impl Into<String>) -> Error {
    Error::new(ErrorKind::BareConversionRecovery {
        path: path.to_path_buf(),
        reason: reason.into(),
    })
}

pub(super) fn recover_before_open(selected: &Path, cancellation: &CancellationToken) -> Result<()> {
    cancellation.check_cancelled()?;
    let absolute = if selected.is_absolute() {
        selected.to_path_buf()
    } else {
        std::env::current_dir()?.join(selected)
    };
    let mut roots = vec![absolute.clone()];
    let gitfile = if absolute.is_file() {
        absolute.clone()
    } else {
        absolute.join(".git")
    };
    if gitfile.is_file() {
        let bytes = fs::read(&gitfile)?;
        if let Some(target) = bytes.strip_prefix(b"gitdir: ") {
            let target = target.strip_suffix(b"\n").unwrap_or(target);
            let target = crate::util::path_buf_from_git_bytes(target, "linked worktree gitfile")?;
            let target = if target.is_absolute() {
                target
            } else {
                gitfile
                    .parent()
                    .ok_or_else(|| recovery_error(&absolute, "gitfile has no parent"))?
                    .join(target)
            };
            roots.push(target);
        }
    }
    let mut visited = std::collections::BTreeSet::new();
    for candidate in roots.iter().flat_map(|root| root.ancestors()) {
        cancellation.check_cancelled()?;
        let Some(parent) = candidate.parent() else {
            continue;
        };
        let Ok(parent) = canonical_path(parent) else {
            continue;
        };
        let Some(name) = candidate.file_name() else {
            continue;
        };
        let root = parent.join(name);
        if !visited.insert(root.clone()) {
            continue;
        }
        let journal_path = journal_path(&root)?;
        if !journal_path.exists() && !trusted_journal_path(&root)?.exists() {
            continue;
        }
        let _lock = ConversionLock::recover(&root, cancellation)?;
        if !journal_path.exists() && !trusted_journal_path(&root)?.exists() {
            continue;
        }
        let journal = read_trusted_journal(&root)?;
        recover_journal(&journal)?;
    }
    Ok(())
}

fn validate_journal_paths(root: &Path, journal: &Journal) -> Result<()> {
    if journal.version != JOURNAL_VERSION
        || journal.root.path() != root
        || journal.staging.path().parent() != root.parent()
        || journal.backup.path().parent() != root.parent()
        || journal.staging.path() == root
        || journal.backup.path() == root
        || journal.staging.path() == journal.backup.path()
        || !generated_directory_name(&journal.staging.path(), ".bare-checkout-")
        || !generated_directory_name(&journal.backup.path(), ".bare-backup-")
    {
        return Err(recovery_error(
            root,
            "journal paths or version do not match the selected repository",
        ));
    }
    for linked in &journal.linked {
        let registration = linked.registration.path();
        if !linked.root.path().is_absolute()
            || linked.root.path().starts_with(root)
            || registration.components().count() != 2
            || registration.parent() != Some(Path::new("worktrees"))
        {
            return Err(recovery_error(
                root,
                "journal linked-worktree paths are invalid",
            ));
        }
    }
    Ok(())
}

fn generated_directory_name(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(prefix))
        .is_some_and(|suffix| {
            suffix.len() >= 6 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn check_copy(path: &Path, identity: &DirectoryIdentity, digest: &str) -> Result<()> {
    if DirectoryIdentity::read(path)? != *identity || tree_digest(path)? != digest {
        return Err(recovery_error(
            path,
            "a repository copy changed after the journal was written",
        ));
    }
    Ok(())
}

fn recover_journal(journal: &Journal) -> Result<()> {
    let root = journal.root.path();
    validate_journal_paths(&root, journal)?;
    if journal.phase == Phase::Committed {
        return cleanup_committed(journal);
    }
    let backup = journal.backup.path();
    let staged = journal.staging.path();
    if backup.exists() {
        check_copy(&backup, &journal.original_identity, &journal.source_digest)?;
        if root.exists() {
            check_copy(
                &root,
                &journal.installed_identity,
                &journal.installed_digest,
            )?;
        }
        if staged.exists() {
            check_copy(
                &staged,
                &journal.installed_identity,
                &journal.installed_digest,
            )?;
        }
        validate_linked_pointers(journal)?;
        if root.exists() {
            if staged.exists() {
                return Err(recovery_error(
                    &root,
                    "both installed and staged checkouts exist",
                ));
            }
            check_copy(
                &root,
                &journal.installed_identity,
                &journal.installed_digest,
            )?;
            rename_directory(&root, &staged)?;
        }
        check_copy(&backup, &journal.original_identity, &journal.source_digest)?;
        rename_directory(&backup, &root)?;
        check_copy(&root, &journal.original_identity, &journal.source_digest)?;
    } else {
        check_copy(&root, &journal.original_identity, &journal.source_digest)?;
        validate_linked_pointers(journal)?;
    }
    restore_linked_pointers(journal)?;
    if staged.exists() {
        check_copy(
            &staged,
            &journal.installed_identity,
            &journal.installed_digest,
        )?;
        fs::remove_dir_all(staged)?;
    }
    remove_journals(&root)?;
    sync_directory(
        root.parent()
            .ok_or_else(|| recovery_error(&root, "repository has no parent"))?,
    )
}

fn validate_linked_pointers(journal: &Journal) -> Result<()> {
    for linked in &journal.linked {
        let path = linked.root.path().join(".git");
        let bytes = fs::read(&path)?;
        if bytes != linked.original_pointer && bytes != linked.installed_pointer {
            return Err(recovery_error(
                &path,
                "linked worktree pointer changed after conversion started",
            ));
        }
    }
    Ok(())
}

fn restore_linked_pointers(journal: &Journal) -> Result<()> {
    for linked in &journal.linked {
        let path = linked.root.path().join(".git");
        let bytes = fs::read(&path)?;
        if bytes == linked.original_pointer {
            continue;
        }
        if bytes != linked.installed_pointer {
            return Err(recovery_error(
                &path,
                "linked worktree pointer changed during recovery",
            ));
        }
        let mut staged = tempfile::NamedTempFile::new_in(linked.root.path())?;
        staged.write_all(&linked.original_pointer)?;
        staged.as_file().sync_all()?;
        staged
            .persist(&path)
            .map_err(|error| Error::from(error.error))?;
        sync_directory(&linked.root.path())?;
    }
    Ok(())
}

fn cleanup_committed(journal: &Journal) -> Result<()> {
    let root = journal.root.path();
    if DirectoryIdentity::read(&root)? != journal.installed_identity {
        return Err(recovery_error(
            &root,
            "the converted repository was replaced before cleanup",
        ));
    }
    let backup = journal.backup.path();
    if backup.exists() {
        check_copy(&backup, &journal.original_identity, &journal.source_digest)?;
        fs::remove_dir_all(backup)?;
    }
    remove_journals(&root)?;
    sync_directory(
        root.parent()
            .ok_or_else(|| recovery_error(&root, "repository has no parent"))?,
    )
}

fn sync_tree(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            sync_tree(&entry.path())?;
        } else if file_type.is_file() {
            sync_staged_file(&entry.path())?;
        } else {
            return Err(refusal(BareConversionRefusal::PathCollision {
                path: entry.path(),
            }));
        }
    }
    sync_directory(root)
}

fn sync_staged_file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let permissions = fs::metadata(path)?.permissions();
        if permissions.readonly() {
            let mut writable = permissions.clone();
            // Windows only: this clears the read-only attribute, not Unix modes.
            #[allow(clippy::permissions_set_readonly_false)]
            writable.set_readonly(false);
            fs::set_permissions(path, writable)?;
        }
        let file = match File::options().write(true).open(path) {
            Ok(file) => file,
            Err(error) => {
                fs::set_permissions(path, permissions).map_err(|restore| {
                    recovery_error(
                        path,
                        format!("{error}; restoring staged file attributes failed: {restore}"),
                    )
                })?;
                return Err(error.into());
            }
        };
        file.set_permissions(permissions)?;
        file.sync_all()?;
    }
    #[cfg(not(windows))]
    File::open(path)?.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failpoint {
    OriginalMoved,
    Installed,
    Repaired,
    Committed,
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct TestFault {
    point: Failpoint,
    crash: bool,
}

#[cfg(test)]
thread_local! { static FAILPOINT: std::cell::Cell<Option<TestFault>> = const { std::cell::Cell::new(None) }; }

fn failpoint(point: Failpoint) -> Result<()> {
    #[cfg(test)]
    if let Some(fault) = FAILPOINT
        .with(|active| active.get())
        .filter(|fault| fault.point == point)
    {
        assert!(!fault.crash, "conversion crash failpoint {point:?}");
        return Err(Error::new(ErrorKind::Backend(format!(
            "conversion failpoint {point:?}"
        ))));
    }
    #[cfg(not(test))]
    let _ = point;
    Ok(())
}

fn rename_directory(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| Error::from(std::io::Error::from(error)))?;
        Ok(())
    }
    #[cfg(windows)]
    {
        fs::rename(source, destination).map_err(Error::from)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        windows
    )))]
    {
        let _ = destination;
        Err(refusal(BareConversionRefusal::UnsupportedLayout {
            path: source.to_path_buf(),
        }))
    }
}

#[cfg(test)]
#[path = "bare_recovery_tests.rs"]
mod tests;
