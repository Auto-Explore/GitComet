use crate::msg::Msg;
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::auth::askpass::{
    AskPassScript, GIT_COMMAND_TIMEOUT_ENV, PromptAuth, append_host_prompt_to_stderr,
    configure_git_auth_prompt, create_askpass_script, git_command_timeout,
    remember_successful_prompt_auth, resolve_git_auth,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::{bytes_to_text_preserving_utf8, git_command};
use gitcomet_core::remote_url::validate_remote_url;
use gitcomet_core::services::CommandOutput;
use gitcomet_core::text_utils::redact_url_userinfo;
use rustc_hash::FxHashMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::super::{executor::TaskExecutor, worker_channel::StoreWorkerSender};
use super::util::send_or_log;

#[cfg(test)]
use gitcomet_core::auth::{
    CachedPassphraseEntry, GITCOMET_AUTH_CACHE_SIZE_ENV, GITCOMET_AUTH_KIND_ENV,
    GITCOMET_AUTH_KIND_PASSPHRASE_CACHED, GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV,
};

const GIT_COMMAND_WAIT_POLL: Duration = Duration::from_millis(100);
struct ActiveCloneHandle {
    cancel_requested: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl ActiveCloneHandle {
    fn new() -> Self {
        Self {
            cancel_requested: AtomicBool::new(false),
            child: Mutex::new(None),
        }
    }

    fn set_child(&self, child: Child) {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(child);
        if self.cancel_requested.load(Ordering::Relaxed)
            && let Some(child) = slot.as_mut()
        {
            let _ = child.kill();
        }
    }

    fn take_stdio(&self) -> (Option<ChildStdout>, Option<ChildStderr>) {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let Some(child) = slot.as_mut() else {
            return (None, None);
        };
        (child.stdout.take(), child.stderr.take())
    }

    fn try_wait(&self) -> std::io::Result<Option<ExitStatus>> {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_mut() {
            Some(child) => child.try_wait(),
            None => Err(std::io::Error::other("clone child missing")),
        }
    }

    fn wait(&self) -> std::io::Result<ExitStatus> {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_mut() {
            Some(child) => child.wait(),
            None => Err(std::io::Error::other("clone child missing")),
        }
    }

    fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = slot.as_mut() {
            let _ = child.kill();
        }
    }

    fn cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Relaxed)
    }
}

struct ActiveCloneRegistration {
    dest: PathBuf,
    handle: Arc<ActiveCloneHandle>,
}

impl ActiveCloneRegistration {
    fn new(dest: PathBuf, handle: Arc<ActiveCloneHandle>) -> Self {
        active_clones()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(dest.clone(), Arc::clone(&handle));
        Self { dest, handle }
    }
}

impl Drop for ActiveCloneRegistration {
    fn drop(&mut self) {
        let mut clones = active_clones().lock().unwrap_or_else(|e| e.into_inner());
        if clones
            .get(&self.dest)
            .is_some_and(|current| Arc::ptr_eq(current, &self.handle))
        {
            clones.remove(&self.dest);
        }
    }
}

fn active_clones() -> &'static Mutex<FxHashMap<PathBuf, Arc<ActiveCloneHandle>>> {
    static ACTIVE_CLONES: OnceLock<Mutex<FxHashMap<PathBuf, Arc<ActiveCloneHandle>>>> =
        OnceLock::new();
    ACTIVE_CLONES.get_or_init(|| Mutex::new(FxHashMap::default()))
}

/// The label shown in the command log and in failure messages; the URL is
/// masked there because a pasted `https://user:token@host` must not be echoed.
fn clone_command_label(url: &str, dest: &Path) -> String {
    format!(
        "git clone --progress {} {}",
        redact_url_userinfo(url),
        dest.display()
    )
}

/// `--` keeps a URL or destination that starts with `-` from being parsed as
/// a `git clone` option (`--upload-pack=<cmd>` would run `<cmd>` locally).
fn build_clone_command(url: &str, dest: &Path) -> Command {
    let mut cmd = git_command();
    cmd.arg("-c")
        .arg("color.ui=false")
        .arg("clone")
        .arg("--progress")
        .arg("--")
        .arg(url)
        .arg(dest);
    cmd
}

