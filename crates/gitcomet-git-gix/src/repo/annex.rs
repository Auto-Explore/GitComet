//! git-annex operations, run through `git annex` so its location tracking,
//! numcopies checks and special remotes all apply.

use crate::util::{
    run_git_background_capture, run_git_parsed_stdout_until_done, run_git_with_output,
    run_git_with_output_until_done, validate_ref_like_arg,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::git_operation::{GitOperationEvent, TransferProgress};
use gitcomet_core::large_files::{
    AnnexLocation, AnnexRepository, AnnexTrust, AnnexUnused, AnnexUnusedEntry, AnnexUnusedKind,
    AnnexWhereis, LargeFileCommand,
};
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use rustc_hash::FxHashMap;
use std::ffi::OsString;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};

/// Progress reaches the store as a message and a repaint; git-annex prints
/// one line per file at least, hundreds a second for small files.
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

fn backend(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Backend(message.into()))
}

/// Names reach argv; refuse anything git-annex could read as an option.
fn validate_name(value: &str, what: &str) -> Result<()> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(backend(format!("invalid {what}: {value:?}")));
    }
    Ok(())
}

fn validate_remote_params(params: &[String]) -> Result<()> {
    match params
        .iter()
        .find(|param| !param.contains('=') || param.starts_with('-'))
    {
        Some(bad) => Err(backend(format!(
            "special remote parameters are key=value pairs; got {bad:?}"
        ))),
        None => Ok(()),
    }
}

/// One finished item from a `--json` run.
#[derive(Debug, Default)]
struct AnnexItem {
    file: Option<String>,
    success: bool,
    note: Option<String>,
    errors: Vec<String>,
}

