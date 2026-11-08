use super::{canonical_path, head_commit, native_mode, native_path, refusal, storage};
use crate::repo::worktrees::parse_git_worktree_list_porcelain_z;
use crate::util::{git_workdir_cmd_for, run_git_metadata_capture, run_git_metadata_output};
use gitcomet_core::domain::{
    BareConversionHead, BareConversionLayout, BareConversionPreview, CommitId, RepositoryMode,
    Worktree,
};
use gitcomet_core::error::{BareConversionRefusal, Error, ErrorKind};
use gitcomet_core::services::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) struct LinkedWorktree {
    pub root: PathBuf,
    pub registration: PathBuf,
    pub pointer: Vec<u8>,
    pub effective_config: Vec<u8>,
}

pub(in crate::repo) struct Preflight {
    pub(super) preview: BareConversionPreview,
    pub(super) common_dir: PathBuf,
    pub(super) config_path: PathBuf,
    pub(super) config_before: Option<Vec<u8>>,
    pub(super) linked: Vec<LinkedWorktree>,
    pub(super) source_digest: String,
    pub(super) effective_config: Vec<u8>,
}

impl Preflight {
    pub(super) fn revalidate(&self) -> Result<()> {
        if native_mode(&self.preview.root)? != RepositoryMode::Bare
            || read_head(&self.preview.root)? != self.preview.head
            || config_bytes(&self.preview.root)? != self.effective_config
            || source_digest(&self.common_dir, self.preview.layout)? != self.source_digest
        {
            return Err(refusal(BareConversionRefusal::StalePreview));
        }
        check_head_available(&self.preview.root, &self.preview.head)?;
        for linked in &self.linked {
            if fs::read(linked.root.join(".git"))? != linked.pointer
                || config_bytes(&linked.root)? != linked.effective_config
            {
                return Err(refusal(BareConversionRefusal::StalePreview));
            }
        }
        Ok(())
    }
}

pub(super) fn inspect(root: &Path) -> Result<Preflight> {
    let root = canonical_path(root)?;
    reject_environment_overrides(&root)?;
    if native_mode(&root)? != RepositoryMode::Bare {
        return Err(refusal(BareConversionRefusal::NotBare));
    }
    if storage::journal_path(&root)?.exists() {
        return Err(Error::new(ErrorKind::BareConversionRecovery {
            path: root,
            reason: "an earlier conversion is pending; reopen the repository to recover it".into(),
        }));
    }
    let git_dir = native_path(&root, "--absolute-git-dir")?;
    let common_dir = native_path(&root, "--git-common-dir")?;
    let layout = if git_dir == root && common_dir == root {
        BareConversionLayout::BareDirectory
    } else if git_dir == root.join(".git")
        && git_dir == common_dir
        && plain_directory(&root.join(".git"))?
    {
        BareConversionLayout::ExistingCheckout
    } else {
        return Err(refusal(BareConversionRefusal::UnsupportedLayout {
            path: root,
        }));
    };
    let head = read_head(&root)?;
    let worktrees = read_worktrees(&root)?;
    check_head_in_worktrees(&head, &worktrees)?;
    let effective_config = config_bytes(&root)?;
    let config = parse_config(&effective_config)?;
    check_worktree_config(&root, &config)?;
    if crate::repo::annex::assistant_running(&common_dir.join("annex")) {
        return Err(refusal(BareConversionRefusal::ActiveOperation {
            path: common_dir.join("annex/daemon.pid"),
        }));
    }
    check_operations(&common_dir)?;
    let mut linked = Vec::new();
    if layout == BareConversionLayout::ExistingCheckout {
        check_existing_checkout(&root, &git_dir, &head)?;
    } else {
        check_relocation_config(&root, &config)?;
        check_root_entries(&root)?;
        storage::verify_relocation_tree(&root)?;
        check_tree_paths(&root, &head)?;
        linked = read_linked_worktrees(&root, &worktrees)?;
    }
    let mut affected_roots = vec![root.clone()];
    affected_roots.extend(
        worktrees
            .iter()
            .filter(|worktree| !worktree.bare && worktree.path != root)
            .map(|worktree| worktree.path.clone()),
    );
    affected_roots.sort();
    affected_roots.dedup();
    let config_path = if worktree_config_enabled(&root)? && worktree_bare_configured(&root)? {
        git_dir.join("config.worktree")
    } else {
        common_dir.join("config")
    };
    let config_before = match fs::read(&config_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if fs::symlink_metadata(&config_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(refusal(BareConversionRefusal::UnsupportedLayout {
            path: config_path,
        }));
    }
    Ok(Preflight {
        preview: BareConversionPreview::new(root, layout, head, affected_roots),
        source_digest: source_digest(&common_dir, layout)?,
        common_dir,
        config_path,
        config_before,
        linked,
        effective_config,
    })
}

fn source_digest(root: &Path, layout: BareConversionLayout) -> Result<String> {
    match layout {
        BareConversionLayout::ExistingCheckout => storage::checkout_metadata_digest(root),
        BareConversionLayout::BareDirectory => storage::tree_digest(root),
    }
}

fn reject_environment_overrides(root: &Path) -> Result<()> {
    for key in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        if std::env::var_os(key).is_some() {
            return Err(refusal(BareConversionRefusal::UnsupportedLayout {
                path: root.to_path_buf(),
            })
            .with_context(format!("{key} overrides repository discovery"), Some(root)));
        }
    }
    Ok(())
}

