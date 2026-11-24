mod diff_summary;

use super::history::gix_head_id_or_none;
use super::{GixRepo, object_id_from_commit_id, oid_to_arc_str};
use crate::util::{
    bytes_to_text_preserving_utf8, fnv1a_64, git_workdir_cmd_for, push_literal_paths,
    run_git_simple, run_git_with_output, stable_path_bytes, submodule_script_path,
};
use diff_summary::{submodule_commit_diff_summary, submodule_worktree_diff_summary};
use gitcomet_core::domain::{
    CommitId, DiffTarget, Submodule, SubmoduleDiffSummary, SubmoduleStatus,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::remote_url::{RemoteUrlPolicy, validate_remote_url_with_policy};
use gitcomet_core::services::{
    CancellationToken, CommandOutput, Result, SubmoduleTrustDecision, SubmoduleTrustTarget,
};
use gitcomet_core::text_utils::redact_url_userinfo;
use gix::bstr::ByteSlice as _;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

const GIT_CONFIG_CONTENTION_RETRIES: usize = 6;
const GIT_CONFIG_CONTENTION_RETRY_DELAY: Duration = Duration::from_millis(25);

fn allow_file_submodule_transport(cmd: &mut Command) {
    // `git submodule` blocks local-path remotes unless `protocol.file.allow` is enabled.
    // Use per-command config so local workflows keep working without disabling `https`/`ssh`.
    cmd.arg("-c").arg("protocol.file.allow=always");
}

impl GixRepo {
    pub(super) fn head_path_is_gitlink_impl(&self, path: &Path) -> Result<bool> {
        let path = if path.is_absolute() {
            path.strip_prefix(&self.spec.workdir).map_err(|_| {
                Error::new(ErrorKind::Backend(format!(
                    "submodule path '{}' is outside repository '{}'",
                    path.display(),
                    self.spec.workdir.display()
                )))
            })?
        } else {
            path
        };
        let repo = self.reopen_repo()?;
        Ok(head_gitlink_commit_id(&repo, path)?.is_some())
    }

    pub(super) fn list_submodules_impl(&self) -> Result<Vec<Submodule>> {
        self.list_submodules_cancellable_impl(&CancellationToken::new())
    }

    pub(super) fn list_submodules_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Submodule>> {
        cancellation.check_cancelled()?;
        // A fresh open sees `.gitmodules` and configuration as they are now.
        // The index is the one this repository already holds, as status
        // reads it: a second copy of a large one costs as much as the rest.
        let gitlinks = collect_gitlinks(&self.repo(), cancellation)?;
        let repo = self.reopen_repo()?;
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(SUBMODULE_LISTERS_MAX);
        list_submodules_in(&repo, gitlinks, workers, cancellation)
    }

    pub(super) fn submodule_diff_summary_cancellable_impl(
        &self,
        target: &DiffTarget,
        cancellation: &CancellationToken,
    ) -> Result<SubmoduleDiffSummary> {
        cancellation.check_cancelled()?;
        let repo = self.reopen_repo()?;
        match target {
            DiffTarget::WorkingTree { path, .. } => {
                submodule_worktree_diff_summary(&repo, path, cancellation)
            }
            DiffTarget::Commit {
                commit_id, path, ..
            } => submodule_commit_diff_summary(&repo, commit_id, path, cancellation),
            _ => Err(Error::new(ErrorKind::Unsupported(
                "submodule summaries require a submodule working-tree target or committed submodule path",
            ))),
        }
    }

    pub(super) fn check_submodule_add_trust_impl(
        &self,
        url: &str,
        path: &Path,
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<SubmoduleTrustDecision> {
        validate_remote_url_with_policy(url, remote_url_policy)?;
        let repo = self.reopen_repo()?;
        let Some(target) = trust_target_from_raw_source(submodule_repo_workdir(&repo), path, url)?
        else {
            return Ok(SubmoduleTrustDecision::Proceed);
        };

        if submodule_source_trusted(submodule_repo_workdir(&repo), &target)? {
            Ok(SubmoduleTrustDecision::Proceed)
        } else {
            Ok(SubmoduleTrustDecision::Prompt {
                sources: vec![target],
            })
        }
    }

    pub(super) fn check_submodule_update_trust_impl(
        &self,
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<SubmoduleTrustDecision> {
        let repo = self.reopen_repo()?;
        let trust_root = submodule_repo_workdir(&repo);
        let mut sources = BTreeMap::new();
        collect_repo_untrusted_submodule_sources(
            &repo,
            trust_root,
            Path::new(""),
            &mut sources,
            remote_url_policy,
        )?;
        if sources.is_empty() {
            Ok(SubmoduleTrustDecision::Proceed)
        } else {
            Ok(SubmoduleTrustDecision::Prompt {
                sources: sources.into_values().collect(),
            })
        }
    }

    pub(super) fn check_submodule_load_trust_impl(
        &self,
        path: &Path,
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<SubmoduleTrustDecision> {
        let repo = self.reopen_repo()?;
        let trust_root = submodule_repo_workdir(&repo);
        let mut sources = BTreeMap::new();
        let found = collect_target_submodule_untrusted_sources(
            &repo,
            trust_root,
            Path::new(""),
            path,
            &mut sources,
            remote_url_policy,
        )?;
        if !found {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not configured in this repository",
                path.display()
            ))));
        }
        if sources.is_empty() {
            Ok(SubmoduleTrustDecision::Proceed)
        } else {
            Ok(SubmoduleTrustDecision::Prompt {
                sources: sources.into_values().collect(),
            })
        }
    }

    pub(super) fn add_submodule_with_output_impl(
        &self,
        url: &str,
        path: &Path,
        branch: Option<&str>,
        name: Option<&str>,
        force: bool,
        approved_sources: &[SubmoduleTrustTarget],
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        validate_remote_url_with_policy(url, remote_url_policy)?;
        let repo = self.reopen_repo()?;
        let trust_root = submodule_repo_workdir(&repo);
        let git_dir = repo.git_dir().to_path_buf();
        persist_submodule_trust_approvals(trust_root, approved_sources)?;

        let mut cmd = self.git_workdir_cmd();
        if let Some(target) = trust_target_from_raw_source(trust_root, path, url)? {
            if !submodule_source_trusted(trust_root, &target)? {
                return Err(untrusted_local_submodule_error(&target, "add"));
            }
            allow_file_submodule_transport(&mut cmd);
        }
        let logical_name = name
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
        validate_submodule_git_dir_name(&logical_name)?;

        let command = push_submodule_add_args(&mut cmd, url, path, branch, name, force);
        match run_git_with_output(cmd, &command) {
            Ok(output) => Ok(output),
            Err(err) => Err(cleanup_failed_submodule_add_error(
                trust_root,
                &git_dir,
                path,
                &logical_name,
                err,
            )),
        }
    }

    pub(super) fn update_submodules_with_output_impl(
        &self,
        approved_sources: &[SubmoduleTrustTarget],
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let trust_root = submodule_repo_workdir(&repo).to_path_buf();
        persist_submodule_trust_approvals(&trust_root, approved_sources)?;

        let mut outputs = Vec::new();
        update_repo_submodules_recursive(
            &repo,
            &trust_root,
            Path::new(""),
            &mut outputs,
            remote_url_policy,
        )?;

        if outputs.is_empty() {
            Ok(CommandOutput::empty_success(
                "git submodule update --init --recursive",
            ))
        } else {
            Ok(combine_submodule_update_outputs(outputs))
        }
    }

    pub(super) fn load_submodule_with_output_impl(
        &self,
        path: &Path,
        approved_sources: &[SubmoduleTrustTarget],
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let trust_root = submodule_repo_workdir(&repo).to_path_buf();
        persist_submodule_trust_approvals(&trust_root, approved_sources)?;

        let mut outputs = Vec::new();
        let found = load_target_submodule_recursive(
            &repo,
            &trust_root,
            Path::new(""),
            path,
            &mut outputs,
            remote_url_policy,
        )?;
        if !found {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not configured in this repository",
                path.display()
            ))));
        }
        if outputs.is_empty() {
            Ok(CommandOutput::empty_success(format!(
                "git submodule update --init -- {}",
                path.display()
            )))
        } else {
            Ok(CommandOutput::combine(
                format!("Load submodule {}", path.display()),
                &outputs,
            ))
        }
    }

    pub(super) fn change_submodule_pointer_with_output_impl(
        &self,
        path: &Path,
        reference: &str,
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let Some(nested_repo) = open_gitlink_repo(&repo, path)? else {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' is not initialized",
                path.display()
            ))));
        };

        let nested_workdir = submodule_repo_workdir(&repo).join(path);
        let nested_status_repo =
            GixRepo::new(nested_workdir.clone(), nested_repo.clone().into_sync())?;
        let nested_status = nested_status_repo.status_impl()?;
        if !nested_status.staged.is_empty() || !nested_status.unstaged.is_empty() {
            return Err(Error::new(ErrorKind::Backend(format!(
                "submodule '{}' has inner changes. Commit, stash, or discard them before changing the pointer.",
                path.display()
            ))));
        }

        let target_commit = resolve_submodule_target_commit_id(&nested_repo, reference)?;
        let target_commit_hex = target_commit.to_string();

        let mut checkout_cmd = git_workdir_cmd_for(&nested_workdir);
        checkout_cmd
            .arg("checkout")
            .arg("--detach")
            .arg(target_commit_hex.as_str());
        let checkout_output = run_git_with_output(
            checkout_cmd,
            &format!("git checkout --detach {}", target_commit_hex),
        )?;

        let mut stage_cmd = self.git_workdir_cmd();
        stage_cmd.arg("add");
        push_literal_paths(&mut stage_cmd, [path]);
        let stage_output =
            run_git_with_output(stage_cmd, &format!("git add -- {}", path.display()))?;

        Ok(CommandOutput::combine(
            format!("Change submodule pointer {}", path.display()),
            &[checkout_output, stage_output],
        ))
    }

    pub(super) fn remove_submodule_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let workdir = submodule_repo_workdir(&repo).to_path_buf();
        let git_dir = repo.git_dir().to_path_buf();
        let logical_name =
            resolve_submodule_logical_name(&repo, path)?.unwrap_or_else(|| path.to_path_buf());

        let mut cmd1 = self.git_workdir_cmd();
        cmd1.arg("submodule").arg("deinit").arg("-f");
        push_literal_paths(&mut cmd1, [submodule_script_path(path)]);
        let out1 =
            run_git_with_output(cmd1, &format!("git submodule deinit -f {}", path.display()))?;

        let mut cmd2 = self.git_workdir_cmd();
        cmd2.arg("rm").arg("-f");
        push_literal_paths(&mut cmd2, [path]);
        let out2 = run_git_with_output(cmd2, &format!("git rm -f {}", path.display()))?;

        cleanup_removed_submodule_metadata(&workdir, &git_dir, &logical_name).map_err(|err| {
            Error::new(ErrorKind::Backend(format!(
                "Removed submodule '{}' from the worktree and index, but failed to clean metadata: {err}",
                path.display()
            ))).with_source(err)
})?;

        Ok(CommandOutput {
            command: format!("Remove submodule {}", path.display()),
            stdout: [out1.stdout.trim_end(), out2.stdout.trim_end()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            stderr: [out1.stderr.trim_end(), out2.stderr.trim_end()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            exit_code: Some(0),
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct GitlinkIndexState {
    kind: Option<gix::hash::Kind>,
    index_id: Option<gix::ObjectId>,
    conflict: bool,
}

impl GitlinkIndexState {
    fn record(&mut self, id: gix::ObjectId, stage: gix::index::entry::Stage) {
        self.kind.get_or_insert(id.kind());
        if stage == gix::index::entry::Stage::Unconflicted {
            self.index_id = Some(id);
        } else {
            self.conflict = true;
        }
    }

    fn null_head(self, repo: &gix::Repository) -> CommitId {
        CommitId(
            self.kind
                .unwrap_or_else(|| repo.object_hash())
                .null()
                .to_string()
                .into(),
        )
    }

    fn index_head_or_null(self, repo: &gix::Repository) -> CommitId {
        self.index_id
            .map(object_id_to_commit_id)
            .unwrap_or_else(|| self.null_head(repo))
    }
}

/// Threads listing a repository's submodules, and the submodules it must
/// have for more than one: each is opened, and its own submodules listed.
const SUBMODULE_LISTERS_MAX: usize = 8;
const SUBMODULE_LISTERS_MIN_SUBMODULES: usize = 16;

/// One gitlink of a repository to list, with everything below it.
enum SubmoduleTask {
    /// By its place among the submodules `.gitmodules` configures.
    Configured {
        at: usize,
        relative_path: PathBuf,
        gitlink: GitlinkIndexState,
    },
    /// A gitlink no configured submodule has as its path.
    Unmapped {
        relative_path: PathBuf,
        gitlink: GitlinkIndexState,
    },
}

#[cfg(test)]
thread_local! {
    /// Runs once on the listing thread just before its helpers start.
    static BEFORE_LISTERS: std::cell::Cell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::Cell::new(None) };
}

/// Each configured submodule's name and path, which positions in
/// [`SubmoduleTask::Configured`] refer to.
fn submodule_mapping(
    submodules: &[gix::Submodule<'_>],
) -> Vec<(gix::bstr::BString, Option<gix::bstr::BString>)> {
    submodules
        .iter()
        .map(|submodule| (submodule.name().to_owned(), submodule.path().ok()))
        .collect()
}

/// The configured submodules of `repo`, in the order gix names them.
fn configured_submodules(repo: &gix::Repository) -> Result<Vec<gix::Submodule<'_>>> {
    let submodules = crate::refs::submodules(repo).map_err(|e| {
        Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))).with_source(e)
    })?;
    Ok(submodules.map(Iterator::collect).unwrap_or_default())
}