fn decode_clone_progress_fragment(fragment: &[u8]) -> Option<String> {
    let fragment = bytes_to_text_preserving_utf8(fragment);
    let fragment = fragment.trim_matches(|ch| matches!(ch, '\r' | '\n'));
    (!fragment.is_empty()).then(|| fragment.to_string())
}

fn take_clone_progress_fragments(pending: &mut Vec<u8>, eof: bool) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut start = 0usize;
    let mut ix = 0usize;

    while ix < pending.len() {
        if matches!(pending[ix], b'\r' | b'\n') {
            if ix > start
                && let Some(fragment) = decode_clone_progress_fragment(&pending[start..ix])
            {
                fragments.push(fragment);
            }

            start = ix + 1;
            if pending[ix] == b'\r' && start < pending.len() && pending[start] == b'\n' {
                start += 1;
                ix += 1;
            }
        }
        ix += 1;
    }

    if eof {
        if start < pending.len()
            && let Some(fragment) = decode_clone_progress_fragment(&pending[start..])
        {
            fragments.push(fragment);
        }
        pending.clear();
    } else if start > 0 {
        pending.drain(..start);
    }

    fragments
}

fn cleanup_aborted_clone_destination(dest: &Path, dest_preexisted: bool) -> Result<(), Error> {
    if dest_preexisted {
        return Ok(());
    }

    let metadata = match fs::symlink_metadata(dest) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(Error::new(ErrorKind::Backend(format!(
                "clone aborted, but failed to inspect partially created destination `{}`: {err}",
                dest.display()
            ))));
        }
    };

    let remove = || {
        if metadata.file_type().is_dir() {
            fs::remove_dir_all(dest)
        } else {
            fs::remove_file(dest)
        }
    };

    #[cfg(windows)]
    let mut removal_result = remove();
    #[cfg(not(windows))]
    let removal_result = remove();
    #[cfg(windows)]
    for _ in 0..10 {
        if removal_result.is_ok() || !dest.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        removal_result = remove();
    }

    removal_result.map_err(|err| {
        Error::new(ErrorKind::Backend(format!(
            "clone aborted, but failed to remove partially created destination `{}`: {err}",
            dest.display()
        )))
    })
}

