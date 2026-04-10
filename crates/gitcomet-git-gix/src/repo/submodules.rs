use super::GixRepo;
use super::history::gix_head_id_or_none;
use crate::util::{
    bytes_to_text_preserving_utf8, git_workdir_cmd_for, run_git_simple, run_git_with_output,
};
use gitcomet_core::domain::{CommitId, Submodule, SubmoduleStatus};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{
    CommandOutput, Result, SubmoduleTrustDecision, SubmoduleTrustTarget,
};
use gix::bstr::ByteSlice as _;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn allow_file_submodule_transport(cmd: &mut Command) {
    // `git submodule` blocks local-path remotes unless `protocol.file.allow` is enabled.
    // Use per-command config so local workflows keep working without disabling `https`/`ssh`.
    cmd.arg("-c").arg("protocol.file.allow=always");
}

impl GixRepo {
    pub(super) fn list_submodules_impl(&self) -> Result<Vec<Submodule>> {
        let repo = self.reopen_repo()?;
        let mut submodules = Vec::new();
        collect_repo_submodules(&repo, Path::new(""), &mut submodules)?;
        submodules.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(submodules)
    }

    pub(super) fn check_submodule_add_trust_impl(
        &self,
        url: &str,
        path: &Path,
    ) -> Result<SubmoduleTrustDecision> {
        let repo = self.reopen_repo()?;
        let Some(target) =
            trust_target_from_raw_source(repo_workdir_for_submodule_trust(&repo), path, url)?
        else {
            return Ok(SubmoduleTrustDecision::Proceed);
        };

        if submodule_source_trusted(repo_workdir_for_submodule_trust(&repo), &target)? {
            Ok(SubmoduleTrustDecision::Proceed)
        } else {
            Ok(SubmoduleTrustDecision::Prompt {
                sources: vec![target],
            })
        }
    }

    pub(super) fn check_submodule_update_trust_impl(&self) -> Result<SubmoduleTrustDecision> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo);
        let mut sources = BTreeMap::new();
        collect_repo_untrusted_submodule_sources(&repo, trust_root, Path::new(""), &mut sources)?;
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
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo);
        persist_submodule_trust_approvals(trust_root, approved_sources)?;

        let mut cmd = self.git_workdir_cmd();
        if let Some(target) = trust_target_from_raw_source(trust_root, path, url)? {
            if !submodule_source_trusted(trust_root, &target)? {
                return Err(untrusted_local_submodule_error(&target, "add"));
            }
            allow_file_submodule_transport(&mut cmd);
        }

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
        cmd.arg(url).arg(path);
        command.push_str(&format!(" {url} {}", path.display()));
        run_git_with_output(cmd, &command)
    }

    pub(super) fn update_submodules_with_output_impl(
        &self,
        approved_sources: &[SubmoduleTrustTarget],
    ) -> Result<CommandOutput> {
        let repo = self.reopen_repo()?;
        let trust_root = repo_workdir_for_submodule_trust(&repo).to_path_buf();
        persist_submodule_trust_approvals(&trust_root, approved_sources)?;

        let mut outputs = Vec::new();
        update_repo_submodules_recursive(
            &repo,
            &trust_root,
            Path::new(""),
            approved_sources,
            &mut outputs,
        )?;

        if outputs.is_empty() {
            Ok(CommandOutput::empty_success(
                "git submodule update --init --recursive",
            ))
        } else {
            Ok(combine_submodule_update_outputs(outputs))
        }
    }

    pub(super) fn remove_submodule_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut cmd1 = self.git_workdir_cmd();
        cmd1.arg("submodule")
            .arg("deinit")
            .arg("-f")
            .arg("--")
            .arg(path);
        let out1 =
            run_git_with_output(cmd1, &format!("git submodule deinit -f {}", path.display()))?;

        let mut cmd2 = self.git_workdir_cmd();
        cmd2.arg("rm").arg("-f").arg("--").arg(path);
        let out2 = run_git_with_output(cmd2, &format!("git rm -f {}", path.display()))?;

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