/// Parse a `--json-progress` line into activity progress.
fn parse_progress(value: &serde_json::Value) -> Option<TransferProgress> {
    let action = value.get("action")?;
    Some(TransferProgress {
        direction: format!("annex {}", action.get("command")?.as_str()?),
        files_done: 0,
        files_total: 0,
        bytes_done: value.get("byte-progress")?.as_u64()?,
        bytes_total: value
            .get("total-size")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        name: action
            .get("file")
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

fn parse_item(value: &serde_json::Value) -> Option<AnnexItem> {
    let success = value.get("success")?.as_bool()?;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    Some(AnnexItem {
        file: text("file"),
        success,
        note: text("note"),
        errors: value
            .get("error-messages")
            .and_then(|v| v.as_array())
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|line| line.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// A readable summary of what failed, with git-annex's own reason. A refused
/// drop explains that the content has no verified copy elsewhere.
fn failure_detail(failed: &[&AnnexItem]) -> String {
    let mut lines = Vec::new();
    for item in failed {
        let reason = item
            .errors
            .iter()
            .map(|e| e.trim().to_string())
            .chain(item.note.iter().map(|n| n.trim().replace('\n', " ")))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(match &item.file {
            Some(file) => format!("{file}: {reason}"),
            None => reason,
        });
    }
    let mut detail = lines.join("\n");
    if detail.contains("Could not verify the existence") {
        detail.push_str(
            "\n\nHint: git-annex keeps content until enough other copies are verified. Copy it to another repository first, or use Move to.",
        );
    }
    detail
}

/// git-annex before 10.20230626 rejects `pull` and `push` as its argument
/// parser's "Invalid argument `pull'".
fn unknown_annex_command(error: &Error, verb: &str) -> bool {
    let ErrorKind::Git(failure) = error.kind() else {
        return false;
    };
    let stderr = String::from_utf8_lossy(failure.stderr());
    stderr
        .lines()
        .any(|line| line.contains("Invalid argument") && line.contains(verb))
}

/// A remote this clone reaches, as `remote.<name>.*` config knows it.
struct KnownRemote {
    uuid: String,
    name: String,
    /// Guessed from the remote's own `annex-*` settings.
    special_type: Option<String>,
    /// `annex-trustlevel`, which overrides trust.log here.
    trust_level: Option<String>,
}

/// Remotes with an `annex-uuid`, and the special remote type from the
/// remote's own `annex-*` settings.
fn remote_names(config: &gix::config::Snapshot<'_>) -> Vec<KnownRemote> {
    let mut remotes = Vec::new();
    for section in config
        .plumbing()
        .sections_by_name("remote")
        .into_iter()
        .flatten()
    {
        let Some(name) = section.header().subsection_name() else {
            continue;
        };
        let name = name.to_string();
        let value = |key: &str| section.value(key).map(|v| v.to_string());
        let Some(uuid) = value("annex-uuid") else {
            continue;
        };
        let special_type = if value("url").is_some() {
            None
        } else {
            Some(
                value("annex-externaltype")
                    .or_else(|| value("annex-directory").map(|_| "directory".into()))
                    .or_else(|| value("annex-rsyncurl").map(|_| "rsync".into()))
                    .or_else(|| {
                        (value("annex-bucket").is_some() || value("annex-s3").is_some())
                            .then(|| "S3".into())
                    })
                    .or_else(|| value("annex-webdav").map(|_| "webdav".into()))
                    .unwrap_or_else(|| "special".into()),
            )
        };
        remotes.push(KnownRemote {
            uuid,
            name,
            special_type,
            trust_level: value("annex-trustlevel"),
        });
    }
    remotes
}

/// `1790075913.5s` in a log line.
fn log_timestamp(text: &str) -> f64 {
    text.trim_end_matches('s').parse().unwrap_or(0.0)
}

/// Newest value per uuid of a `<uuid> <value> timestamp=<t>s` log such as
/// uuid.log or trust.log; later lines win ties, as the journal is newer.
fn newest_per_uuid(text: &str) -> FxHashMap<String, String> {
    let mut newest: FxHashMap<String, (f64, String)> = FxHashMap::default();
    for line in text.lines() {
        let Some((uuid, rest)) = line.trim_end().split_once(' ') else {
            continue;
        };
        let (value, timestamp) = match rest.rsplit_once(" timestamp=") {
            Some((value, timestamp)) => (value, log_timestamp(timestamp)),
            None => (rest, 0.0),
        };
        if newest
            .get(uuid)
            .is_none_or(|(existing, _)| timestamp >= *existing)
        {
            newest.insert(uuid.to_string(), (timestamp, value.to_string()));
        }
    }
    newest
        .into_iter()
        .map(|(uuid, (_, value))| (uuid, value))
        .collect()
}

/// numcopies.log: `<t>s <n>`, or `<n> timestamp=<t>s` from older git-annex.
fn parse_numcopies_log(text: &str) -> Option<u32> {
    let mut newest: Option<(f64, u32)> = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(first), Some(second)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (timestamp, value) = match second.strip_prefix("timestamp=") {
            Some(timestamp) => (log_timestamp(timestamp), first),
            None => (log_timestamp(first), second),
        };
        let Ok(value) = value.parse() else { continue };
        if newest.is_none_or(|(existing, _)| timestamp >= existing) {
            newest = Some((timestamp, value));
        }
    }
    newest.map(|(_, value)| value)
}

/// git-annex's own pseudo-repositories, always listed unless dead.
const BUILTIN_REPOSITORIES: [(&str, &str); 2] = [
    ("00000000-0000-0000-0000-000000000001", "web"),
    ("00000000-0000-0000-0000-000000000002", "bittorrent"),
];

/// Repositories as `git annex info` lists them: every uuid with a description
/// or a remote, dead ones left out, grouped by trust and ordered by uuid.
/// A remote's name joins its description the way git-annex shows it.
fn repositories_from_logs(
    uuid_log: &str,
    trust_log: &str,
    remotes: &[KnownRemote],
    special_remotes: &FxHashMap<String, SpecialRemote>,
    here: Option<&str>,
) -> Vec<AnnexRepository> {
    let mut descriptions = newest_per_uuid(uuid_log);
    for (uuid, description) in BUILTIN_REPOSITORIES {
        descriptions.insert(uuid.to_string(), description.to_string());
    }
    for remote in remotes {
        descriptions.entry(remote.uuid.clone()).or_default();
    }
    let trust_log = newest_per_uuid(trust_log);
    let mut repositories: Vec<AnnexRepository> = descriptions
        .into_iter()
        .filter_map(|(uuid, description)| {
            let remote = remotes.iter().find(|remote| remote.uuid == uuid);
            let level = remote
                .and_then(|remote| remote.trust_level.as_deref())
                .map(|level| match level {
                    "trusted" => "1",
                    "untrusted" => "0",
                    "dead" => "X",
                    _ => "?",
                })
                .or_else(|| trust_log.get(&uuid).map(String::as_str));
            let trust = match level {
                Some("X") => return None,
                Some("1") => AnnexTrust::Trusted,
                Some("0") => AnnexTrust::Untrusted,
                _ => AnnexTrust::Semitrusted,
            };
            let description = match remote {
                Some(remote) => {
                    let name = format!("[{}]", remote.name);
                    if description.is_empty() || description == remote.name {
                        name
                    } else {
                        format!("{description} {name}")
                    }
                }
                None => description,
            };
            Some(AnnexRepository {
                description,
                remote_name: remote.map(|remote| remote.name.clone()),
                // remote.log is authoritative and also covers special remotes
                // this clone has not enabled; the config guess is a fallback.
                special_type: special_remotes
                    .get(&uuid)
                    .map(|special| special.special_type.clone())
                    .or_else(|| remote.and_then(|remote| remote.special_type.clone())),
                special_name: special_remotes
                    .get(&uuid)
                    .and_then(|special| special.name.clone()),
                trust,
                here: here == Some(uuid.as_str()),
                uuid,
            })
        })
        .collect();
    let rank = |trust: AnnexTrust| match trust {
        AnnexTrust::Trusted => 0,
        AnnexTrust::Semitrusted => 1,
        AnnexTrust::Untrusted => 2,
    };
    repositories.sort_by(|a, b| (rank(a.trust), &a.uuid).cmp(&(rank(b.trust), &b.uuid)));
    repositories
}

#[derive(Clone, Debug, PartialEq)]
struct SpecialRemote {
    name: Option<String>,
    special_type: String,
}

/// Special remotes by uuid from `remote.log` lines
/// (`<uuid> name=usb type=directory … timestamp=<n>s`); the newest line per
/// uuid wins. `type=git` remotes are git repositories.
fn parse_remote_log(text: &str) -> FxHashMap<String, SpecialRemote> {
    let mut newest: FxHashMap<String, (u64, SpecialRemote)> = FxHashMap::default();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(uuid) = fields.next() else { continue };
        let mut special_type = None;
        let mut name = None;
        let mut timestamp = 0u64;
        for field in fields {
            if let Some(value) = field.strip_prefix("type=") {
                special_type = Some(value);
            } else if let Some(value) = field.strip_prefix("name=") {
                name = Some(value.to_string());
            } else if let Some(value) = field.strip_prefix("timestamp=") {
                // Fractional seconds (`1790075913.5s`) only matter for ties.
                timestamp = value
                    .trim_end_matches('s')
                    .split('.')
                    .next()
                    .and_then(|secs| secs.parse().ok())
                    .unwrap_or(0);
            }
        }
        let Some(special_type) = special_type else {
            continue;
        };
        if newest
            .get(uuid)
            .is_none_or(|(existing, _)| timestamp >= *existing)
        {
            let special = SpecialRemote {
                name,
                special_type: special_type.to_string(),
            };
            newest.insert(uuid.to_string(), (timestamp, special));
        }
    }
    newest
        .into_iter()
        .filter(|(_, (_, special))| special.special_type != "git")
        .map(|(uuid, (_, special))| (uuid, special))
        .collect()
}

/// A top-level git-annex log: the local `git-annex` branch's copy, then the
/// journals' uncommitted lines, which are newer. Fetched `git-annex` branches
/// are not merged in; git-annex does that when a command runs.
fn annex_log(repo: &gix::Repository, name: &str) -> String {
    let mut text = repo
        .find_reference("refs/heads/git-annex")
        .ok()
        .and_then(|mut reference| reference.peel_to_tree().ok())
        .and_then(|tree| tree.lookup_entry_by_path(name).ok().flatten())
        .and_then(|entry| entry.object().ok())
        .map(|object| String::from_utf8_lossy(&object.data).into_owned())
        .unwrap_or_default();
    let annex_dir = repo.common_dir().join("annex");
    for journal in ["journal", "journal-private"] {
        if let Ok(lines) = std::fs::read_to_string(annex_dir.join(journal).join(name)) {
            text.push('\n');
            text.push_str(&lines);
        }
    }
    text
}

fn special_remotes(repo: &gix::Repository) -> FxHashMap<String, SpecialRemote> {
    parse_remote_log(&annex_log(repo, "remote.log"))
}

/// `git annex unused --json`: one object with numbered key lists.
fn parse_unused(json: &str) -> Result<AnnexUnused> {
    let value: serde_json::Value = json
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str(line).ok())
        .ok_or_else(|| backend("git annex unused: no result"))?;
    let mut entries = Vec::new();
    for (list, kind) in [
        ("unused-list", AnnexUnusedKind::Unused),
        ("bad-list", AnnexUnusedKind::Bad),
        ("tmp-list", AnnexUnusedKind::Temporary),
    ] {
        let Some(map) = value.get(list).and_then(|v| v.as_object()) else {
            continue;
        };
        // Numbered "1", "2", …; keep git-annex's order.
        let mut numbered: Vec<(u64, &str)> = map
            .iter()
            .filter_map(|(number, key)| Some((number.parse().ok()?, key.as_str()?)))
            .collect();
        numbered.sort_unstable_by_key(|(number, _)| *number);
        entries.extend(numbered.into_iter().map(|(_, key)| AnnexUnusedEntry {
            key: key.to_string(),
            kind,
        }));
    }
    Ok(AnnexUnused { entries })
}

/// The assistant writes its pid to `.git/annex/daemon.pid` and leaves it
/// behind when killed, so the process itself is checked.
pub(super) fn assistant_running(annex_dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(annex_dir.join("daemon.pid")) else {
        return false;
    };
    let Some(pid) = text.trim().parse::<i32>().ok().filter(|pid| *pid > 0) else {
        return false;
    };
    #[cfg(unix)]
    {
        rustix::process::Pid::from_raw(pid)
            .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
    }
    #[cfg(windows)]
    {
        // git-annex's own check on Windows: the running assistant holds
        // `daemon.pid.<pid>.lck` exclusively; a killed one leaves it openable.
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_SHARE_READ: u32 = 1;
        const ERROR_SHARING_VIOLATION: i32 = 32;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(annex_dir.join(format!("daemon.pid.{pid}.lck")))
            .is_err_and(|error| error.raw_os_error() == Some(ERROR_SHARING_VIOLATION))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

fn parse_whereis(json: &str) -> Result<AnnexWhereis> {
    let value: serde_json::Value = json
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str(line).ok())
        .ok_or_else(|| backend("git annex whereis: no result"))?;
    let locations = |key: &str| -> Vec<AnnexLocation> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                Some(AnnexLocation {
                    uuid: entry.get("uuid")?.as_str()?.to_string(),
                    description: entry
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    here: entry.get("here").and_then(|v| v.as_bool()).unwrap_or(false),
                })
            })
            .collect()
    };
    Ok(AnnexWhereis {
        key: value
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        copies: locations("whereis"),
        untrusted: locations("untrusted"),
    })
}

impl super::GixRepo {
    fn git_annex(&self, args: &[&str]) -> std::process::Command {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("annex").args(args);
        cmd
    }

    /// Run a `--json` annex command over paths, streaming `--json-progress`
    /// lines into the activity panel. Fails when any item failed, with
    /// git-annex's own reason per file. Runs until done or cancelled: a
    /// remote that reports no progress keeps a transfer silent.
    fn run_annex_json(
        &self,
        args: &[OsString],
        paths: &[PathBuf],
        label: &str,
    ) -> Result<CommandOutput> {
        let by_key = args
            .iter()
            .any(|arg| arg.to_string_lossy().starts_with("--key="));
        // `dropunused all` names its targets itself.
        let names_targets = by_key || args.iter().any(|arg| arg == "all");
        if paths.is_empty() && !names_targets {
            return Err(backend(format!("{label}: no paths given")));
        }
        let mut cmd = self.git_annex(&["--json", "--json-error-messages"]);
        cmd.args(args);
        if !paths.is_empty() {
            cmd.arg("--").args(paths);
        }
        // The reader runs on its own thread; report progress to this operation.
        // Items are shared so a failing exit can still report per-file reasons.
        let operation = gitcomet_core::git_operation::current();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let reader_items = std::sync::Arc::clone(&shared);
        let run = run_git_parsed_stdout_until_done(cmd, label, move |stdout| {
            let emit = |progress| {
                if let Some(operation) = operation.as_ref() {
                    operation.emit(GitOperationEvent::TransferProgress(progress));
                }
            };
            let mut last_emit: Option<std::time::Instant> = None;
            let mut pending = None;
            for line in std::io::BufReader::new(stdout).lines() {
                let line = line.map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if let Some(progress) = parse_progress(&value) {
                    if last_emit.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL) {
                        last_emit = Some(std::time::Instant::now());
                        pending = None;
                        emit(progress);
                    } else {
                        pending = Some(progress);
                    }
                } else if let Some(item) = parse_item(&value) {
                    reader_items
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(item);
                }
            }
            // The newest state always arrives, however soon after the last.
            if let Some(progress) = pending {
                emit(progress);
            }
            Ok(())
        });
        let items = std::mem::take(
            &mut *shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        // A nonzero exit without per-file results (no such remote, not
        // initialized) keeps Git's own error and stderr.
        if let Err(error) = run
            && !items.iter().any(|item| !item.success)
        {
            return Err(error);
        }
        let failed: Vec<&AnnexItem> = items.iter().filter(|item| !item.success).collect();
        if !failed.is_empty() {
            let detail = failure_detail(&failed);
            return Err(Error::new(ErrorKind::Git(GitFailure::new(
                label,
                GitFailureId::CommandFailed,
                Some(1),
                Vec::new(),
                detail.clone().into_bytes(),
                Some(detail),
            ))));
        }
        let done = items
            .iter()
            .filter_map(|item| item.file.as_deref())
            .collect::<Vec<_>>();
        Ok(CommandOutput {
            command: label.to_string(),
            stdout: if !done.is_empty() {
                done.join("\n")
            } else if !items.is_empty() {
                format!("{} done", items.len())
            } else {
                "Nothing to do".to_string()
            },
            stderr: String::new(),
            exit_code: Some(0),
        })
    }

    /// A user-started command: runs until done or cancelled, since git-annex
    /// prints nothing while one file transfers or one tree is checked out.
    fn run_annex_plain(&self, args: &[&str], extra: &[&str], label: &str) -> Result<CommandOutput> {
        let mut cmd = self.git_annex(args);
        cmd.args(extra);
        run_git_with_output_until_done(cmd, label)
    }

    /// `git annex pull` / `push` arrived in 10.20230626; older git-annex
    /// (Ubuntu 22.04, Debian 11) does the same with a one-way `sync`.
    fn run_annex_one_way(&self, pull: bool, content: bool) -> Result<CommandOutput> {
        let (verb, other_way) = if pull {
            ("pull", "--no-push")
        } else {
            ("push", "--no-pull")
        };
        let content = if content { "--content" } else { "--no-content" };
        let label = format!("git annex {verb}");
        match self.run_annex_plain(&[verb, content], &[], &label) {
            Err(error) if unknown_annex_command(&error, verb) => {
                self.run_annex_plain(&["sync", other_way, "--no-commit", content], &[], &label)
            }
            result => result,
        }
    }

    pub(super) fn run_annex_command(&self, command: &LargeFileCommand) -> Result<CommandOutput> {
        let result = self.run_annex_command_inner(command);
        if command.restages_after() {
            // Also after a failure or a cancel, which kill git-annex before its
            // own restage. A fresh operation keeps a cancelled one's flag from
            // refusing this run; its output is not worth reporting.
            let quiet = gitcomet_core::git_operation::GitOperationContext::new(
                "git annex restage",
                |_, _| {},
            );
            let _scope = gitcomet_core::git_operation::attach(&quiet);
            // Nobody can cancel this one, so it keeps the silence deadline.
            let _ = run_git_with_output(self.git_annex(&["restage"]), "git annex restage");
        }
        result
    }

    fn run_annex_command_inner(&self, command: &LargeFileCommand) -> Result<CommandOutput> {
        use LargeFileCommand as C;
        let os = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        let content = |content: bool| if content { "--content" } else { "--no-content" };
        match command {
            C::AnnexGet { paths, from } => {
                let mut args = os(&["get", "--json-progress"]);
                if let Some(from) = from {
                    validate_name(from, "remote")?;
                    args.push(format!("--from={from}").into());
                }
                self.run_annex_json(&args, paths, "git annex get")
            }
            C::AnnexGetKeys { keys } => {
                let mut outputs = Vec::new();
                for key in keys {
                    gitcomet_core::annex::parse_key(key)
                        .ok_or_else(|| backend(format!("not a git-annex key: {key}")))?;
                    let mut args = os(&["get", "--json-progress"]);
                    args.push(format!("--key={key}").into());
                    outputs.push(self.run_annex_json(&args, &[], "git annex get")?.stdout);
                }
                Ok(CommandOutput {
                    command: "git annex get".to_string(),
                    stdout: outputs.join("\n"),
                    stderr: String::new(),
                    exit_code: Some(0),
                })
            }
            C::AnnexDrop { paths, from, force } => {
                let mut args = os(&["drop"]);
                if let Some(from) = from {
                    validate_name(from, "remote")?;
                    args.push(format!("--from={from}").into());
                }
                if *force {
                    args.push("--force".into());
                }
                self.run_annex_json(&args, paths, "git annex drop")
            }
            C::AnnexCopy { paths, to } | C::AnnexMove { paths, to } => {
                validate_name(to, "remote")?;
                let verb = if matches!(command, C::AnnexCopy { .. }) {
                    "copy"
                } else {
                    "move"
                };
                let mut args = os(&[verb, "--json-progress"]);
                args.push(format!("--to={to}").into());
                self.run_annex_json(&args, paths, &format!("git annex {verb}"))
            }
            C::AnnexUnlock { paths } => {
                self.run_annex_json(&os(&["unlock"]), paths, "git annex unlock")
            }
            C::AnnexLock { paths } => self.run_annex_json(&os(&["lock"]), paths, "git annex lock"),
            C::AnnexAdd { paths } => self.run_annex_json(&os(&["add"]), paths, "git annex add"),
            C::AnnexPull { content } => self.run_annex_one_way(true, *content),
            C::AnnexPush { content } => self.run_annex_one_way(false, *content),
            C::AnnexSync { content: c } => {
                self.run_annex_plain(&["sync", "--no-commit", content(*c)], &[], "git annex sync")
            }
            C::AnnexInit => self.run_annex_plain(&["init"], &[], "git annex init"),
            C::AnnexAdjust { mode } => {
                self.run_annex_plain(&["adjust", mode.as_arg()], &[], "git annex adjust")
            }
            C::AnnexLeaveAdjusted { base } => {
                validate_ref_like_arg(base, "branch")?;
                let mut cmd = self.git_workdir_cmd();
                cmd.args(["checkout", base.as_str()]);
                run_git_with_output_until_done(cmd, "git checkout")
            }
            C::AnnexEnableRemote { name, params } => {
                validate_name(name, "remote")?;
                validate_remote_params(params)?;
                let mut extra = vec![name.as_str()];
                extra.extend(params.iter().map(String::as_str));
                self.run_annex_plain(&["enableremote"], &extra, "git annex enableremote")
            }
            C::AnnexInitRemote {
                name,
                special_type,
                params,
            } => {
                validate_name(name, "remote name")?;
                validate_name(special_type, "remote type")?;
                if name.contains(char::is_whitespace) {
                    return Err(backend("a special remote name cannot contain spaces"));
                }
                validate_remote_params(params)?;
                let type_arg = format!("type={special_type}");
                let mut extra = vec![name.as_str(), type_arg.as_str()];
                extra.extend(params.iter().map(String::as_str));
                self.run_annex_plain(&["initremote"], &extra, "git annex initremote")
            }
            C::AnnexTrust { repository, trust } => {
                validate_name(repository, "repository")?;
                self.run_annex_plain(&[trust.as_arg()], &[repository.as_str()], "git annex trust")
            }
            C::AnnexDescribe {
                repository,
                description,
            } => {
                validate_name(repository, "repository")?;
                if description.trim().is_empty() {
                    return Err(backend("a description cannot be empty"));
                }
                self.run_annex_plain(
                    &["describe"],
                    &[repository.as_str(), description.as_str()],
                    "git annex describe",
                )
            }
            C::AnnexNumcopies { copies } => {
                if *copies == 0 {
                    return Err(backend("numcopies must be at least 1"));
                }
                let copies = copies.to_string();
                self.run_annex_plain(&["numcopies"], &[copies.as_str()], "git annex numcopies")
            }
            C::AnnexFsck => self.run_annex_plain(&["fsck", "--fast"], &[], "git annex fsck"),
            C::AnnexRestage => self.run_annex_plain(&["restage"], &[], "git annex restage"),
            C::AnnexDropUnused { force } => {
                // `dropunused` works on the numbers the last `unused` wrote, so
                // refresh them first rather than trust an older listing.
                self.run_annex_plain(&["unused", "--quiet"], &[], "git annex unused")?;
                let mut args = os(&["dropunused"]);
                if *force {
                    args.push("--force".into());
                }
                args.push("all".into());
                self.run_annex_json(&args, &[], "git annex dropunused")
            }
            C::AnnexWebapp => self.spawn_annex_webapp(),
            C::AnnexStopAssistant => {
                self.run_annex_plain(&["assistant", "--stop"], &[], "git annex assistant --stop")
            }
            _ => Err(backend(format!(
                "{} is not a git-annex command",
                command.label()
            ))),
        }
    }

    /// Repositories and numcopies for the repo summary, read from git-annex's
    /// logs. Running `git annex info` instead would merge fetched branches,
    /// write remotes' uuids into config and probe URL remotes over the network.
    pub(super) fn annex_repositories(
        &self,
        repo: &gix::Repository,
    ) -> (
        Vec<gitcomet_core::large_files::AnnexRepository>,
        Option<u32>,
    ) {
        let config = repo.config_snapshot();
        let here = config
            .string("annex.uuid")
            .map(|uuid| String::from_utf8_lossy(&uuid).into_owned());
        let repositories = repositories_from_logs(
            &annex_log(repo, "uuid.log"),
            &annex_log(repo, "trust.log"),
            &remote_names(&config),
            &special_remotes(repo),
            here.as_deref(),
        );
        (
            repositories,
            parse_numcopies_log(&annex_log(repo, "numcopies.log")),
        )
    }

    /// Starts `git annex webapp` without waiting: it serves the webapp and
    /// runs the assistant until stopped, so it outlives the command runner's
    /// timeout. A thread reaps it when it exits.
    fn spawn_annex_webapp(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_annex(&["webapp"]);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| backend(format!("could not start git annex webapp: {e}")))?;
        std::thread::Builder::new()
            .name("git-annex-webapp".into())
            .spawn(move || {
                let _ = child.wait();
            })
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        Ok(CommandOutput {
            command: "git annex webapp".to_string(),
            stdout: "Started the git-annex webapp; it opens in your browser.".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        })
    }

    pub(super) fn annex_unused_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<AnnexUnused> {
        parse_unused(&run_git_background_capture(
            self.git_annex(&["unused", "--json"]),
            "git annex unused",
            cancellation,
        )?)
    }

    pub(super) fn annex_whereis_impl(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<AnnexWhereis> {
        let mut cmd = self.git_annex(&["whereis", "--json"]);
        cmd.arg("--").arg(path);
        parse_whereis(&run_git_background_capture(
            cmd,
            "git annex whereis",
            cancellation,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_items_and_refusals() {
        let progress: serde_json::Value = serde_json::from_str(
            r#"{"action":{"command":"copy","file":"big.bin","note":"to backup..."},"byte-progress":32752,"percent-progress":"16.38%","total-size":200000}"#,
        )
        .unwrap();
        let progress = parse_progress(&progress).unwrap();
        assert_eq!(progress.summary(), "annex copy big.bin · 32.8 KB of 200 KB");

        let refused: serde_json::Value = serde_json::from_str(
            r#"{"command":"drop","error-messages":[],"file":"big.bin","note":"unsafe\nCould not verify the existence of the 1 necessary copy.\n(Use --force to override this check, or adjust numcopies.)","success":false}"#,
        )
        .unwrap();
        let item = parse_item(&refused).unwrap();
        assert!(!item.success);
        let detail = failure_detail(&[&item]);
        assert!(
            detail.starts_with("big.bin: unsafe Could not verify"),
            "{detail}"
        );
        assert!(detail.contains("Hint: git-annex keeps content"), "{detail}");
    }

    #[test]
    fn repositories_come_from_the_uuid_and_trust_logs() {
        let uuid_log = "aaa laptop timestamp=5s\n\
                        bbb backup timestamp=5s\n\
                        ccc usb drive timestamp=5s\n\
                        ddd gone timestamp=5s\n\
                        aaa old name timestamp=1s\n";
        let trust_log = "ccc 0 timestamp=5s\n\
                         ddd X timestamp=5s\n\
                         00000000-0000-0000-0000-000000000001 X timestamp=5s\n";
        let remotes = [KnownRemote {
            uuid: "bbb".into(),
            name: "backup".into(),
            special_type: Some("directory".into()),
            trust_level: None,
        }];
        let types = parse_remote_log("ccc name=usb type=rsync timestamp=1s\n");
        let repos = repositories_from_logs(uuid_log, trust_log, &remotes, &types, Some("aaa"));
        let summary: Vec<_> = repos
            .iter()
            .map(|repo| (repo.uuid.as_str(), repo.description.as_str(), repo.trust))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "00000000-0000-0000-0000-000000000002",
                    "bittorrent",
                    AnnexTrust::Semitrusted
                ),
                ("aaa", "laptop", AnnexTrust::Semitrusted),
                ("bbb", "[backup]", AnnexTrust::Semitrusted),
                ("ccc", "usb drive", AnnexTrust::Untrusted),
            ],
            "dead ones are left out; the newest description wins"
        );
        assert!(repos[1].here);
        assert_eq!(repos[2].display_name(), "backup");
        assert_eq!(repos[2].special_type.as_deref(), Some("directory"));
        // Not enabled here, so only remote.log knows its type and name.
        assert_eq!(repos[3].special_type.as_deref(), Some("rsync"));
        assert_eq!(repos[3].special_name.as_deref(), Some("usb"));
    }

    #[test]
    fn numcopies_log_takes_the_newest_value_in_either_format() {
        assert_eq!(parse_numcopies_log("1790106673s 2\n"), Some(2));
        assert_eq!(
            parse_numcopies_log("3 timestamp=10s\n1790106673s 2\n"),
            Some(2)
        );
        assert_eq!(parse_numcopies_log(""), None);
    }

    #[test]
    fn remote_log_keeps_the_newest_type_per_uuid() {
        let log = "u1 encryption=none name=usb type=directory timestamp=1790075912s\n\
                   u1 name=usb type=rsync timestamp=1790075999.25s\n\
                   u2 name=cloud type=S3 bucket=x timestamp=5s\n\
                   u3 name=gitrepo type=git location=x timestamp=5s\n\
                   u4 name=typeless timestamp=5s\n\
                   \n";
        let types = parse_remote_log(log);
        let kind = |uuid: &str| types.get(uuid).map(|s| s.special_type.as_str());
        assert_eq!(kind("u1"), Some("rsync"));
        assert_eq!(kind("u2"), Some("S3"));
        assert_eq!(types["u2"].name.as_deref(), Some("cloud"));
        assert!(!types.contains_key("u3"), "type=git is a git repository");
        assert!(!types.contains_key("u4"));
    }

    #[test]
    fn parses_unused_lists_in_number_order() {
        let json = r#"{"bad-list":{"1":"SHA256E-s9--bad"},"command":"unused","success":true,"tmp-list":{},"unused-list":{"10":"SHA256E-s4--ten.bin","2":"SHA256E-s6--two.bin"}}"#;
        let unused = parse_unused(json).unwrap();
        let keys: Vec<_> = unused
            .entries
            .iter()
            .map(|entry| (entry.key.as_str(), entry.kind))
            .collect();
        assert_eq!(
            keys,
            [
                ("SHA256E-s6--two.bin", AnnexUnusedKind::Unused),
                ("SHA256E-s4--ten.bin", AnnexUnusedKind::Unused),
                ("SHA256E-s9--bad", AnnexUnusedKind::Bad),
            ]
        );
        assert_eq!(unused.known_bytes(), 19);
        assert!(parse_unused("").is_err());
    }

    /// A killed assistant leaves `daemon.pid`; only its held lock file says
    /// it still runs.
    #[cfg(windows)]
    #[test]
    fn assistant_on_windows_counts_as_running_only_while_its_lock_is_held() {
        use std::os::windows::fs::OpenOptionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("daemon.pid"), "4242\n").unwrap();
        assert!(!assistant_running(dir.path()), "a stale pid file alone");
        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(dir.path().join("daemon.pid.4242.lck"))
            .unwrap();
        assert!(assistant_running(dir.path()));
        drop(held);
        assert!(
            !assistant_running(dir.path()),
            "left behind by a killed assistant"
        );
    }

    #[test]
    fn parses_whereis() {
        let json = r#"{"command":"whereis","key":"K","success":true,"untrusted":[],"whereis":[{"description":"lap","here":true,"urls":[],"uuid":"u1"},{"description":"[backup]","here":false,"urls":[],"uuid":"u2"}]}"#;
        let whereis = parse_whereis(json).unwrap();
        assert_eq!(whereis.key, "K");
        assert_eq!(whereis.copies.len(), 2);
        assert!(whereis.copies[0].here);
    }
}
