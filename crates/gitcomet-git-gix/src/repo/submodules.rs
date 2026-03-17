use super::GixRepo;
use super::history::gix_head_id_or_none;
use crate::util::run_git_with_output;
use gitcomet_core::domain::{CommitId, Submodule, SubmoduleStatus};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
impl GixRepo {
    pub(super) fn list_submodules_impl(&self) -> Result<Vec<Submodule>> {
        let repo = self.reopen_repo()?;
        let mut submodules = Vec::new();
        collect_repo_submodules(&repo, Path::new(""), &mut submodules)?;
        submodules.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(submodules)
    }

    pub(super) fn add_submodule_with_output_impl(
        &self,
        url: &str,
        path: &Path,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("submodule")
            .arg("add")
            .arg(url)
            .arg(path)
            // Local-path submodule URLs are used in tests and supported workflows.
            // Explicitly allow `file` transport for this command.
            .env("GIT_ALLOW_PROTOCOL", "file");
        run_git_with_output(cmd, &format!("git submodule add {url} {}", path.display()))
    }

    pub(super) fn update_submodules_with_output_impl(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("submodule")
            .arg("update")
            .arg("--init")
            .arg("--recursive")
            // Keep behavior consistent with add: allow local-path submodule URLs.
            .env("GIT_ALLOW_PROTOCOL", "file");
        run_git_with_output(cmd, "git submodule update --init --recursive")
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

    let state = submodule
        .state()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule state: {e}"))))?;
    let nested_repo = if state.repository_exists && state.worktree_checkout {
        submodule
            .open()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix submodule open: {e}"))))?
    } else {
        None
    };
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

fn pathbuf_from_gix_path(path: &gix::bstr::BStr) -> Result<PathBuf> {
    gix::path::try_from_bstr(path)
        .map(|path| path.into_owned())
        .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))
}

fn object_id_to_commit_id(id: gix::ObjectId) -> CommitId {
    CommitId(id.to_string().into())
}