fn collect_repo_submodules(
    repo: &gix::Repository,
    prefix: &Path,
    out: &mut Vec<Submodule>,
) -> Result<()> {
    let mut gitlinks = collect_gitlinks(repo)?;
    if let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    {
        for submodule in submodules {
            let relative_path = submodule
                .path()
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
                .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
            let Some(gitlink) = gitlinks.remove(&relative_path) else {
                continue;
            };

            let full_path = prefix.join(&relative_path);
            let (row, nested_repo) =
                configured_submodule_row(repo, submodule, full_path.clone(), gitlink)?;
            out.push(row);
            if let Some(nested_repo) = nested_repo {
                collect_repo_submodules(&nested_repo, &full_path, out)?;
            }
        }
    }

    for (relative_path, gitlink) in gitlinks {
        let full_path = prefix.join(&relative_path);
        out.push(Submodule {
            path: full_path.clone(),
            head: gitlink.index_head_or_null(repo),
            status: SubmoduleStatus::MissingMapping,
        });
        if let Some(nested_repo) = open_gitlink_repo(repo, &relative_path)? {
            collect_repo_submodules(&nested_repo, &full_path, out)?;
        }
    }

    Ok(())
}

fn collect_repo_untrusted_submodule_sources(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    out: &mut BTreeMap<PathBuf, SubmoduleTrustTarget>,
) -> Result<()> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(());
    };

    let current_workdir = repo_workdir_for_submodule_trust(repo);
    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        let full_path = prefix.join(&relative_path);

        if let Some(target) = trust_target_from_submodule(current_workdir, &full_path, &submodule)?
        {
            if !submodule_source_trusted(trust_root, &target)? {
                out.insert(full_path.clone(), target);
            }
        }

        if let Some(nested_repo) = open_configured_submodule_repo(&submodule)? {
            collect_repo_untrusted_submodule_sources(&nested_repo, trust_root, &full_path, out)?;
        }
    }

    Ok(())
}

fn update_repo_submodules_recursive(
    repo: &gix::Repository,
    trust_root: &Path,
    prefix: &Path,
    approved_sources: &[SubmoduleTrustTarget],
    outputs: &mut Vec<CommandOutput>,
) -> Result<()> {
    let Some(submodules) = repo
        .submodules()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodules: {e}"))))?
    else {
        return Ok(());
    };

    let current_workdir = repo_workdir_for_submodule_trust(repo);
    for submodule in submodules {
        let relative_path = submodule
            .path()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule path: {e}"))))
            .and_then(|path| pathbuf_from_gix_path(path.as_ref()))?;
        let full_path = prefix.join(&relative_path);

        let local_target = trust_target_from_submodule(current_workdir, &full_path, &submodule)?;

        let mut cmd = git_workdir_cmd_for(current_workdir);
        if let Some(target) = local_target.as_ref() {
            if !submodule_source_trusted(trust_root, target)? {
                return Err(untrusted_local_submodule_error(target, "update"));
            }
            allow_file_submodule_transport(&mut cmd);
        }

        cmd.arg("submodule")
            .arg("update")
            .arg("--init")
            .arg("--")
            .arg(&relative_path);
        outputs.push(run_git_with_output(
            cmd,
            &format!("git submodule update --init -- {}", full_path.display()),
        )?);

        if let Some(nested_repo) = open_gitlink_repo(repo, &relative_path)? {
            update_repo_submodules_recursive(
                &nested_repo,
                trust_root,
                &full_path,
                approved_sources,
                outputs,
            )?;
        }
    }

    Ok(())
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
                head: gitlink.null_head(repo),
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
                head: gitlink.index_head_or_null(repo),
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
            head,
            status,
        },
        Some(nested_repo),
    ))
}