/// What there is to list: each configured submodule that has a gitlink, by
/// its first mention, then the gitlinks left over.
fn submodule_tasks(
    submodules: &[gix::Submodule<'_>],
    mut gitlinks: BTreeMap<PathBuf, GitlinkIndexState>,
    cancellation: &CancellationToken,
) -> Result<Vec<SubmoduleTask>> {
    let mut tasks = Vec::with_capacity(gitlinks.len());
    for (at, submodule) in submodules.iter().enumerate() {
        cancellation.check_cancelled()?;
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        if let Some(gitlink) = gitlinks.remove(&relative_path) {
            tasks.push(SubmoduleTask::Configured {
                at,
                relative_path,
                gitlink,
            });
        }
    }
    tasks.extend(
        gitlinks
            .into_iter()
            .map(|(relative_path, gitlink)| SubmoduleTask::Unmapped {
                relative_path,
                gitlink,
            }),
    );
    Ok(tasks)
}

/// The row of one gitlink and the rows of every submodule below it.
fn list_submodule(
    repo: &gix::Repository,
    submodules: &[gix::Submodule<'_>],
    prefix: &Path,
    task: &SubmoduleTask,
    cancellation: &CancellationToken,
) -> Result<Vec<Submodule>> {
    cancellation.check_cancelled()?;
    let mut rows = Vec::new();
    let (full_path, nested_repo) = match task {
        SubmoduleTask::Configured {
            at,
            relative_path,
            gitlink,
        } => {
            let full_path = prefix.join(relative_path);
            let (row, nested_repo) = configured_submodule_row(
                repo,
                submodules[*at].clone(),
                full_path.clone(),
                *gitlink,
            )?;
            rows.push(row);
            (full_path, nested_repo)
        }
        SubmoduleTask::Unmapped {
            relative_path,
            gitlink,
        } => {
            let full_path = prefix.join(relative_path);
            rows.push(Submodule {
                path: full_path.clone(),
                recorded_head: gitlink.index_head_or_null(repo),
                checked_out_head: None,
                status: SubmoduleStatus::MissingMapping,
            });
            (full_path, open_gitlink_repo(repo, relative_path)?)
        }
    };
    cancellation.check_cancelled()?;
    if let Some(nested_repo) = nested_repo {
        collect_nested_submodules(&nested_repo, &full_path, &mut rows, cancellation)?;
    }
    Ok(rows)
}

/// Lists a repository below the top one, on the thread that reached it.
fn collect_repo_submodules(
    repo: &gix::Repository,
    prefix: &Path,
    out: &mut Vec<Submodule>,
    cancellation: &CancellationToken,
) -> Result<()> {
    cancellation.check_cancelled()?;
    let gitlinks = collect_gitlinks(repo, cancellation)?;
    let submodules = configured_submodules(repo)?;
    for task in submodule_tasks(&submodules, gitlinks, cancellation)? {
        out.extend(list_submodule(
            repo,
            &submodules,
            prefix,
            &task,
            cancellation,
        )?);
    }
    Ok(())
}

/// Whether a failed nested enumeration prunes that subtree instead of failing
/// the whole listing. Everything but cancellation does: narrowing it to the
/// corrupt-repository kinds would let one unreadable gitlink empty the whole
/// Submodules section, and a pruned subtree only looks like an empty one.
fn nested_failure_prunes_subtree(kind: &ErrorKind) -> bool {
    match kind {
        ErrorKind::Backend(_)
        | ErrorKind::Unsupported(_)
        | ErrorKind::NotARepository
        | ErrorKind::BareUnsupported
        | ErrorKind::BareConversionRefused(_)
        | ErrorKind::Io(_)
        | ErrorKind::Git(_) => true,
        ErrorKind::Cancelled
        | ErrorKind::RepositoryChanged
        | ErrorKind::BareConversionRecovery { .. } => false,
    }
}

/// Recurse into one submodule; a broken one prunes only its own subtree,
/// rather than failing the whole listing.
fn collect_nested_submodules(
    nested_repo: &gix::Repository,
    full_path: &Path,
    out: &mut Vec<Submodule>,
    cancellation: &CancellationToken,
) -> Result<()> {
    let collected = out.len();
    match collect_repo_submodules(nested_repo, full_path, out, cancellation) {
        Ok(()) => Ok(()),
        Err(error) if nested_failure_prunes_subtree(error.kind()) => {
            out.truncate(collected);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Append the `git submodule add` arguments and return the display label.
///
/// `--` keeps a user-typed URL or path that starts with `-` out of the option
/// parser (`--reference=<path>` would borrow objects from an arbitrary repo).
fn push_submodule_add_args(
    cmd: &mut Command,
    url: &str,
    path: &Path,
    branch: Option<&str>,
    name: Option<&str>,
    force: bool,
) -> String {
    cmd.arg("submodule").arg("add");
    let mut command = "git submodule add".to_string();
    if let Some(branch) = branch {
        cmd.arg("--branch").arg(branch);
        command.push_str(&format!(" --branch {branch}"));
    }
    if force {
        cmd.arg("--force");
        command.push_str(" --force");
    }
    if let Some(name) = name {
        cmd.arg("--name").arg(name);
        command.push_str(&format!(" --name {name}"));
    }
    // The label stays a human-readable summary; `--` is an argv concern only,
    // and credentials in the URL are masked because the label is displayed.
    push_literal_paths(
        cmd,
        [std::ffi::OsStr::new(url), &*submodule_script_path(path)],
    );
    command.push_str(&format!(" {} {}", redact_url_userinfo(url), path.display()));
    command
}

/// A configured submodule with its path relative to its repository and joined
/// onto the walk's prefix.
struct SubmoduleEntry<'repo> {
    submodule: gix::Submodule<'repo>,
    relative_path: PathBuf,
    full_path: PathBuf,
}

/// `repo`'s configured submodules in order, each resolved when reached so a walk
/// that stops at its target never reads the entries after it.
fn submodule_entries<'repo>(
    repo: &'repo gix::Repository,
    prefix: &Path,
) -> Result<impl Iterator<Item = Result<SubmoduleEntry<'repo>>> + 'repo> {
    let submodules = crate::refs::submodules(repo).map_err(|e| {
        Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))).with_source(e)
    })?;
    let prefix = prefix.to_path_buf();
    Ok(submodules.into_iter().flatten().map(move |submodule| {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        let full_path = prefix.join(&relative_path);
        Ok(SubmoduleEntry {
            submodule,
            relative_path,
            full_path,
        })
    }))
}

