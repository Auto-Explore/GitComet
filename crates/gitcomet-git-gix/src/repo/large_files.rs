use super::GixRepo;
use crate::util::{
    git_index_blob_spec, git_revision_blob_spec, git_stage_blob_spec, run_git_capture_bytes,
    run_git_raw_output, run_git_with_output,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CommandOutput, LargeFilePathKind, PathLargeFileInfo, RepoLargeFileCapabilities, Result,
};
use std::path::{Path, PathBuf};

impl GixRepo {
    pub(super) fn large_file_capabilities_impl(&self) -> Result<RepoLargeFileCapabilities> {
        let uses_git_lfs = worktree_mentions_filter(&self.spec.workdir, "lfs")?;
        let uses_git_annex = self.repo_common_dir().join("annex").is_dir()
            || worktree_mentions_filter(&self.spec.workdir, "annex")?;

        Ok(RepoLargeFileCapabilities {
            uses_git_lfs,
            git_lfs_available: self.git_subcommand_available("lfs", "version"),
            uses_git_annex,
            git_annex_available: self.git_subcommand_available("annex", "version"),
        })
    }

    pub(super) fn large_file_path_info_impl(&self, path: &Path) -> Result<PathLargeFileInfo> {
        let relative_path = repo_relative_path(&self.spec.workdir, path)?;
        let kind = if path_is_locked_annex_symlink(
            &self.spec.workdir,
            &self.repo_common_dir(),
            &relative_path,
        )? {
            LargeFilePathKind::GitAnnexLocked
        } else {
            match self.git_check_attr_filter(&relative_path)?.as_deref() {
                Some("lfs") => LargeFilePathKind::GitLfs,
                Some("annex") => LargeFilePathKind::GitAnnexUnlocked,
                _ => LargeFilePathKind::Plain,
            }
        };

        Ok(PathLargeFileInfo {
            path: relative_path,
            kind,
        })
    }

    pub(super) fn materialize_revision_path_bytes(&self, revision: &str, path: &Path) -> Result<Vec<u8>> {
        let spec = git_revision_blob_spec(revision, path)?;
        self.git_cat_file_filters(spec)
    }

    pub(super) fn materialize_index_path_bytes(&self, path: &Path) -> Result<Vec<u8>> {
        let spec = git_index_blob_spec(path)?;
        self.git_cat_file_filters(spec)
    }

    pub(super) fn materialize_stage_path_bytes(&self, stage: u8, path: &Path) -> Result<Vec<u8>> {
        let spec = git_stage_blob_spec(stage, path)?;
        self.git_cat_file_filters(spec)
    }

