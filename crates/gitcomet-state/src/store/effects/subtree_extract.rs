use crate::model::{RepoId, SubtreeExtractProgressMeter, SubtreeExtractProgressStage};
use crate::msg::Msg;
use gitcomet_core::auth::{
    GITCOMET_AUTH_KIND_ENV, GITCOMET_AUTH_KIND_HOST_VERIFICATION, GITCOMET_AUTH_KIND_PASSPHRASE,
    GITCOMET_AUTH_KIND_USERNAME_PASSWORD, GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV,
    GitAuthKind, StagedGitAuth, clear_staged_git_auth, stage_git_auth_for_current_thread,
};
use gitcomet_core::domain::{SubtreeExtractOptions, SubtreeSourceConfig};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::configure_background_command;
use gitcomet_core::services::CommandOutput;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, mpsc};

use super::super::executor::TaskExecutor;
use super::util::{RepoMap, send_or_log, spawn_with_repo};

struct AskPassScript {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

#[cfg(unix)]
fn askpass_script_contents() -> &'static [u8] {
    br#"#!/bin/sh
prompt="$1"
lower_prompt=$(printf '%s' "$prompt" | tr '[:upper:]' '[:lower:]')
kind="${GITCOMET_AUTH_KIND:-}"
if [ "$kind" = "username_password" ]; then
  case "$lower_prompt" in
    *username*) printf '%s\n' "${GITCOMET_AUTH_USERNAME:-}" ;;
    *) printf '%s\n' "${GITCOMET_AUTH_SECRET:-}" ;;
  esac
elif [ "$kind" = "host_verification" ]; then
  case "$lower_prompt" in
    *continue\ connecting*|*yes/no*|*fingerprint*) printf '%s\n' "${GITCOMET_AUTH_SECRET:-}" ;;
    *) printf '\n' ;;
  esac
else
  printf '%s\n' "${GITCOMET_AUTH_SECRET:-}"
fi
"#
}

#[cfg(windows)]
fn askpass_script_contents() -> &'static [u8] {
    br#"@echo off
setlocal EnableDelayedExpansion
set "prompt=%~1"
if /I "%GITCOMET_AUTH_KIND%"=="username_password" (
  echo %prompt% | findstr /I "username" >nul
  if not errorlevel 1 (
    echo %GITCOMET_AUTH_USERNAME%
    exit /b 0
  )
  echo %GITCOMET_AUTH_SECRET%
  exit /b 0
)
if /I "%GITCOMET_AUTH_KIND%"=="host_verification" (
  echo %prompt% | findstr /I /C:"continue connecting" /C:"yes/no" /C:"fingerprint" >nul
  if not errorlevel 1 (
    echo %GITCOMET_AUTH_SECRET%
  )
  exit /b 0
)
echo %GITCOMET_AUTH_SECRET%
"#
}

fn io_error(err: std::io::Error) -> Error {
    Error::new(ErrorKind::Io(err.kind()))
}

fn create_askpass_script() -> Result<AskPassScript, Error> {
    let dir = tempfile::tempdir().map_err(io_error)?;
    #[cfg(windows)]
    let script_name = "gitcomet-askpass.cmd";
    #[cfg(not(windows))]
    let script_name = "gitcomet-askpass.sh";
    let path = dir.path().join(script_name);
    fs::write(&path, askpass_script_contents()).map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mut permissions = fs::metadata(&path).map_err(io_error)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).map_err(io_error)?;
    }
    Ok(AskPassScript { _dir: dir, path })
}

fn configure_auth_prompt(cmd: &mut Command, auth: &StagedGitAuth, askpass: &AskPassScript) {
    cmd.env("GIT_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS_REQUIRE", "force");
    if cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("DISPLAY").is_none() {
        cmd.env("DISPLAY", "gitcomet:0");
    }

    let kind = match auth.kind {
        GitAuthKind::UsernamePassword => GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
        GitAuthKind::Passphrase => GITCOMET_AUTH_KIND_PASSPHRASE,
        GitAuthKind::HostVerification => GITCOMET_AUTH_KIND_HOST_VERIFICATION,
    };
    cmd.env(GITCOMET_AUTH_KIND_ENV, kind);
    if let Some(username) = auth.username.as_deref() {
        cmd.env(GITCOMET_AUTH_USERNAME_ENV, username);
    } else {
        cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
    }
    cmd.env(GITCOMET_AUTH_SECRET_ENV, &auth.secret);
}