/// Records `submodule`'s source if it is local and untrusted, then its nested ones.
fn collect_untrusted_submodule_source(
    current_workdir: &Path,
    trust_root: &Path,
    entry: &SubmoduleEntry<'_>,
    out: &mut BTreeMap<PathBuf, SubmoduleTrustTarget>,
    remote_url_policy: RemoteUrlPolicy,
) -> Result<()> {
    let SubmoduleEntry {
        submodule,
        full_path,
        ..
    } = entry;
    if let Some(target) =
        trust_target_from_submodule(current_workdir, full_path, submodule, remote_url_policy)?
        && !submodule_source_trusted(trust_root, &target)?
    {
        out.insert(full_path.to_path_buf(), target);
    }
    if let Some(nested_repo) = open_configured_submodule_repo(submodule)? {
        collect_repo_untrusted_submodule_sources(
            &nested_repo,
            trust_root,
            full_path,
            out,
            remote_url_policy,
        )?;
    }
    Ok(())
}

/// `git submodule update --init` for one submodule of `repo`, refusing an
/// untrusted local source, then the same for everything nested in it.
fn update_submodule_recursive(
    repo: &gix::Repository,
    trust_root: &Path,
    entry: &SubmoduleEntry<'_>,
    outputs: &mut Vec<CommandOutput>,
    remote_url_policy: RemoteUrlPolicy,
) -> Result<()> {
    let SubmoduleEntry {
        submodule,
        relative_path,
        full_path,
    } = entry;
    let current_workdir = submodule_repo_workdir(repo);
    let local_target =
        trust_target_from_submodule(current_workdir, full_path, submodule, remote_url_policy)?;
    let mut cmd = git_workdir_cmd_for(current_workdir);
    if let Some(target) = local_target.as_ref() {
        if !submodule_source_trusted(trust_root, target)? {
            return Err(untrusted_local_submodule_error(target, "update"));
        }
        allow_file_submodule_transport(&mut cmd);
    }

    cmd.arg("submodule").arg("update").arg("--init");
    push_literal_paths(&mut cmd, [submodule_script_path(relative_path)]);
    outputs.push(run_git_with_output(
        cmd,
        &format!("git submodule update --init -- {}", full_path.display()),
    )?);

    if let Some(nested_repo) = open_gitlink_repo(repo, relative_path)? {
        update_repo_submodules_recursive(
            &nested_repo,
            trust_root,
            full_path,
            outputs,
            remote_url_policy,
        )?;
    }
    Ok(())
}