pub(super) fn schedule_clone_repo(
    executor: &TaskExecutor,
    msg_tx: StoreWorkerSender,
    url: String,
    dest: PathBuf,
    auth: Option<StagedGitAuth>,
) {
    let active_clone = Arc::new(ActiveCloneHandle::new());
    let registration = ActiveCloneRegistration::new(dest.clone(), Arc::clone(&active_clone));
    let dest_preexisted = dest.exists();

    executor.spawn(move || {
        let _registration = registration;

        if let Err(err) = validate_remote_url(&url) {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                    url,
                    dest,
                    result: Err(err),
                }),
            );
            return;
        }

        let mut cmd = build_clone_command(&url, &dest);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0");

        let (askpass_script, prompt_auth) = match (|| {
            let auth = resolve_git_auth(auth);
            let script = create_askpass_script().map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            configure_git_auth_prompt(&mut cmd, auth.as_ref(), &script);
            Ok::<(AskPassScript, Option<PromptAuth>), Error>((script, auth))
        })() {
            Ok(context) => context,
            Err(err) => {
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                        url: url.clone(),
                        dest: dest.clone(),
                        result: Err(err),
                    }),
                );
                return;
            }
        };

        let command_str = clone_command_label(&url, &dest);

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let err = Error::new(ErrorKind::Io(e.kind()));
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                        url,
                        dest,
                        result: Err(err),
                    }),
                );
                return;
            }
        };
        active_clone.set_child(child);

        let (stdout, stderr) = active_clone.take_stdio();
        let stdout_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stdout) = stdout {
                let _ = stdout.read_to_end(&mut buf);
            }
            bytes_to_text_preserving_utf8(&buf)
        });

        let progress_dest = Arc::new(dest.clone());
        let progress_tx = msg_tx.clone();
        let stderr_handle = std::thread::spawn(move || {
            let mut stderr_bytes = Vec::new();
            let mut pending = Vec::new();
            if let Some(mut stderr) = stderr {
                let mut buf = [0u8; 4096];
                loop {
                    match stderr.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = &buf[..n];
                            stderr_bytes.extend_from_slice(chunk);
                            pending.extend_from_slice(chunk);
                            for line in take_clone_progress_fragments(&mut pending, false) {
                                send_or_log(
                                    &progress_tx,
                                    Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
                                        dest: Arc::clone(&progress_dest),
                                        line,
                                    }),
                                );
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            for line in take_clone_progress_fragments(&mut pending, true) {
                send_or_log(
                    &progress_tx,
                    Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
                        dest: Arc::clone(&progress_dest),
                        line,
                    }),
                );
            }
            stderr_bytes
        });

        let timeout = git_command_timeout();
        let start = Instant::now();
        let mut timed_out = false;
        let status = loop {
            match active_clone.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        timed_out = true;
                        active_clone.request_cancel();
                        break active_clone.wait();
                    }
                    std::thread::sleep(GIT_COMMAND_WAIT_POLL);
                }
                Err(e) => break Err(e),
            }
        };
        let stdout_str = stdout_handle.join().unwrap_or_default();
        let mut stderr_bytes = stderr_handle.join().unwrap_or_default();
        append_host_prompt_to_stderr(&mut stderr_bytes, &askpass_script);
        let stderr_acc = bytes_to_text_preserving_utf8(&stderr_bytes);

        let mut result = match status {
            Ok(status) => {
                if timed_out {
                    Err(Error::new(ErrorKind::Backend(format!(
                        "{command_str} timed out after {} seconds (set {GIT_COMMAND_TIMEOUT_ENV} to override)",
                        timeout.as_secs()
                    ))))
                } else if active_clone.cancel_requested() && !status.success() {
                    Err(Error::new(ErrorKind::Backend("clone aborted".to_string())))
                } else {
                    let out = CommandOutput {
                        command: command_str,
                        stdout: stdout_str,
                        stderr: stderr_acc,
                        exit_code: status.code(),
                    };
                    if status.success() {
                        Ok(out)
                    } else {
                        let combined = out.combined();
                        let message = if combined.is_empty() {
                            format!("{} failed", out.command)
                        } else {
                            format!("{} failed: {combined}", out.command)
                        };
                        Err(Error::new(ErrorKind::Backend(message)))
                    }
                }
            }
            Err(e) => Err(Error::new(ErrorKind::Io(e.kind()))),
        };

        if result.is_err() && active_clone.cancel_requested()
            && let Err(cleanup_err) = cleanup_aborted_clone_destination(&dest, dest_preexisted) {
                result = Err(match result {
                    Ok(_) => cleanup_err,
                    Err(err) => Error::new(ErrorKind::Backend(format!("{err}; {cleanup_err}"))),
                });
            }

        if result.is_ok() {
            remember_successful_prompt_auth(prompt_auth.as_ref(), &askpass_script);
        }

        let ok = result.is_ok();
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                url: url.clone(),
                dest: dest.clone(),
                result,
            }),
        );

        if ok {
            send_or_log(&msg_tx, Msg::OpenRepo(dest));
        }
    });
}

