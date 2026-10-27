use super::GixRepo;
use super::bare_lifecycle::{ConversionLock, OperationLock};
use crate::util::{git_workdir_cmd_for, path_buf_from_git_bytes, run_git_metadata_capture};
use gitcomet_core::domain::{
    BareConversionHead, BareConversionLayout, BareConversionPreview, BareModeDisabled,
    RepositoryMode,
};
use gitcomet_core::error::{BareConversionRefusal, Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "bare_conversion_tests.rs"]
mod conversion_tests;
#[path = "bare_preflight.rs"]
pub(super) mod preflight;
#[path = "bare_storage.rs"]
pub(super) mod storage;
#[cfg(test)]
#[path = "bare_tests.rs"]
mod tests;

pub(crate) fn native_mode(root: &Path) -> Result<RepositoryMode> {
    let mut cmd = git_workdir_cmd_for(root);
    cmd.args(["rev-parse", "--is-bare-repository"]);
    match run_git_metadata_capture(cmd, "git rev-parse --is-bare-repository")?.as_slice() {
        b"true\n" | b"true\r\n" => Ok(RepositoryMode::Bare),
        b"false\n" | b"false\r\n" => Ok(RepositoryMode::WorkingTree),
        _ => Err(Error::new(ErrorKind::Backend(
            "Git returned an invalid repository mode".into(),
        ))),
    }
}

pub(crate) fn recover_before_open(
    root: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    storage::recover_before_open(root, cancellation.unwrap_or(&CancellationToken::new()))
}

pub(super) fn refusal(reason: BareConversionRefusal) -> Error {
    Error::new(ErrorKind::BareConversionRefused(reason))
}

fn native_path(root: &Path, flag: &str) -> Result<PathBuf> {
    let mut cmd = git_workdir_cmd_for(root);
    cmd.args(["rev-parse", "--path-format=absolute", flag]);
    let bytes = run_git_metadata_capture(cmd, "git rev-parse repository path")?;
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    let bytes = if cfg!(windows) {
        bytes.strip_suffix(b"\r").unwrap_or(bytes)
    } else {
        bytes
    };
    let path = path_buf_from_git_bytes(bytes, "Git repository path")?;
    canonical_path(&path)
}

pub(super) fn canonical_path(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .map(gitcomet_core::path_utils::strip_windows_verbatim_prefix)
        .map_err(Error::from)
}

pub(crate) fn discovery_lock(common_dir: &Path) -> Result<OperationLock> {
    OperationLock::acquire(&canonical_path(common_dir)?)
}

impl GixRepo {
    pub(crate) fn has_same_opened_mode(&self, other: &Self) -> bool {
        self.opened_topology.mode() == other.opened_topology.mode()
    }

    pub(super) fn repository_mode_impl(&self) -> Result<RepositoryMode> {
        let mode = native_mode(&self.spec.workdir)?;
        if mode != self.opened_topology.mode() {
            return Err(Error::new(ErrorKind::RepositoryChanged));
        }
        Ok(mode)
    }

    pub(crate) fn release_conversion_store(&self) -> Result<()> {
        let fresh = self.reopen_repo()?.into_sync();
        let common = self.shared_store().common.clone();
        common.invalidate();
        let mut fresh = fresh;
        let shared = common.attach(&mut fresh, self.private_store_scope);
        *self.config_repo.lock().expect("config repository") =
            super::config::ConfigRepo::new(fresh.to_thread_local());
        *self.store.write().expect("repo store") = super::RepoStore {
            repo: fresh,
            generation: shared.mapping_id,
            shared,
        };
        self.history_cache.invalidate_store();
        Ok(())
    }

    pub(crate) fn ensure_current(&self) -> Result<()> {
        self.lifecycle.check(self.lifecycle_generation)?;
        self.opened_topology.check_conversion_intent()?;
        self.opened_topology.validate(&self.spec.workdir)
    }

    pub(crate) fn operation_lease(&self) -> Result<super::bare_lifecycle::OperationLease> {
        let disk = self.opened_topology.acquire_operation_lock()?;
        let lease = self.lifecycle.begin(self.lifecycle_generation, disk)?;
        self.opened_topology.validate(&self.spec.workdir)?;
        Ok(lease)
    }

    pub(super) fn require_working_tree(&self) -> Result<()> {
        self.ensure_current()?;
        if native_mode(&self.spec.workdir)? == RepositoryMode::Bare {
            return Err(Error::new(ErrorKind::BareUnsupported));
        }
        Ok(())
    }

    pub(super) fn require_bare_head_available(&self) -> Result<()> {
        self.ensure_current()?;
        if native_mode(&self.spec.workdir)? == RepositoryMode::Bare {
            let head = preflight::read_head(&self.spec.workdir)?;
            preflight::check_head_available(&self.spec.workdir, &head)?;
        }
        Ok(())
    }

    pub(super) fn preview_disable_bare_mode_impl(&self) -> Result<BareConversionPreview> {
        let preflight = preflight::inspect(&self.spec.workdir)?;
        let preview = preflight.preview.clone();
        *self
            .conversion_preview
            .lock()
            .expect("bare conversion preview") = Some(preflight);
        Ok(preview)
    }

    pub(super) fn disable_bare_mode_impl(
        &self,
        preview: &BareConversionPreview,
        cancellation: &CancellationToken,
    ) -> Result<BareModeDisabled> {
        cancellation.check_cancelled()?;
        self.ensure_current()?;
        let source = self
            .conversion_preview
            .lock()
            .expect("bare conversion preview")
            .take()
            .filter(|source| source.preview == *preview)
            .ok_or_else(|| refusal(BareConversionRefusal::StalePreview))?;
        let _lock = ConversionLock::acquire(&source.common_dir, cancellation)?;
        let exclusive = self
            .lifecycle
            .convert(self.lifecycle_generation, cancellation)?;
        source.revalidate()?;
        cancellation.check_cancelled()?;
        crate::backend::release_conversion_stores(self)?;
        let result = match source.preview.layout {
            BareConversionLayout::ExistingCheckout => storage::repair_config(&source),
            BareConversionLayout::BareDirectory => storage::relocate(&source, cancellation),
        };
        exclusive.invalidate();
        result?;
        Ok(BareModeDisabled {
            root: preview.root.clone(),
            affected_roots: preview.affected_roots.clone(),
            old_common_dir: source.common_dir,
            new_common_dir: preview.root.join(".git"),
        })
    }
}

fn head_commit(head: &BareConversionHead) -> Option<&gitcomet_core::domain::CommitId> {
    match head {
        BareConversionHead::Branch { commit, .. } => commit.as_ref(),
        BareConversionHead::Detached { commit } => Some(commit),
    }
}