fn git_command(workdir: &Path) -> Command {
    let mut cmd = Command::new("git");
    configure_background_command(&mut cmd);
    cmd.arg("-C")
        .arg(workdir)
        .arg("-c")
        .arg("color.ui=false")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn command_output(command: String, output: Output) -> CommandOutput {
    CommandOutput {
        command,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
    }
}

fn run_git_output(
    mut cmd: Command,
    command: String,
    auth: Option<&StagedGitAuth>,
) -> Result<CommandOutput, Error> {
    let askpass = if auth.is_some() {
        Some(create_askpass_script()?)
    } else {
        None
    };
    if let (Some(auth), Some(askpass)) = (auth, askpass.as_ref()) {
        configure_auth_prompt(&mut cmd, auth, askpass);
    }
    let output = cmd.output().map_err(io_error)?;
    Ok(command_output(command, output))
}

fn run_git(
    cmd: Command,
    command: String,
    auth: Option<&StagedGitAuth>,
) -> Result<CommandOutput, Error> {
    let output = run_git_output(cmd, command.clone(), auth)?;
    if output.exit_code == Some(0) {
        Ok(output)
    } else {
        let details = if output.stderr.trim().is_empty() {
            output.stdout.trim()
        } else {
            output.stderr.trim()
        };
        Err(Error::new(ErrorKind::Backend(if details.is_empty() {
            format!("{command} failed")
        } else {
            format!("{command} failed:\n{details}")
        })))
    }
}

fn parse_split_commit(output: &CommandOutput) -> Result<String, Error> {
    if let Some(revision) = output
        .stdout
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
    {
        return Ok(revision.to_string());
    }

    let combined = output.combined();
    combined
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "git subtree split produced no revision".to_string(),
            ))
        })
}

fn push_output_lines(
    msg_tx: &mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: &Arc<PathBuf>,
    destination_repo: Option<&Arc<PathBuf>>,
    progress: SubtreeExtractProgressMeter,
    output: &CommandOutput,
) {
    for line in output
        .combined()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        send_progress(
            msg_tx,
            repo_id,
            Arc::clone(path),
            destination_repo.cloned(),
            progress,
            Some(line.to_string()),
        );
    }
}

fn send_progress(
    msg_tx: &mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: Arc<PathBuf>,
    destination_repo: Option<Arc<PathBuf>>,
    progress: SubtreeExtractProgressMeter,
    line: Option<String>,
) {
    send_or_log(
        msg_tx,
        Msg::Internal(crate::msg::InternalMsg::ExtractSubtreeProgress {
            repo_id,
            path,
            destination_repo,
            progress,
            line,
        }),
    );
}

fn default_destination_branch(
    options: &SubtreeExtractOptions,
    existing_source: Option<&SubtreeSourceConfig>,
) -> String {
    options
        .destination_branch
        .clone()
        .or_else(|| options.split.branch.clone())
        .or_else(|| existing_source.map(|source| source.reference.clone()))
        .unwrap_or_else(|| "main".to_string())
}

fn normalize_subtree_split_path(path: &Path) -> Result<PathBuf, Error> {
    const OUTSIDE_WORKDIR_ERROR: &str = "subtree path must stay inside the repository workdir";
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::new(ErrorKind::Backend(
                    OUTSIDE_WORKDIR_ERROR.to_string(),
                )));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(Error::new(ErrorKind::Backend(
            "subtree path must not be empty".to_string(),
        )));
    }
    Ok(normalized)
}

fn validate_destination_repository(
    source_repo: &Path,
    destination_repo: &Path,
) -> Result<(), Error> {
    if destination_repo == source_repo {
        return Err(Error::new(ErrorKind::Backend(format!(
            "destination repository {} matches the source repository; choose a different folder",
            destination_repo.display()
        ))));
    }

    let Ok(metadata) = fs::metadata(destination_repo) else {
        return Ok(());
    };

    if !metadata.is_dir() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "destination path {} already exists and is not a directory",
            destination_repo.display()
        ))));
    }

    let mut entries = fs::read_dir(destination_repo).map_err(io_error)?;
    if entries.next().transpose().map_err(io_error)?.is_some() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "destination directory {} already exists and is not empty",
            destination_repo.display()
        ))));
    }

    Ok(())
}

fn store_updated_subtree_source(
    repo: &Arc<dyn gitcomet_core::services::GitRepository>,
    subtree_path: &Path,
    destination_repo: &Path,
    destination_branch: &str,
    remote_repository: Option<&str>,
    existing_source: Option<&SubtreeSourceConfig>,
) -> Result<(), Error> {
    let local_repository = destination_repo.display().to_string();
    repo.store_subtree_source_config(
        subtree_path,
        &SubtreeSourceConfig {
            local_repository: Some(local_repository.clone()),
            repository: remote_repository.unwrap_or(&local_repository).to_string(),
            reference: destination_branch.to_string(),
            push_refspec: Some(format!("refs/heads/{destination_branch}")),
            squash: existing_source.map(|source| source.squash).unwrap_or(true),
        },
    )
}