    pub(super) fn write_materialized_preview_cache(
        &self,
        logical_path: &Path,
        cache_key: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        let cache_path = materialized_preview_cache_path(&self.spec.workdir, logical_path, cache_key);
        if std::fs::metadata(&cache_path).is_ok_and(|m| m.is_file()) {
            return Ok(cache_path);
        }

        let mut tmp_file =
            tempfile::NamedTempFile::new_in(std::env::temp_dir()).map_err(io_err_to_error)?;
        use std::io::Write as _;
        tmp_file.write_all(bytes).map_err(io_err_to_error)?;
        tmp_file.flush().map_err(io_err_to_error)?;

        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent).map_err(io_err_to_error)?;
        }
        match tmp_file.persist(&cache_path) {
            Ok(_) => Ok(cache_path),
            Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(cache_path),
            Err(err) => Err(io_err_to_error(err.error)),
        }
    }

    pub(super) fn lfs_fetch_with_output_impl(&self) -> Result<CommandOutput> {
        self.ensure_git_lfs_available()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("lfs").arg("fetch");
        run_git_with_output(cmd, "git lfs fetch")
    }

    pub(super) fn lfs_pull_with_output_impl(&self) -> Result<CommandOutput> {
        self.ensure_git_lfs_available()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("lfs").arg("pull");
        run_git_with_output(cmd, "git lfs pull")
    }

    pub(super) fn lfs_track_with_output_impl(&self, pattern: &str) -> Result<CommandOutput> {
        self.ensure_git_lfs_available()?;
        validate_nonempty_arg(pattern, "git lfs track pattern")?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("lfs").arg("track").arg(pattern);
        run_git_with_output(cmd, "git lfs track")
    }

    pub(super) fn lfs_untrack_with_output_impl(&self, pattern: &str) -> Result<CommandOutput> {
        self.ensure_git_lfs_available()?;
        validate_nonempty_arg(pattern, "git lfs untrack pattern")?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("lfs").arg("untrack").arg(pattern);
        run_git_with_output(cmd, "git lfs untrack")
    }

    pub(super) fn lfs_prune_with_output_impl(&self) -> Result<CommandOutput> {
        self.ensure_git_lfs_available()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("lfs").arg("prune");
        run_git_with_output(cmd, "git lfs prune")
    }

    pub(super) fn lfs_migrate_import_with_output_impl(
        &self,
        pattern: &str,
    ) -> Result<CommandOutput> {
        self.ensure_git_lfs_available()?;
        validate_nonempty_arg(pattern, "git lfs migrate import pattern")?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("lfs")
            .arg("migrate")
            .arg("import")
            .arg("--include")
            .arg(pattern);
        run_git_with_output(cmd, "git lfs migrate import")
    }

    pub(super) fn annex_init_with_output_impl(&self) -> Result<CommandOutput> {
        self.ensure_git_annex_available()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex").arg("init");
        run_git_with_output(cmd, "git annex init")
    }

    pub(super) fn annex_sync_with_output_impl(&self) -> Result<CommandOutput> {
        self.ensure_git_annex_available()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex").arg("sync");
        run_git_with_output(cmd, "git annex sync")
    }

    pub(super) fn annex_get_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        self.ensure_git_annex_available()?;
        let relative_path = repo_relative_path(&self.spec.workdir, path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex").arg("get").arg("--").arg(&relative_path);
        run_git_with_output(cmd, "git annex get")
    }

    pub(super) fn annex_unlock_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        self.ensure_git_annex_available()?;
        let relative_path = repo_relative_path(&self.spec.workdir, path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex")
            .arg("unlock")
            .arg("--")
            .arg(&relative_path);
        run_git_with_output(cmd, "git annex unlock")
    }

    pub(super) fn annex_lock_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        self.ensure_git_annex_available()?;
        let relative_path = repo_relative_path(&self.spec.workdir, path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex").arg("lock").arg("--").arg(&relative_path);
        run_git_with_output(cmd, "git annex lock")
    }

    pub(super) fn annex_add_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        self.ensure_git_annex_available()?;
        let relative_path = repo_relative_path(&self.spec.workdir, path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex").arg("add").arg("--").arg(&relative_path);
        run_git_with_output(cmd, "git annex add")
    }

    pub(super) fn annex_drop_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        self.ensure_git_annex_available()?;
        let relative_path = repo_relative_path(&self.spec.workdir, path)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex").arg("drop").arg("--").arg(&relative_path);
        run_git_with_output(cmd, "git annex drop")
    }

    fn repo_common_dir(&self) -> PathBuf {
        self._repo.to_thread_local().common_dir().to_path_buf()
    }

    fn git_subcommand_available(&self, top_level: &str, nested: &str) -> bool {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg(top_level).arg(nested);
        run_git_raw_output(cmd, "git tool probe")
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn ensure_git_lfs_available(&self) -> Result<()> {
        if self.git_subcommand_available("lfs", "version") {
            return Ok(());
        }
        Err(Error::new(ErrorKind::Backend(
            "git lfs is not available in this environment".to_string(),
        )))
    }

    fn ensure_git_annex_available(&self) -> Result<()> {
        if self.git_subcommand_available("annex", "version") {
            return Ok(());
        }
        Err(Error::new(ErrorKind::Backend(
            "git annex is not available in this environment".to_string(),
        )))
    }

    fn git_check_attr_filter(&self, path: &Path) -> Result<Option<String>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("check-attr")
            .arg("-z")
            .arg("filter")
            .arg("--")
            .arg(path);
        let output = run_git_capture_bytes(cmd, "git check-attr")?;
        let mut fields = output.split(|b| *b == b'\0');
        let _path = fields.next();
        let attr = fields.next().and_then(|field| std::str::from_utf8(field).ok());
        let value = fields.next().and_then(|field| std::str::from_utf8(field).ok());
        if attr != Some("filter") {
            return Ok(None);
        }
        Ok(match value {
            Some("lfs") => Some("lfs".to_string()),
            Some("annex") => Some("annex".to_string()),
            _ => None,
        })
    }

    fn git_cat_file_filters(&self, spec: std::ffi::OsString) -> Result<Vec<u8>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("cat-file").arg("--filters").arg(spec);
        run_git_capture_bytes(cmd, "git cat-file --filters")
    }
}