fn collect_repo_untrusted_submodule_sources(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    out: &mut BTreeMap<PathBuf, SubmoduleTrustTarget>,
    remote_url_policy: RemoteUrlPolicy,
) -> Result<()> {
    let current_workdir = submodule_repo_workdir(repo);
    for entry in submodule_entries(repo, prefix)? {
        let entry = entry?;
        collect_untrusted_submodule_source(
            current_workdir,
            trust_root,
            &entry,
            out,
            remote_url_policy,
        )?;
    }

    Ok(())
}

fn update_repo_submodules_recursive(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    outputs: &mut Vec<CommandOutput>,
    remote_url_policy: RemoteUrlPolicy,
) -> Result<()> {
    for entry in submodule_entries(repo, prefix)? {
        let entry = entry?;
        update_submodule_recursive(repo, trust_root, &entry, outputs, remote_url_policy)?;
    }

    Ok(())
}

fn collect_target_submodule_untrusted_sources(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    target_path: &Path,
    out: &mut BTreeMap<PathBuf, SubmoduleTrustTarget>,
    remote_url_policy: RemoteUrlPolicy,
) -> Result<bool> {
    let current_workdir = submodule_repo_workdir(repo);
    for entry in submodule_entries(repo, prefix)? {
        let entry = entry?;
        if entry.full_path == target_path {
            collect_untrusted_submodule_source(
                current_workdir,
                trust_root,
                &entry,
                out,
                remote_url_policy,
            )?;
            return Ok(true);
        }

        if target_path.starts_with(&entry.full_path)
            && let Some(nested_repo) = open_configured_submodule_repo(&entry.submodule)?
            && collect_target_submodule_untrusted_sources(
                &nested_repo,
                trust_root,
                &entry.full_path,
                target_path,
                out,
                remote_url_policy,
            )?
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn load_target_submodule_recursive(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    target_path: &Path,
    outputs: &mut Vec<CommandOutput>,
    remote_url_policy: RemoteUrlPolicy,
) -> Result<bool> {
    for entry in submodule_entries(repo, prefix)? {
        let entry = entry?;
        if entry.full_path == target_path {
            update_submodule_recursive(repo, trust_root, &entry, outputs, remote_url_policy)?;
            return Ok(true);
        }

        if target_path.starts_with(&entry.full_path)
            && let Some(nested_repo) = open_gitlink_repo(repo, &entry.relative_path)?
            && load_target_submodule_recursive(
                &nested_repo,
                trust_root,
                &entry.full_path,
                target_path,
                outputs,
                remote_url_policy,
            )?
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn configured_submodule_row(
    repo: &gix::Repository,
    submodule: gix::Submodule<'_>,
    full_path: PathBuf,
    gitlink: GitlinkIndexState,
) -> Result<(Submodule, Option<gix::Repository>)> {
    if gitlink.conflict {
        return Ok((
            Submodule {
                path: full_path,
                recorded_head: gitlink.null_head(repo),
                checked_out_head: None,
                status: SubmoduleStatus::MergeConflict,
            },
            None,
        ));
    }

    let nested_repo = open_configured_submodule_repo(&submodule)?;
    let Some(nested_repo) = nested_repo else {
        return Ok((
            Submodule {
                path: full_path,
                recorded_head: gitlink.index_head_or_null(repo),
                checked_out_head: None,
                status: SubmoduleStatus::NotInitialized,
            },
            None,
        ));
    };

    let checked_out_head_id = gix_head_id_or_none(&nested_repo)?;
    let status = if checked_out_head_id == gitlink.index_id {
        SubmoduleStatus::UpToDate
    } else {
        SubmoduleStatus::HeadMismatch
    };
    let head = checked_out_head_id
        .map(object_id_to_commit_id)
        .unwrap_or_else(|| gitlink.null_head(repo));

    Ok((
        Submodule {
            path: full_path,
            recorded_head: gitlink.index_head_or_null(repo),
            checked_out_head: Some(head),
            status,
        },
        Some(nested_repo),
    ))
}

fn resolve_submodule_target_commit_id(
    repo: &gix::Repository,
    reference: &str,
) -> Result<gix::ObjectId> {
    let object = crate::refs::resolve_required(repo, reference)
        .map_err(|error| {
            Error::new(ErrorKind::Backend(format!(
                "submodule reference '{}' did not resolve to an object",
                reference
            )))
            .with_source(error)
        })?
        .object()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "resolve submodule reference '{}': {e}",
                reference
            )))
        })?;
    let commit = object.peel_to_commit().map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "submodule reference '{}' does not point to a commit: {e}",
            reference
        )))
    })?;
    Ok(commit.id)
}