pub(super) fn read_head(root: &Path) -> Result<BareConversionHead> {
    let mut symbolic = git_workdir_cmd_for(root);
    symbolic.args(["symbolic-ref", "--quiet", "HEAD"]);
    let symbolic = run_git_metadata_output(symbolic, "git symbolic-ref HEAD")?;
    let branch = if symbolic.status.success() {
        let name = std::str::from_utf8(&symbolic.stdout)
            .map_err(|_| refusal(BareConversionRefusal::InvalidHead))?
            .trim_end_matches(['\r', '\n']);
        let name = name
            .strip_prefix("refs/heads/")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| refusal(BareConversionRefusal::InvalidHead))?;
        Some(name.to_owned())
    } else if symbolic.status.code() == Some(1) {
        None
    } else {
        return Err(refusal(BareConversionRefusal::InvalidHead));
    };
    let mut commit = git_workdir_cmd_for(root);
    commit.args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
    let commit = run_git_metadata_output(commit, "git rev-parse HEAD")?;
    if commit.status.success() {
        let id = std::str::from_utf8(&commit.stdout)
            .map_err(|_| refusal(BareConversionRefusal::InvalidHead))?
            .trim_end_matches(['\r', '\n']);
        let id = gix::ObjectId::from_hex(id.as_bytes())
            .map_err(|_| refusal(BareConversionRefusal::InvalidHead))?;
        let commit = CommitId(id.to_string().into());
        return Ok(match branch {
            Some(name) => BareConversionHead::Branch {
                name,
                commit: Some(commit),
            },
            None => BareConversionHead::Detached { commit },
        });
    }
    let Some(name) = branch else {
        return Err(refusal(BareConversionRefusal::InvalidHead));
    };
    let reference = format!("refs/heads/{name}");
    let mut valid = git_workdir_cmd_for(root);
    valid.args(["check-ref-format", &reference]);
    if !run_git_metadata_output(valid, "git check-ref-format")?
        .status
        .success()
    {
        return Err(refusal(BareConversionRefusal::InvalidHead));
    }
    let mut exists = git_workdir_cmd_for(root);
    exists.args(["show-ref", "--verify", "--quiet", &reference]);
    if run_git_metadata_output(exists, "git show-ref HEAD branch")?
        .status
        .code()
        != Some(1)
    {
        return Err(refusal(BareConversionRefusal::InvalidHead));
    }
    Ok(BareConversionHead::Branch { name, commit: None })
}