pub(super) fn schedule_abort_clone_repo(_msg_tx: StoreWorkerSender, dest: PathBuf) {
    let handle = active_clones()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&dest)
        .cloned();
    if let Some(handle) = handle {
        handle.request_cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn command_env_value(cmd: &Command, key: &str) -> Option<String> {
        use std::ffi::OsStr;

        cmd.get_envs().find_map(|(k, v)| {
            if k == OsStr::new(key) {
                v.and_then(|value| value.to_str().map(ToOwned::to_owned))
            } else {
                None
            }
        })
    }

    fn command_env_removed(cmd: &Command, key: &str) -> bool {
        use std::ffi::OsStr;

        cmd.get_envs()
            .any(|(k, v)| k == OsStr::new(key) && v.is_none())
    }

    /// Scheme coverage lives with the validator; this pins the clone-only shape.
    #[test]
    fn validate_clone_url_rejects_option_like_inputs() {
        for url in [
            "-",
            "-o=evil",
            "--upload-pack=touch /tmp/pwned",
            "  --template=/tmp/x",
        ] {
            let err = validate_remote_url(url).expect_err(url);
            assert!(
                err.to_string().contains("cannot start with '-'"),
                "{url}: {err}"
            );
        }
    }

    #[test]
    fn clone_command_label_masks_credentials_in_the_url() {
        let dest = Path::new("/tmp/gitcomet-clone-dest");
        assert_eq!(
            clone_command_label("https://user:s3cret@example.com/org/repo.git", dest),
            "git clone --progress https://user:***@example.com/org/repo.git /tmp/gitcomet-clone-dest"
        );
        let cmd = build_clone_command("https://user:s3cret@example.com/org/repo.git", dest);
        assert!(
            cmd.get_args()
                .any(|arg| arg == "https://user:s3cret@example.com/org/repo.git"),
            "argv must carry the real URL"
        );
    }

    #[test]
    fn build_clone_command_separates_positionals_from_options() {
        let dest = Path::new("/tmp/gitcomet-clone-dest");
        let cmd = build_clone_command("git@github.com:org/repo.git", dest);
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                std::ffi::OsStr::new("-c"),
                std::ffi::OsStr::new("protocol.ext.allow=never"),
                std::ffi::OsStr::new("-c"),
                std::ffi::OsStr::new("color.ui=false"),
                std::ffi::OsStr::new("clone"),
                std::ffi::OsStr::new("--progress"),
                std::ffi::OsStr::new("--"),
                std::ffi::OsStr::new("git@github.com:org/repo.git"),
                dest.as_os_str(),
            ]
        );
    }

    #[test]
    fn append_host_prompt_to_stderr_includes_logged_prompt_with_fingerprint() {
        let askpass = create_askpass_script().expect("askpass script creation");
        std::fs::write(
            askpass.host_prompt_log_path(),
            "The authenticity of host 'github.com (140.82.121.3)' can't be established.\nED25519 key fingerprint is: SHA256:+DiY...\nAre you sure you want to continue connecting (yes/no/[fingerprint])?",
        )
        .expect("write prompt log");

        let mut stderr = b"Host key verification failed.\n".to_vec();
        append_host_prompt_to_stderr(&mut stderr, &askpass);
        let stderr = bytes_to_text_preserving_utf8(&stderr);

        assert!(stderr.contains("SSH host verification prompt:"));
        assert!(stderr.contains("ED25519 key fingerprint is: SHA256:+DiY..."));
        assert!(stderr.contains("yes/no/[fingerprint]"));
    }

    #[test]
    fn append_host_prompt_to_stderr_skips_when_prompt_already_present() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let prompt = "Are you sure you want to continue connecting (yes/no/[fingerprint])?";
        std::fs::write(askpass.host_prompt_log_path(), prompt).expect("write prompt log");

        let mut stderr = format!("Host key verification failed.\n{prompt}\n").into_bytes();
        append_host_prompt_to_stderr(&mut stderr, &askpass);
        let stderr = bytes_to_text_preserving_utf8(&stderr);

        assert_eq!(stderr.matches("SSH host verification prompt:").count(), 0);
        assert_eq!(stderr.matches(prompt).count(), 1);
    }

    #[test]
    fn take_clone_progress_fragments_streams_carriage_return_updates() {
        let mut pending =
            b"Receiving objects:   1% (1/100)\rReceiving objects:  20% (20/100)".to_vec();

        let fragments = take_clone_progress_fragments(&mut pending, false);
        assert_eq!(fragments, vec!["Receiving objects:   1% (1/100)"]);
        assert_eq!(pending, b"Receiving objects:  20% (20/100)".to_vec());

        pending.extend_from_slice(b"\rResolving deltas:   5% (1/20)\n");
        let fragments = take_clone_progress_fragments(&mut pending, false);
        assert_eq!(
            fragments,
            vec![
                "Receiving objects:  20% (20/100)",
                "Resolving deltas:   5% (1/20)",
            ]
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn take_clone_progress_fragments_retains_partial_chunks_between_reads() {
        let mut pending = b"Receiving objects:  4".to_vec();
        assert!(take_clone_progress_fragments(&mut pending, false).is_empty());
        assert_eq!(pending, b"Receiving objects:  4".to_vec());

        pending.extend_from_slice(b"2% (42/100)\rResolving deltas:  1");
        assert_eq!(
            take_clone_progress_fragments(&mut pending, false),
            vec!["Receiving objects:  42% (42/100)"]
        );
        assert_eq!(pending, b"Resolving deltas:  1".to_vec());

        pending.extend_from_slice(b"0% (2/20)\n");
        assert_eq!(
            take_clone_progress_fragments(&mut pending, false),
            vec!["Resolving deltas:  10% (2/20)"]
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn take_clone_progress_fragments_handles_a_crlf_split_across_reads() {
        // The `\r\n` skip looks one byte past the terminator, which is not there
        // yet on the first read. Consuming the `\n` again on the second read
        // would emit an empty fragment and reset the toast to no progress.
        let mut pending = b"Receiving objects:  42% (42/100)\r".to_vec();
        assert_eq!(
            take_clone_progress_fragments(&mut pending, false),
            vec!["Receiving objects:  42% (42/100)"]
        );
        assert!(pending.is_empty(), "the lone carriage return is consumed");

        pending.extend_from_slice(b"\nResolving deltas: 100% (20/20)\r\n");
        assert_eq!(
            take_clone_progress_fragments(&mut pending, false),
            vec!["Resolving deltas: 100% (20/20)"],
            "the orphaned newline must not produce an empty fragment"
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn take_clone_progress_fragments_flushes_remainder_at_eof() {
        let mut pending = b"Updating files: 100% (4/4), done.".to_vec();
        let fragments = take_clone_progress_fragments(&mut pending, true);
        assert_eq!(fragments, vec!["Updating files: 100% (4/4), done."]);
        assert!(pending.is_empty());
    }

    #[test]
    fn cleanup_aborted_clone_destination_removes_new_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("clone");
        std::fs::create_dir_all(dest.join(".git").join("objects")).expect("create clone dir");
        std::fs::write(dest.join(".git").join("HEAD"), "ref: refs/heads/main\n")
            .expect("write head");

        cleanup_aborted_clone_destination(&dest, false).expect("cleanup succeeds");

        assert!(
            !dest.exists(),
            "aborted clone destination should be removed"
        );
    }

    #[test]
    fn cleanup_aborted_clone_destination_preserves_preexisting_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("clone");
        std::fs::create_dir_all(&dest).expect("create preexisting dest");
        let sentinel = dest.join("keep.txt");
        std::fs::write(&sentinel, "keep\n").expect("write sentinel");

        cleanup_aborted_clone_destination(&dest, true).expect("cleanup succeeds");

        assert!(dest.exists(), "preexisting destination should be preserved");
        assert!(sentinel.exists(), "preexisting contents should remain");
    }

    #[test]
    fn cleanup_aborted_clone_destination_removes_new_file_like_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("clone");
        std::fs::write(&dest, "partial\n").expect("create partial file");

        cleanup_aborted_clone_destination(&dest, false).expect("cleanup succeeds");

        assert!(!dest.exists(), "partial file should be removed");
    }

    #[test]
    fn configure_clone_auth_prompt_sets_cached_passphrase_env_and_removes_username() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let mut cmd = Command::new("git");
        cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
        let auth = PromptAuth::CachedPassphrases(vec![
            CachedPassphraseEntry {
                prompt: "Enter passphrase for key '/tmp/key-a':".to_string(),
                secret: "ssh-passphrase-a".to_string(),
            },
            CachedPassphraseEntry {
                prompt: "Enter passphrase for key '/tmp/key-b':".to_string(),
                secret: "ssh-passphrase-b".to_string(),
            },
        ]);

        configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
            Some(GITCOMET_AUTH_KIND_PASSPHRASE_CACHED)
        );
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_SECRET_ENV));
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_CACHE_SIZE_ENV).as_deref(),
            Some("2")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_0").as_deref(),
            Some("Enter passphrase for key '/tmp/key-a':")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_0").as_deref(),
            Some("ssh-passphrase-a")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_1").as_deref(),
            Some("Enter passphrase for key '/tmp/key-b':")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_1").as_deref(),
            Some("ssh-passphrase-b")
        );
    }
}