fn gitlink_commit_id_in_object(
    repo: &gix::Repository,
    object_id: gix::ObjectId,
    path: &Path,
) -> Result<Option<CommitId>> {
    let object = repo.find_object(object_id).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix find object {object_id}: {e}"
        )))
    })?;
    let tree = object.peel_to_tree().map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix peel tree for submodule '{}': {e}",
            path.display()
        )))
    })?;
    let Some(entry) = tree.lookup_entry_by_path(path).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix lookup submodule '{}': {e}",
            path.display()
        )))
    })?
    else {
        return Ok(None);
    };
    if !entry.mode().is_commit() {
        return Ok(None);
    }
    Ok(Some(object_id_to_commit_id(entry.object_id())))
}

fn head_gitlink_commit_id(repo: &gix::Repository, path: &Path) -> Result<Option<CommitId>> {
    let Some(head_id) = gix_head_id_or_none(repo)? else {
        return Ok(None);
    };
    gitlink_commit_id_in_object(repo, head_id, path)
}

fn resolve_submodule_logical_name(repo: &gix::Repository, path: &Path) -> Result<Option<PathBuf>> {
    let Some(submodules) = crate::refs::submodules(repo).map_err(|e| {
        Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))).with_source(e)
    })?
    else {
        return Ok(None);
    };

    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        if relative_path == path {
            let name = submodule.validated_name().map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "submodule '{}' has an unsafe name {:?}: {e}",
                    path.display(),
                    submodule.name()
                )))
            })?;
            let logical_name = pathbuf_from_gix_path(name)?;
            validate_submodule_git_dir_name(&logical_name)?;
            return Ok(Some(logical_name));
        }
    }

    Ok(None)
}

fn cleanup_failed_submodule_add_error(
    workdir: &Path,
    git_dir: &Path,
    path: &Path,
    logical_name: &Path,
    err: Error,
) -> Error {
    let clone_only_state = match failed_submodule_add_left_clone_only_state(workdir, path) {
        Ok(value) => value,
        Err(probe_err) => {
            return append_failed_submodule_add_note(
                err,
                &format!(
                    "{} could not inspect failed submodule add state: {probe_err}",
                    gitcomet_core::identity::current().display_name()
                ),
                probe_err,
                workdir,
            );
        }
    };

    if !clone_only_state {
        return err;
    }

    match cleanup_failed_submodule_add_leftovers(workdir, git_dir, path, logical_name) {
        Ok(()) => err,
        Err(cleanup_err) => append_failed_submodule_add_note(
            err,
            &format!("Cleanup after failed submodule add also failed: {cleanup_err}"),
            cleanup_err,
            workdir,
        ),
    }
}

fn failed_submodule_add_left_clone_only_state(workdir: &Path, path: &Path) -> Result<bool> {
    let repo = crate::open::open_worktree_repo(workdir).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "open repo after failed submodule add {}: {e}",
            workdir.display()
        )))
    })?;
    Ok(!submodule_path_registered(&repo, path)?)
}

fn submodule_path_registered(repo: &gix::Repository, path: &Path) -> Result<bool> {
    if configured_submodule_path_exists(repo, path)? {
        return Ok(true);
    }
    Ok(collect_gitlinks(repo, &CancellationToken::new())?.contains_key(path))
}

fn configured_submodule_path_exists(repo: &gix::Repository, path: &Path) -> Result<bool> {
    let Some(submodules) = crate::refs::submodules(repo).map_err(|e| {
        Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))).with_source(e)
    })?
    else {
        return Ok(false);
    };

    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        if relative_path == path {
            return Ok(true);
        }
    }

    Ok(false)
}

fn cleanup_failed_submodule_add_leftovers(
    workdir: &Path,
    git_dir: &Path,
    path: &Path,
    logical_name: &Path,
) -> Result<()> {
    remove_failed_submodule_checkout(workdir, git_dir, path, logical_name)?;
    cleanup_removed_submodule_metadata(workdir, git_dir, logical_name)
}

fn cleanup_removed_submodule_metadata(
    workdir: &Path,
    git_dir: &Path,
    logical_name: &Path,
) -> Result<()> {
    remove_local_submodule_config_section_if_present(workdir, logical_name)?;
    remove_submodule_git_dir(git_dir, logical_name)?;
    Ok(())
}

fn remove_failed_submodule_checkout(
    workdir: &Path,
    git_dir: &Path,
    submodule_path: &Path,
    logical_name: &Path,
) -> Result<()> {
    let checkout_path = submodule_worktree_path(workdir, submodule_path)?;
    if !checkout_path.exists() {
        return Ok(());
    }

    let expected_git_dir = canonicalize_or_original(git_dir.join("modules").join(logical_name));
    let Some(actual_git_dir) = checkout_git_dir_reference(&checkout_path)? else {
        return Err(Error::new(ErrorKind::Backend(format!(
            "refusing to remove failed submodule checkout {} because it is not linked to {}",
            checkout_path.display(),
            expected_git_dir.display()
        ))));
    };

    if actual_git_dir != expected_git_dir {
        return Err(Error::new(ErrorKind::Backend(format!(
            "refusing to remove failed submodule checkout {} because it points to {} instead of {}",
            checkout_path.display(),
            actual_git_dir.display(),
            expected_git_dir.display()
        ))));
    }

    fs::remove_dir_all(&checkout_path).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "remove failed submodule checkout {}: {e}",
            checkout_path.display()
        )))
        .with_source(e)
        .with_context("remove failed submodule checkout", Some(&checkout_path))
    })
}