fn collect_gitlinks(repo: &gix::Repository) -> Result<BTreeMap<PathBuf, GitlinkIndexState>> {
    let index = repo
        .index_or_load_from_head_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let path_backing = index.path_backing();

    let mut gitlinks: BTreeMap<PathBuf, GitlinkIndexState> = BTreeMap::new();
    for entry in index.entries() {
        if entry.mode != gix::index::entry::Mode::COMMIT {
            continue;
        }

        let path = pathbuf_from_gix_path(entry.path_in(path_backing))?;
        let state = gitlinks.entry(path).or_default();
        state.kind.get_or_insert(entry.id.kind());
        if entry.stage() == gix::index::entry::Stage::Unconflicted {
            state.index_id = Some(entry.id);
        } else {
            state.conflict = true;
        }
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

    match gix::open(&path) {
        Ok(repo) => Ok(Some(repo)),
        Err(gix::open::Error::NotARepository { .. }) => Ok(None),
        Err(gix::open::Error::Io(io)) if io.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(gix::open::Error::Io(io)) => Err(Error::new(ErrorKind::Io(io.kind()))),
        Err(e) => Err(Error::new(ErrorKind::Backend(format!(
            "gix open nested submodule repo {}: {e}",
            path.display()
        )))),
    }
}

fn open_configured_submodule_repo(
    submodule: &gix::Submodule<'_>,
) -> Result<Option<gix::Repository>> {
    let state = submodule
        .state()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule state: {e}"))))?;
    if !(state.repository_exists && state.worktree_checkout) {
        return Ok(None);
    }
    submodule
        .open()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule open: {e}"))))
}

fn trust_target_from_submodule(
    current_repo_workdir: &Path,
    full_submodule_path: &Path,
    submodule: &gix::Submodule<'_>,
) -> Result<Option<SubmoduleTrustTarget>> {
    let url = submodule
        .url()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule url: {e}"))))?;
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
    if let Some(host) = url.host.as_deref() {
        if !host.eq_ignore_ascii_case("localhost") {
            let host_path = PathBuf::from(format!("//{host}")).join(&path);
            path = host_path;
        }
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
        if git_config_get_bool_global(trust_root, &key)?.unwrap_or(false) {
            continue;
        }

        let mut cmd = git_workdir_cmd_for(trust_root);
        cmd.arg("config").arg("--global").arg(&key).arg("true");
        run_git_simple(cmd, &format!("git config --global {key} true"))?;
    }
    Ok(())
}

fn submodule_source_trusted(trust_root: &Path, source: &SubmoduleTrustTarget) -> Result<bool> {
    let key = submodule_file_transport_consent_key(trust_root, &source.local_source_path);
    Ok(git_config_get_bool_global(trust_root, &key)?.unwrap_or(false))
}

fn untrusted_local_submodule_error(source: &SubmoduleTrustTarget, action: &str) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "Refusing to {action} local submodule '{}' from '{}'. Explicit trust is required before enabling file transport.",
        source.submodule_path.display(),
        source.display_source
    )))
}

fn combine_submodule_update_outputs(outputs: Vec<CommandOutput>) -> CommandOutput {
    CommandOutput {
        command: "git submodule update --init --recursive".to_string(),
        stdout: outputs
            .iter()
            .map(|output| output.stdout.trim_end())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        stderr: outputs
            .iter()
            .map(|output| output.stderr.trim_end())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        exit_code: Some(0),
    }
}

fn repo_workdir_for_submodule_trust(repo: &gix::Repository) -> &Path {
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
    cmd.arg("config")
        .arg("--global")
        .arg("--type=bool")
        .arg("--get")
        .arg(key);

    let output = cmd
        .output()
        .map_err(|err| Error::new(ErrorKind::Io(err.kind())))?;

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

fn stable_path_bytes(path: &Path) -> Vec<u8> {
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

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn pathbuf_from_gix_path(path: &gix::bstr::BStr) -> Result<PathBuf> {
    gix::path::try_from_bstr(path)
        .map(|path| path.into_owned())
        .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))
}

fn object_id_to_commit_id(id: gix::ObjectId) -> CommitId {
    CommitId(id.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::allow_file_submodule_transport;
    use super::submodule_file_transport_consent_key;
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn allow_file_submodule_transport_uses_git_config_not_protocol_allowlist() {
        let mut cmd = Command::new("git");

        allow_file_submodule_transport(&mut cmd);

        let args = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args, ["-c", "protocol.file.allow=always"]);
        assert!(
            !cmd.get_envs()
                .any(|(key, _)| key == OsStr::new("GIT_ALLOW_PROTOCOL"))
        );
    }

    #[test]
    fn consent_key_depends_on_root_and_source_path() {
        let a = submodule_file_transport_consent_key(
            Path::new("/repo-a"),
            Path::new("/sources/local-one"),
        );
        let b = submodule_file_transport_consent_key(
            Path::new("/repo-a"),
            Path::new("/sources/local-two"),
        );
        let c = submodule_file_transport_consent_key(
            Path::new("/repo-b"),
            Path::new("/sources/local-one"),
        );

        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