pub(super) fn schedule_extract_subtree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    options: SubtreeExtractOptions,
    auth: Option<StagedGitAuth>,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let destination_repo = options.destination_repository.clone().map(Arc::new);
        let result = (|| {
            let normalized_path = normalize_subtree_split_path(&path)?;
            let path_arc = Arc::new(normalized_path.clone());

            if let Some(remote) = options.remote_repository.as_deref()
                && destination_repo.is_none()
            {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "a destination repository is required before publishing to `{remote}`"
                ))));
            }

            if let Some(destination_repo_path) = options.destination_repository.as_deref() {
                validate_destination_repository(&repo.spec().workdir, destination_repo_path)?;
            }

            if let Some(auth) = auth.clone() {
                stage_git_auth_for_current_thread(auth);
            }

            let existing_subtree = repo
                .list_subtrees()?
                .into_iter()
                .find(|subtree| subtree.path == normalized_path);
            let existing_source = existing_subtree
                .as_ref()
                .and_then(|subtree| subtree.source.clone());

            send_progress(
                &msg_tx,
                repo_id,
                Arc::clone(&path_arc),
                destination_repo.clone(),
                SubtreeExtractProgressMeter {
                    stage: SubtreeExtractProgressStage::Splitting,
                    percent: 5,
                },
                Some(format!(
                    "Splitting subtree history from {}",
                    normalized_path.display()
                )),
            );

            let split_output = repo.split_subtree_with_output(&normalized_path, &options.split)?;
            push_output_lines(
                &msg_tx,
                repo_id,
                &path_arc,
                destination_repo.as_ref(),
                SubtreeExtractProgressMeter {
                    stage: SubtreeExtractProgressStage::Splitting,
                    percent: 55,
                },
                &split_output,
            );
            let split_revision = parse_split_commit(&split_output)?;

            let Some(destination_repo_path) = options.destination_repository.as_ref() else {
                send_progress(
                    &msg_tx,
                    repo_id,
                    Arc::clone(&path_arc),
                    None,
                    SubtreeExtractProgressMeter {
                        stage: SubtreeExtractProgressStage::Splitting,
                        percent: 100,
                    },
                    Some(format!("Split complete at {split_revision}")),
                );
                if auth.is_some() {
                    clear_staged_git_auth();
                }
                return Ok(());
            };

            let destination_branch = default_destination_branch(&options, existing_source.as_ref());
            fs::create_dir_all(destination_repo_path).map_err(io_error)?;
            send_progress(
                &msg_tx,
                repo_id,
                Arc::clone(&path_arc),
                destination_repo.clone(),
                SubtreeExtractProgressMeter {
                    stage: SubtreeExtractProgressStage::PreparingDestination,
                    percent: 62,
                },
                Some(format!(
                    "Preparing destination repository at {}",
                    destination_repo_path.display()
                )),
            );

            let mut init_cmd = git_command(destination_repo_path);
            init_cmd.arg("init").arg("--quiet");
            let init_output = run_git(
                init_cmd,
                format!("git -C {} init --quiet", destination_repo_path.display()),
                None,
            )?;
            push_output_lines(
                &msg_tx,
                repo_id,
                &path_arc,
                destination_repo.as_ref(),
                SubtreeExtractProgressMeter {
                    stage: SubtreeExtractProgressStage::PreparingDestination,
                    percent: 68,
                },
                &init_output,
            );

            let mut fetch_cmd = git_command(destination_repo_path);
            fetch_cmd
                .arg("-c")
                .arg("protocol.file.allow=always")
                .arg("fetch")
                .arg("--no-tags")
                .arg(&repo.spec().workdir)
                .arg(&split_revision);
            let fetch_output = run_git(
                fetch_cmd,
                format!(
                    "git -C {} -c protocol.file.allow=always fetch --no-tags {} {}",
                    destination_repo_path.display(),
                    repo.spec().workdir.display(),
                    split_revision
                ),
                None,
            )?;
            push_output_lines(
                &msg_tx,
                repo_id,
                &path_arc,
                destination_repo.as_ref(),
                SubtreeExtractProgressMeter {
                    stage: SubtreeExtractProgressStage::PreparingDestination,
                    percent: 76,
                },
                &fetch_output,
            );

            let mut checkout_cmd = git_command(destination_repo_path);
            checkout_cmd
                .arg("checkout")
                .arg("-B")
                .arg(&destination_branch)
                .arg("FETCH_HEAD");
            let checkout_output = run_git(
                checkout_cmd,
                format!(
                    "git -C {} checkout -B {} FETCH_HEAD",
                    destination_repo_path.display(),
                    destination_branch
                ),
                None,
            )?;
            push_output_lines(
                &msg_tx,
                repo_id,
                &path_arc,
                destination_repo.as_ref(),
                SubtreeExtractProgressMeter {
                    stage: SubtreeExtractProgressStage::PreparingDestination,
                    percent: 86,
                },
                &checkout_output,
            );

            if let Some(remote) = options.remote_repository.as_deref() {
                let mut get_origin_cmd = git_command(destination_repo_path);
                get_origin_cmd.arg("remote").arg("get-url").arg("origin");
                let origin_output = run_git_output(
                    get_origin_cmd,
                    format!(
                        "git -C {} remote get-url origin",
                        destination_repo_path.display()
                    ),
                    None,
                )?;

                if origin_output.exit_code == Some(0) {
                    if origin_output.stdout.trim() != remote {
                        let mut set_url_cmd = git_command(destination_repo_path);
                        set_url_cmd
                            .arg("remote")
                            .arg("set-url")
                            .arg("origin")
                            .arg(remote);
                        let set_url_output = run_git(
                            set_url_cmd,
                            format!(
                                "git -C {} remote set-url origin {}",
                                destination_repo_path.display(),
                                remote
                            ),
                            None,
                        )?;
                        push_output_lines(
                            &msg_tx,
                            repo_id,
                            &path_arc,
                            destination_repo.as_ref(),
                            SubtreeExtractProgressMeter {
                                stage: SubtreeExtractProgressStage::PreparingDestination,
                                percent: 90,
                            },
                            &set_url_output,
                        );
                    }
                } else {
                    let mut add_remote_cmd = git_command(destination_repo_path);
                    add_remote_cmd
                        .arg("remote")
                        .arg("add")
                        .arg("origin")
                        .arg(remote);
                    let add_remote_output = run_git(
                        add_remote_cmd,
                        format!(
                            "git -C {} remote add origin {}",
                            destination_repo_path.display(),
                            remote
                        ),
                        None,
                    )?;
                    push_output_lines(
                        &msg_tx,
                        repo_id,
                        &path_arc,
                        destination_repo.as_ref(),
                        SubtreeExtractProgressMeter {
                            stage: SubtreeExtractProgressStage::PreparingDestination,
                            percent: 90,
                        },
                        &add_remote_output,
                    );
                }

                send_progress(
                    &msg_tx,
                    repo_id,
                    Arc::clone(&path_arc),
                    destination_repo.clone(),
                    SubtreeExtractProgressMeter {
                        stage: SubtreeExtractProgressStage::PublishingDestination,
                        percent: 94,
                    },
                    Some(format!("Publishing {} to {}", destination_branch, remote)),
                );

                let mut push_cmd = git_command(destination_repo_path);
                push_cmd
                    .arg("push")
                    .arg("-u")
                    .arg("origin")
                    .arg(format!("HEAD:refs/heads/{destination_branch}"));
                let push_output = run_git(
                    push_cmd,
                    format!(
                        "git -C {} push -u origin HEAD:refs/heads/{}",
                        destination_repo_path.display(),
                        destination_branch
                    ),
                    auth.as_ref(),
                )?;
                push_output_lines(
                    &msg_tx,
                    repo_id,
                    &path_arc,
                    destination_repo.as_ref(),
                    SubtreeExtractProgressMeter {
                        stage: SubtreeExtractProgressStage::PublishingDestination,
                        percent: 99,
                    },
                    &push_output,
                );
            }

            if existing_subtree.is_some() {
                store_updated_subtree_source(
                    &repo,
                    &normalized_path,
                    destination_repo_path,
                    &destination_branch,
                    options.remote_repository.as_deref(),
                    existing_source.as_ref(),
                )?;
            }
            send_progress(
                &msg_tx,
                repo_id,
                Arc::clone(&path_arc),
                destination_repo.clone(),
                SubtreeExtractProgressMeter {
                    stage: if options.remote_repository.is_some() {
                        SubtreeExtractProgressStage::PublishingDestination
                    } else {
                        SubtreeExtractProgressStage::PreparingDestination
                    },
                    percent: 100,
                },
                Some(format!(
                    "Subtree extracted into {}",
                    destination_repo_path.display()
                )),
            );

            if auth.is_some() {
                clear_staged_git_auth();
            }
            Ok(())
        })();

        if auth.is_some() {
            clear_staged_git_auth();
        }

        let destination_repo_path = options.destination_repository.clone();
        let succeeded = result.is_ok();
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::ExtractSubtreeFinished {
                repo_id,
                path: path.clone(),
                destination_repo: destination_repo_path.clone(),
                result,
            }),
        );
        if succeeded && let Some(destination_repo_path) = destination_repo_path {
            send_or_log(&msg_tx, Msg::OpenRepo(destination_repo_path));
        }
    });
}