fn submodule_worktree_path(workdir: &Path, submodule_path: &Path) -> Result<PathBuf> {
    if submodule_path.is_absolute() {
        if submodule_path.starts_with(workdir) {
            return Ok(submodule_path.to_path_buf());
        }
        return Err(Error::new(ErrorKind::Backend(format!(
            "refusing to clean failed submodule add outside repository workdir: {}",
            submodule_path.display()
        ))));
    }
    Ok(workdir.join(submodule_path))
}

fn checkout_git_dir_reference(checkout_path: &Path) -> Result<Option<PathBuf>> {
    let dot_git = checkout_path.join(".git");
    let metadata = match fs::metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(
                Error::from(err).with_context("inspect submodule Git directory", Some(&dot_git))
            );
        }
    };

    if metadata.is_dir() {
        return Ok(Some(canonicalize_or_original(dot_git)));
    }

    let bytes = fs::read(&dot_git).map_err(|error| {
        Error::from(error).with_context("read submodule Git directory", Some(&dot_git))
    })?;
    let text = bytes_to_text_preserving_utf8(&bytes);
    let Some(git_dir) = text.strip_prefix("gitdir:") else {
        return Ok(None);
    };

    let git_dir = PathBuf::from(git_dir.trim());
    let resolved = if git_dir.is_absolute() {
        git_dir
    } else {
        checkout_path.join(git_dir)
    };
    Ok(Some(canonicalize_or_original(resolved)))
}

#[derive(Debug)]
struct SubmoduleCleanupError {
    original: Error,
    cleanup: Error,
}

impl std::fmt::Display for SubmoduleCleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; {}", self.original, self.cleanup)
    }
}

impl std::error::Error for SubmoduleCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.original)
    }
}

fn append_failed_submodule_add_note(
    err: Error,
    note: &str,
    cleanup: Error,
    workdir: &Path,
) -> Error {
    let error = match err.kind() {
        ErrorKind::Git(failure) => {
            let mut detailed_failure = GitFailure::new(
                failure.command(),
                failure.id(),
                failure.exit_code(),
                failure.stdout().to_vec(),
                failure.stderr().to_vec(),
                Some(match failure.detail() {
                    Some(detail) if !detail.is_empty() => format!("{detail}\n\n{note}"),
                    _ => note.to_string(),
                }),
            );
            if let Some(auth_outcome) = failure.auth_outcome() {
                detailed_failure = detailed_failure.with_auth_outcome(auth_outcome.clone());
            }
            Error::new(ErrorKind::Git(detailed_failure))
        }
        _ => Error::new(ErrorKind::Backend(format!("{err}\n\n{note}"))),
    };
    error
        .with_source(SubmoduleCleanupError {
            original: err,
            cleanup,
        })
        .with_context("clean up failed submodule add", Some(workdir))
}

fn remove_local_submodule_config_section_if_present(
    workdir: &Path,
    logical_name: &Path,
) -> Result<()> {
    let Some(logical_name) = logical_name.to_str() else {
        return Err(Error::new(ErrorKind::Unsupported(
            "submodule logical name is not valid UTF-8",
        )));
    };
    // Freshly: `submodule deinit` usually removed the section just before.
    // Edited through gix: `git config --remove-section` cannot match a name
    // such as `[z]`, and its refusal is translated prose.
    let repo =
        crate::open::open_worktree_repo_with_options(workdir, gix::open::Options::isolated())
            .map_err(|e| crate::open::map_open_error(e, "gix open submodule config", workdir))?;
    super::porcelain::edit_local_config_strict(&repo, |config| {
        let mut removed = false;
        while config
            .remove_section("submodule", Some(logical_name.into()))
            .is_some()
        {
            removed = true;
        }
        Ok(removed)
    })
}

/// Refuse a submodule name that could leave `.git/modules/` once joined onto
/// it.
///
/// Names come from `.gitmodules` (`[submodule "<name>"]`, tracked content that
/// whoever published the repository controls) or from the user's own `--name`.
/// Git only forbids `..` components, so an absolute name passes its checks —
/// and `Path::join` adopts an absolute right-hand side wholesale, which would
/// aim the metadata removal below at an arbitrary directory.
fn validate_submodule_git_dir_name(logical_name: &Path) -> Result<()> {
    let mut has_normal_component = false;
    for component in logical_name.components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_submodule_name_error(logical_name));
            }
        }
    }
    if !has_normal_component {
        return Err(unsafe_submodule_name_error(logical_name));
    }
    Ok(())
}

fn unsafe_submodule_name_error(logical_name: &Path) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "refusing to use submodule name '{}': it must be a relative path without '..' components so it stays inside .git/modules",
        logical_name.display()
    )))
}

fn remove_submodule_git_dir(git_dir: &Path, logical_name: &Path) -> Result<()> {
    validate_submodule_git_dir_name(logical_name)?;
    let modules_root = git_dir.join("modules");
    let module_dir = modules_root.join(logical_name);
    if !module_dir.starts_with(&modules_root) {
        return Err(unsafe_submodule_name_error(logical_name));
    }
    if !module_dir.exists() {
        return Ok(());
    }

    fs::remove_dir_all(&module_dir).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "remove submodule git dir {}: {e}",
            module_dir.display()
        )))
        .with_source(e)
        .with_context("remove submodule Git directory", Some(&module_dir))
    })?;
    prune_empty_module_parent_dirs(&modules_root, &module_dir)
}

fn prune_empty_module_parent_dirs(modules_root: &Path, removed_dir: &Path) -> Result<()> {
    let mut current = removed_dir.parent();
    while let Some(dir) = current {
        if dir == modules_root || !dir.starts_with(modules_root) {
            break;
        }

        let mut entries = fs::read_dir(dir).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "read module metadata dir {}: {e}",
                dir.display()
            )))
            .with_source(e)
            .with_context("read module metadata directory", Some(dir))
        })?;
        match entries.next() {
            None => {
                fs::remove_dir(dir).map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "remove empty module metadata dir {}: {e}",
                        dir.display()
                    )))
                    .with_source(e)
                    .with_context("remove empty module metadata directory", Some(dir))
                })?;
                current = dir.parent();
            }
            Some(Ok(_)) => break,
            Some(Err(e)) => {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "read module metadata dir entry {}: {e}",
                    dir.display()
                )))
                .with_source(e)
                .with_context("read module metadata directory entry", Some(dir)));
            }
        }
    }
    Ok(())
}