fn read_worktrees(root: &Path) -> Result<Vec<Worktree>> {
    let mut cmd = git_workdir_cmd_for(root);
    cmd.args(["worktree", "list", "--porcelain", "-z"]);
    let worktrees =
        parse_git_worktree_list_porcelain_z(&run_git_metadata_capture(cmd, "git worktree list")?)?;
    let registrations = native_path(root, "--git-common-dir")?.join("worktrees");
    let entries = match fs::read_dir(&registrations) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(worktrees),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let registration = entry?.path();
        let inaccessible = || {
            refusal(BareConversionRefusal::InaccessibleWorktree {
                path: registration.clone(),
            })
        };
        let recorded = fs::read(registration.join("gitdir")).map_err(|_| inaccessible())?;
        let recorded = recorded.strip_suffix(b"\n").unwrap_or(&recorded);
        let recorded = if cfg!(windows) {
            recorded.strip_suffix(b"\r").unwrap_or(recorded)
        } else {
            recorded
        };
        let gitdir = crate::util::path_buf_from_git_bytes(recorded, "linked worktree gitdir")?;
        if !worktrees
            .iter()
            .any(|worktree| !worktree.bare && gitdir == worktree.path.join(".git"))
        {
            return Err(inaccessible());
        }
    }
    Ok(worktrees)
}

pub(super) fn check_head_available(root: &Path, head: &BareConversionHead) -> Result<()> {
    check_head_in_worktrees(head, &read_worktrees(root)?)
}

fn check_head_in_worktrees(head: &BareConversionHead, worktrees: &[Worktree]) -> Result<()> {
    let BareConversionHead::Branch { name, .. } = head else {
        return Ok(());
    };
    if let Some(worktree) = worktrees
        .iter()
        .find(|worktree| !worktree.bare && worktree.branch.as_deref() == Some(name))
    {
        return Err(refusal(BareConversionRefusal::BranchInUse {
            branch: name.clone(),
            worktree: worktree.path.clone(),
        }));
    }
    Ok(())
}

pub(super) fn config_bytes(root: &Path) -> Result<Vec<u8>> {
    let mut cmd = git_workdir_cmd_for(root);
    cmd.args(["config", "--includes", "--null", "--list"]);
    run_git_metadata_capture(cmd, "git config --includes --null --list")
}

fn parse_config(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (key, value) = match entry.iter().position(|byte| *byte == b'\n') {
                Some(index) => (&entry[..index], &entry[index + 1..]),
                None => (entry, b"".as_slice()),
            };
            let key = std::str::from_utf8(key).map_err(|error| {
                Error::new(ErrorKind::Backend("Invalid Git configuration key".into()))
                    .with_source(error)
            })?;
            Ok((key.to_ascii_lowercase(), value.to_vec()))
        })
        .collect()
}

fn worktree_config_enabled(root: &Path) -> Result<bool> {
    let mut cmd = git_workdir_cmd_for(root);
    cmd.args([
        "config",
        "--type=bool",
        "--get",
        "extensions.worktreeConfig",
    ]);
    let output = run_git_metadata_output(cmd, "git config extensions.worktreeConfig")?;
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    match output.stdout.as_slice() {
        b"true\n" | b"true\r\n" if output.status.success() => Ok(true),
        b"false\n" | b"false\r\n" if output.status.success() => Ok(false),
        _ => Err(Error::new(ErrorKind::Backend(
            "Invalid extensions.worktreeConfig value".into(),
        ))),
    }
}

fn worktree_bare_configured(root: &Path) -> Result<bool> {
    let mut command = git_workdir_cmd_for(root);
    command.args(["config", "--worktree", "--includes", "--get", "core.bare"]);
    let output = run_git_metadata_output(command, "git config --worktree core.bare")?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(crate::util::git_command_failed_error(
            "git config --worktree core.bare",
            output,
        )),
    }
}

fn check_worktree_config(root: &Path, config: &[(String, Vec<u8>)]) -> Result<()> {
    if let Some((_, value)) = config.iter().rev().find(|(key, _)| key == "core.worktree") {
        let configured = crate::util::path_buf_from_git_bytes(value, "core.worktree")?;
        let git_dir = native_path(root, "--absolute-git-dir")?;
        let configured = if configured.is_absolute() {
            configured
        } else {
            git_dir.join(configured)
        };
        if canonical_path(&configured).ok().as_deref() != Some(root) {
            return Err(refusal(BareConversionRefusal::ConflictingWorktree {
                path: configured,
            }));
        }
    }
    Ok(())
}