fn validate_nonempty_arg(value: &str, kind: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "{kind} must not be empty"
        ))));
    }
    if value.starts_with('-') {
        return Err(Error::new(ErrorKind::Backend(format!(
            "{kind} must not start with '-'"
        ))));
    }
    Ok(())
}

fn repo_relative_path(workdir: &Path, path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return path.strip_prefix(workdir).map(|p| p.to_path_buf()).map_err(|_| {
            Error::new(ErrorKind::Backend(format!(
                "path '{}' is outside repository workdir '{}'",
                path.display(),
                workdir.display()
            )))
        });
    }
    Ok(path.to_path_buf())
}

fn worktree_mentions_filter(workdir: &Path, filter_name: &str) -> Result<bool> {
    fn visit(dir: &Path, filter_name: &str) -> Result<bool> {
        let entries = std::fs::read_dir(dir).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            if file_type.is_dir() {
                if entry.file_name() == ".git" {
                    continue;
                }
                if visit(&path, filter_name)? {
                    return Ok(true);
                }
                continue;
            }
            if entry.file_name() != ".gitattributes" {
                continue;
            }
            let contents = std::fs::read_to_string(&path)
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            if contents.contains(&format!("filter={filter_name}")) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    visit(workdir, filter_name)
}

fn path_is_locked_annex_symlink(workdir: &Path, common_dir: &Path, path: &Path) -> Result<bool> {
    let full_path = workdir.join(path);
    let metadata = match std::fs::symlink_metadata(&full_path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(Error::new(ErrorKind::Io(e.kind()))),
    };
    if !metadata.file_type().is_symlink() {
        return Ok(false);
    }

    let target = std::fs::read_link(&full_path).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    let resolved = if target.is_absolute() {
        target
    } else {
        full_path
            .parent()
            .unwrap_or(workdir)
            .join(target)
    };
    Ok(path_starts_with_lexical(
        &resolved,
        &common_dir.join("annex").join("objects"),
    ))
}

fn path_starts_with_lexical(path: &Path, prefix: &Path) -> bool {
    let normalized_path = lexicalize_path(path);
    let normalized_prefix = lexicalize_path(prefix);
    normalized_path.starts_with(&normalized_prefix)
}

fn lexicalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn materialized_preview_cache_path(workdir: &Path, logical_path: &Path, cache_key: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};

    let mut hasher = rustc_hash::FxHasher::default();
    workdir.hash(&mut hasher);
    logical_path.hash(&mut hasher);
    cache_key.hash(&mut hasher);
    let hash = hasher.finish();
    let suffix = logical_path
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "gitcomet-diff-preview-materialized-{hash:016x}{suffix}"
    ))
}

fn io_err_to_error(error: std::io::Error) -> Error {
    Error::new(ErrorKind::Io(error.kind()))
}