fn collect_gitlinks(
    repo: &gix::Repository,
    cancellation: &CancellationToken,
) -> Result<BTreeMap<PathBuf, GitlinkIndexState>> {
    cancellation.check_cancelled()?;
    use gix::worktree::IndexPersistedOrInMemory as Index;
    let index = match crate::refs::index_or_load_from_head_or_empty(repo)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))).with_source(e))?
    {
        // A held repository's snapshot can lag a rewrite within one tick.
        Index::Persisted(index) => Index::Persisted(super::status::index_as_on_disk(repo, index)?),
        from_head => from_head,
    };
    let path_backing = index.path_backing();

    let mut gitlinks: BTreeMap<PathBuf, GitlinkIndexState> = BTreeMap::new();
    for entry in index.entries() {
        cancellation.check_cancelled()?;
        if entry.mode != gix::index::entry::Mode::COMMIT {
            continue;
        }

        let path = pathbuf_from_gix_path(entry.path_in(path_backing))?;
        gitlinks
            .entry(path)
            .or_default()
            .record(entry.id, entry.stage());
    }

    Ok(gitlinks)
}

fn open_gitlink_repo(
    repo: &gix::Repository,
    relative_path: &Path,
) -> Result<Option<gix::Repository>> {
    let Some(workdir) = repo.workdir() else {
        return Ok(None);
    };
    let path = workdir.join(relative_path);

    match crate::open::open_worktree_repo(&path) {
        Ok(repo) => Ok(Some(repo)),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(crate::open::map_open_error(
            e,
            "gix open nested submodule repo",
            &path,
        )),
    }
}

fn open_configured_submodule_repo(
    submodule: &gix::Submodule<'_>,
) -> Result<Option<gix::Repository>> {
    let state = crate::refs::submodule_state(submodule).map_err(|e| {
        Error::new(ErrorKind::Backend(format!("gix submodule state: {e}"))).with_source(e)
    })?;
    if !(state.repository_exists && state.worktree_checkout) {
        return Ok(None);
    }
    // gix's own `Submodule::open` opens the submodule's git directory directly
    // (`modules/<name>`, following the worktree `.git` pointer) and sets the
    // workdir by hand, so it already handles submodule worktrees whose directory
    // ends in `.git` — no `git_dir_for_workdir` shim is needed here.
    submodule
        .open()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule open: {e}"))))
}

fn trust_target_from_submodule(
    current_repo_workdir: &Path,
    full_submodule_path: &Path,
    submodule: &gix::Submodule<'_>,
    remote_url_policy: RemoteUrlPolicy,
) -> Result<Option<SubmoduleTrustTarget>> {
    let url = submodule
        .url()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule url: {e}"))))?;
    // .gitmodules ships with the repository, so hold its URL to the same
    // allowlist as the Add dialog rather than to Git's protocol policy alone.
    validate_remote_url_with_policy(
        &bytes_to_text_preserving_utf8(url.to_bstring().as_ref()),
        remote_url_policy,
    )?;
    trust_target_from_url(current_repo_workdir, full_submodule_path, &url)
}

fn trust_target_from_raw_source(
    current_repo_workdir: &Path,
    submodule_path: &Path,
    raw_source: &str,
) -> Result<Option<SubmoduleTrustTarget>> {
    let url = gix::url::parse(raw_source.as_bytes().as_bstr()).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "invalid submodule source {raw_source:?}: {e}"
        )))
    })?;
    let display_source = raw_source.trim().to_string();
    trust_target_from_parsed_url(current_repo_workdir, submodule_path, &url, display_source)
}

fn trust_target_from_url(
    current_repo_workdir: &Path,
    submodule_path: &Path,
    url: &gix::Url,
) -> Result<Option<SubmoduleTrustTarget>> {
    let display_source = bytes_to_text_preserving_utf8(url.to_bstring().as_ref());
    trust_target_from_parsed_url(current_repo_workdir, submodule_path, url, display_source)
}

fn trust_target_from_parsed_url(
    current_repo_workdir: &Path,
    submodule_path: &Path,
    url: &gix::Url,
    display_source: String,
) -> Result<Option<SubmoduleTrustTarget>> {
    if url.scheme != gix::url::Scheme::File {
        return Ok(None);
    }

    let local_source_path = canonicalize_or_original(resolve_local_file_transport_path(
        current_repo_workdir,
        url,
    )?);
    Ok(Some(SubmoduleTrustTarget {
        submodule_path: submodule_path.to_path_buf(),
        display_source,
        local_source_path,
    }))
}

fn resolve_local_file_transport_path(
    current_repo_workdir: &Path,
    url: &gix::Url,
) -> Result<PathBuf> {
    let mut path = pathbuf_from_gix_path(url.path.as_ref())?;
    if let Some(host) = url.host.as_deref()
        && !host.eq_ignore_ascii_case("localhost")
    {
        let host_path = PathBuf::from(format!("//{host}")).join(&path);
        path = host_path;
    }
    if path.is_relative() {
        path = current_repo_workdir.join(path);
    }
    Ok(path)
}

fn persist_submodule_trust_approvals(
    trust_root: &Path,
    approved_sources: &[SubmoduleTrustTarget],
) -> Result<()> {
    for source in approved_sources {
        let key = submodule_file_transport_consent_key(trust_root, &source.local_source_path);
        if git_config_get_bool_global_with_retry(trust_root, &key)?.unwrap_or(false) {
            continue;
        }

        if let Err(write_err) = git_config_set_bool_global_with_retry(trust_root, &key) {
            if git_config_get_bool_global_with_retry(trust_root, &key)?.unwrap_or(false) {
                continue;
            }
            return Err(write_err);
        }
    }
    Ok(())
}

fn git_config_get_bool_global_with_retry(trust_root: &Path, key: &str) -> Result<Option<bool>> {
    retry_git_config_contention(|| git_config_get_bool_global(trust_root, key))
}

fn git_config_set_bool_global_with_retry(trust_root: &Path, key: &str) -> Result<()> {
    retry_git_config_contention(|| {
        let mut cmd = git_workdir_cmd_for(trust_root);
        // Contention is recognized by Git's English text.
        cmd.env("LC_ALL", "C")
            .arg("config")
            .arg("--global")
            .arg(key)
            .arg("true");
        run_git_simple(cmd, &format!("git config --global {key} true"))
    })
}

fn retry_git_config_contention<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
    for attempt in 0..GIT_CONFIG_CONTENTION_RETRIES {
        match operation() {
            Ok(value) => return Ok(value),
            Err(err) => {
                if attempt + 1 == GIT_CONFIG_CONTENTION_RETRIES
                    || !is_git_config_contention_error(&err)
                {
                    return Err(err);
                }
                thread::sleep(GIT_CONFIG_CONTENTION_RETRY_DELAY);
            }
        }
    }
    unreachable!("the retry loop always returns after the final attempt");
}