fn check_existing_checkout(root: &Path, git_dir: &Path, head: &BareConversionHead) -> Result<()> {
    let mut cmd = git_workdir_cmd_for(root);
    cmd.arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(root);
    cmd.args([
        "-c",
        "core.bare=false",
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
    ]);
    let output = run_git_metadata_capture(cmd, "git rev-parse repaired checkout")?;
    let output = output.strip_suffix(b"\n").unwrap_or(&output);
    let output = if cfg!(windows) {
        output.strip_suffix(b"\r").unwrap_or(output)
    } else {
        output
    };
    let top = crate::util::path_buf_from_git_bytes(output, "checkout top level")?;
    if canonical_path(&top)? != root {
        return Err(refusal(BareConversionRefusal::ConflictingWorktree {
            path: top,
        }));
    }
    if !git_dir.join("index").is_file() && head_commit(head).is_some() {
        return Err(refusal(BareConversionRefusal::ExistingIndex));
    }
    let mut cmd = git_workdir_cmd_for(root);
    cmd.arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(root);
    cmd.args(["-c", "core.bare=false", "ls-files", "--stage", "-z"]);
    run_git_metadata_capture(cmd, "git ls-files retained index")
        .map_err(|error| refusal(BareConversionRefusal::ExistingIndex).with_source(error))?;
    Ok(())
}

fn check_relocation_config(root: &Path, config: &[(String, Vec<u8>)]) -> Result<()> {
    for (key, value) in config {
        let reject = if key.starts_with("includeif.gitdir") || key.starts_with("includeif.worktree")
        {
            true
        } else if key == "include.path" || (key.starts_with("includeif.") && key.ends_with(".path"))
        {
            let path = expanded_config_path(value)?;
            !path.is_absolute()
                || path.starts_with(root)
                || canonical_path(&path).is_ok_and(|path| path.starts_with(root))
        } else if [
            "lfs.storage",
            "annex.objects",
            "annex.storage",
            "core.alternaterefscommand",
        ]
        .contains(&key.as_str())
        {
            true
        } else if key == "core.hookspath"
            || key == "core.attributesfile"
            || key == "core.excludesfile"
        {
            let path = expanded_config_path(value)?;
            !path.is_absolute()
                || path.starts_with(root)
                || canonical_path(&path).is_ok_and(|path| path.starts_with(root))
        } else {
            key.contains("annex") && (key.ends_with("directory") || key.ends_with("storage"))
        };
        if reject {
            return Err(refusal(BareConversionRefusal::RelocationDependentConfig {
                key: key.clone(),
            }));
        }
    }
    for relative in ["objects/info/alternates", "objects/info/http-alternates"] {
        if root.join(relative).exists() {
            return Err(refusal(BareConversionRefusal::RelocationDependentConfig {
                key: relative.into(),
            }));
        }
    }
    Ok(())
}

fn expanded_config_path(value: &[u8]) -> Result<PathBuf> {
    if let Some(relative) = value.strip_prefix(b"~/") {
        let home = gix::path::env::home_dir().ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "Cannot resolve home directory in Git configuration".into(),
            ))
        })?;
        return Ok(home.join(crate::util::path_buf_from_git_bytes(
            relative,
            "Git configured path",
        )?));
    }
    crate::util::path_buf_from_git_bytes(value, "Git configured path")
}

fn check_root_entries(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name() == "index" {
            return Err(refusal(BareConversionRefusal::ExistingIndex));
        }
        let allowed = entry.file_name().to_str().is_some_and(|name| {
            matches!(
                name,
                "HEAD"
                    | "config"
                    | "config.worktree"
                    | "description"
                    | "objects"
                    | "refs"
                    | "packed-refs"
                    | "logs"
                    | "hooks"
                    | "info"
                    | "branches"
                    | "shallow"
                    | "worktrees"
                    | "reftable"
                    | "FETCH_HEAD"
                    | "ORIG_HEAD"
                    | "COMMIT_EDITMSG"
                    | "rr-cache"
                    | "lfs"
                    | "annex"
                    | "gc.log"
                    | "gitk.cache"
            )
        });
        if !allowed {
            return Err(refusal(BareConversionRefusal::UnknownEntry {
                path: entry.path(),
            }));
        }
    }
    Ok(())
}