fn is_git_config_contention_error(err: &Error) -> bool {
    let text = match err.kind() {
        ErrorKind::Git(failure) => format!(
            "{}{}{}",
            failure.detail().unwrap_or_default(),
            String::from_utf8_lossy(failure.stderr()),
            String::from_utf8_lossy(failure.stdout())
        ),
        _ => err.to_string(),
    };
    let text = text.to_ascii_lowercase();
    text.contains("could not lock config file")
        || (text.contains("unable to access")
            && text.contains("permission denied")
            && text.contains("reading the configuration files"))
}

fn submodule_source_trusted(trust_root: &Path, source: &SubmoduleTrustTarget) -> Result<bool> {
    let key = submodule_file_transport_consent_key(trust_root, &source.local_source_path);
    Ok(git_config_get_bool_global_with_retry(trust_root, &key)?.unwrap_or(false))
}

fn untrusted_local_submodule_error(source: &SubmoduleTrustTarget, action: &str) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "Refusing to {action} local submodule '{}' from '{}'. Explicit trust is required before enabling file transport.",
        source.submodule_path.display(),
        source.display_source
    )))
}

fn combine_submodule_update_outputs(outputs: Vec<CommandOutput>) -> CommandOutput {
    CommandOutput::combine("git submodule update --init --recursive", &outputs)
}

fn submodule_repo_workdir(repo: &gix::Repository) -> &Path {
    repo.workdir().unwrap_or_else(|| repo.git_dir())
}

fn submodule_file_transport_consent_key(trust_root: &Path, source_path: &Path) -> String {
    let root = canonicalize_or_original(trust_root.to_path_buf());
    let source = canonicalize_or_original(source_path.to_path_buf());

    let mut bytes = stable_path_bytes(&root);
    bytes.push(0);
    bytes.extend_from_slice(&stable_path_bytes(&source));
    format!(
        "gitcomet.submodule.allowfiletransport-{:016x}",
        fnv1a_64(&bytes)
    )
}

fn git_config_get_bool_global(trust_root: &Path, key: &str) -> Result<Option<bool>> {
    let mut cmd = git_workdir_cmd_for(trust_root);
    // Contention is recognized by Git's English text.
    cmd.env("LC_ALL", "C")
        .arg("config")
        .arg("--global")
        .arg("--type=bool")
        .arg("--get")
        .arg(key);

    let output = cmd.output().map_err(Error::from)?;

    if output.status.success() {
        let value = bytes_to_text_preserving_utf8(&output.stdout);
        return match value.trim() {
            "true" => Ok(Some(true)),
            "false" => Ok(Some(false)),
            other => Err(Error::new(ErrorKind::Backend(format!(
                "Invalid boolean value for git config {key}: {:?}. Expected true or false.",
                other
            )))),
        };
    }

    if output.status.code() == Some(1) {
        return Ok(None);
    }

    Err(Error::new(ErrorKind::Backend(format!(
        "git config --global --type=bool --get {key} failed: {}",
        bytes_to_text_preserving_utf8(&output.stderr).trim()
    ))))
}

fn pathbuf_from_gix_path(path: &gix::bstr::BStr) -> Result<PathBuf> {
    gix::path::from_bstr(path)
        .map(|path| path.into_owned())
        .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))
}

fn object_id_to_commit_id(id: gix::ObjectId) -> CommitId {
    CommitId(oid_to_arc_str(&id))
}

/// Submodules of an already-open repository, sorted by path.
/// Every submodule of `repo` and below, by path. `gitlinks` are those of its
/// index. With enough of them, `workers` threads share the submodules out:
/// each opens a repository and lists it in turn, and none needs another's.
fn list_submodules_in(
    repo: &gix::Repository,
    gitlinks: BTreeMap<PathBuf, GitlinkIndexState>,
    workers: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<Submodule>> {
    let configured = configured_submodules(repo)?;
    // Every gitlink makes one task.
    let workers = workers.min(gitlinks.len() / SUBMODULE_LISTERS_MIN_SUBMODULES);
    let again = (workers >= 2).then(|| gitlinks.clone());
    let tasks = submodule_tasks(&configured, gitlinks, cancellation)?;
    let mut submodules = Vec::new();
    if workers < 2 {
        for task in &tasks {
            submodules.extend(list_submodule(
                repo,
                &configured,
                Path::new(""),
                task,
                cancellation,
            )?);
        }
    } else {
        // A repository handle and what borrows it stay with one thread, so
        // each reads `.gitmodules` again; one that finds other mappings than
        // the tasks name lists nothing.
        let mapping = submodule_mapping(&configured);
        drop(configured);
        #[cfg(test)]
        BEFORE_LISTERS.take().into_iter().for_each(|hook| hook());
        let next = std::sync::atomic::AtomicUsize::new(0);
        type Listed = Vec<(usize, Result<Vec<Submodule>>)>;
        let list = |repo: gix::Repository| -> Result<Option<Listed>> {
            let configured = configured_submodules(&repo)?;
            if submodule_mapping(&configured) != mapping {
                return Ok(None);
            }
            let mut listed = Vec::new();
            while let Some((at, task)) =
                Some(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                    .and_then(|at| Some((at, tasks.get(at)?)))
            {
                let rows = list_submodule(&repo, &configured, Path::new(""), task, cancellation);
                listed.push((at, rows));
            }
            Ok(Some(listed))
        };
        let listed = std::thread::scope(|scope| {
            let helpers: Vec<_> = (1..workers)
                .map(|_| {
                    let repo = repo.clone();
                    scope.spawn(|| list(repo))
                })
                .collect();
            let mut listed = vec![list(repo.clone())];
            for helper in helpers {
                listed.push(
                    helper
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                );
            }
            listed
        });
        if let Some(gitlinks) = again.filter(|_| listed.iter().any(|l| matches!(l, Ok(None)))) {
            // `.gitmodules` changed while the threads started: one reads it.
            return list_submodules_in(repo, gitlinks, 1, cancellation);
        }
        // In the tasks' order, so that the first failure is the one a single
        // thread would have stopped at.
        let mut by_task: Vec<Option<Result<Vec<Submodule>>>> = tasks.iter().map(|_| None).collect();
        for listed in listed {
            for (at, rows) in listed?.into_iter().flatten() {
                by_task[at] = Some(rows);
            }
        }
        for rows in by_task.into_iter().flatten() {
            submodules.extend(rows?);
        }
    }
    cancellation.check_cancelled()?;
    submodules.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    Ok(submodules)
}

#[cfg(test)]
mod tests;