fn check_operations(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".lock")
            || matches!(
                name.as_ref(),
                "rebase-apply"
                    | "rebase-merge"
                    | "sequencer"
                    | "MERGE_HEAD"
                    | "CHERRY_PICK_HEAD"
                    | "REVERT_HEAD"
                    | "BISECT_START"
            )
        {
            return Err(refusal(BareConversionRefusal::ActiveOperation {
                path: entry.path(),
            }));
        }
        let metadata = entry.path().symlink_metadata()?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            check_operations(&entry.path())?;
        }
    }
    Ok(())
}

fn check_tree_paths(root: &Path, head: &BareConversionHead) -> Result<()> {
    let Some(commit) = head_commit(head) else {
        return Ok(());
    };
    let mut cmd = git_workdir_cmd_for(root);
    cmd.args([
        "ls-tree",
        "-r",
        "-z",
        "--name-only",
        "--full-tree",
        commit.as_ref(),
    ]);
    let output = run_git_metadata_capture(cmd, "git ls-tree checkout paths")?;
    let mut folded = BTreeMap::new();
    for bytes in output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = crate::util::path_buf_from_git_bytes(bytes, "checkout path")?;
        if bytes
            .split(|byte| *byte == b'/')
            .any(|part| part.eq_ignore_ascii_case(b".git"))
        {
            return Err(refusal(BareConversionRefusal::PathCollision { path }));
        }
        for end in bytes
            .iter()
            .enumerate()
            .filter_map(|(index, byte)| (*byte == b'/').then_some(index))
            .chain(std::iter::once(bytes.len()))
        {
            let prefix = &bytes[..end];
            if folded
                .insert(prefix.to_ascii_lowercase(), prefix.to_vec())
                .is_some_and(|previous| previous != prefix)
            {
                return Err(refusal(BareConversionRefusal::PathCollision { path }));
            }
        }
    }
    Ok(())
}

fn read_linked_worktrees(root: &Path, worktrees: &[Worktree]) -> Result<Vec<LinkedWorktree>> {
    let mut linked = Vec::new();
    for worktree in worktrees.iter().filter(|worktree| !worktree.bare) {
        let inaccessible = || {
            refusal(BareConversionRefusal::InaccessibleWorktree {
                path: worktree.path.clone(),
            })
        };
        let path = canonical_path(&worktree.path).map_err(|_| inaccessible())?;
        if path.starts_with(root) || !plain_directory(&path)? {
            return Err(inaccessible());
        }
        let dot_git = path.join(".git");
        if !fs::symlink_metadata(&dot_git)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            return Err(inaccessible());
        }
        let registration = native_path(&path, "--absolute-git-dir").map_err(|_| inaccessible())?;
        if registration.parent() != Some(root.join("worktrees").as_path()) {
            return Err(inaccessible());
        }
        if registration.join("modules").exists() {
            return Err(refusal(BareConversionRefusal::UnsupportedLayout {
                path: registration.join("modules"),
            }));
        }
        let effective_config = config_bytes(&path)?;
        let config = parse_config(&effective_config)?;
        check_relocation_config(root, &config)?;
        check_worktree_config(&path, &config)?;
        if let Some((_, value)) = config.iter().rev().find(|(key, _)| key == "core.worktree")
            && !expanded_config_path(value)?.is_absolute()
        {
            return Err(refusal(BareConversionRefusal::RelocationDependentConfig {
                key: "core.worktree".into(),
            }));
        }
        let registration_path = registration.join("gitdir");
        let recorded = fs::read(&registration_path)?;
        let recorded = recorded.strip_suffix(b"\n").unwrap_or(&recorded);
        let recorded = crate::util::path_buf_from_git_bytes(recorded, "linked worktree gitdir")?;
        if recorded != dot_git {
            return Err(inaccessible());
        }
        linked.push(LinkedWorktree {
            root: path,
            registration: registration
                .strip_prefix(root)
                .map_err(|_| inaccessible())?
                .to_path_buf(),
            pointer: fs::read(dot_git)?,
            effective_config,
        });
    }
    Ok(linked)
}

fn plain_directory(path: &Path) -> Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.is_dir() && !metadata.file_type().is_symlink())
}
