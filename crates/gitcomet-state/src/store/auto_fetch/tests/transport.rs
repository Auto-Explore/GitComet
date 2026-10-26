use super::*;
use crate::model::{CommandSource, GitLogTagFetchMode, Loadable};
use crate::msg::{FetchMsg, Msg};
use crate::store::{AppStore, StoreEvent};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use std::sync::mpsc;
use std::thread::JoinHandle;

const HANG_GUARD: Duration = Duration::from_secs(10);

struct GatedStore {
    store: AppStore,
    events: smol::channel::Receiver<StoreEvent>,
    extra_stores: Vec<AppStore>,
    root: tempfile::TempDir,
    clone: PathBuf,
    latest: String,
    first: String,
    repo_id: RepoId,
    started: File,
    release: File,
    ready: mpsc::Receiver<std::io::Result<String>>,
    reader: Option<JoinHandle<()>>,
    group: Option<u32>,
    _session: crate::session::TestSessionFilePathGuard,
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn process_states() -> Vec<(u32, u32, char)> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,pgid=,stat="])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next().unwrap().parse().unwrap();
            let group = fields.next().unwrap().parse().unwrap();
            let state = fields.next().unwrap().chars().next().unwrap();
            assert!(fields.next().is_none());
            (pid, group, state)
        })
        .collect()
}

impl GatedStore {
    fn new() -> Self {
        let (root, remote, clone, latest) = remote_fixture();
        let first = git(&clone, &["rev-parse", "HEAD"]);
        let _session = crate::session::push_test_session_file_path_override(Some(
            root.path().join("session.json"),
        ));
        for name in ["started", "release"] {
            assert!(
                Command::new("mkfifo")
                    .arg(root.path().join(name))
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let open_gate = |name| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.path().join(name))
                .unwrap()
        };
        let started = open_gate("started");
        let release = open_gate("release");
        let script = root.path().join("transport.sh");
        let remote = shell_quote(remote.to_str().unwrap());
        let root_path = shell_quote(root.path().to_str().unwrap());
        std::fs::write(
            &script,
            format!(
                "set -eu\n\
                 root={root_path}\n\
                 if mkdir \"$root/first-attempt\" 2>/dev/null; then\n\
                   group=$(ps -o pgid= -p \"$$\")\n\
                   printf '%s\\n' \"$group\" > \"$root/group\"\n\
                   exec 3<&0\n\
                   sh -c 'printf \"%s\\n\" ready > \"$1/started\"; IFS= read -r release < \"$1/release\"; exec git-upload-pack \"$2\"' fixture \"$root\" {remote} <&3 &\n\
                   wait \"$!\"\n\
                 else\n\
                   group=$(cat \"$root/group\")\n\
                   if ps -axo pid=,pgid=,stat= | awk -v group=\"$group\" '$1 == group || ($2 == group && $3 !~ /^[ZX]/) {{ found = 1 }} END {{ exit !found }}'; then\n\
                     touch \"$root/overlap\"\n\
                   fi\n\
                   exec git-upload-pack {remote}\n\
                 fi\n"
            ),
        )
        .unwrap();
        git(
            &clone,
            &[
                "remote",
                "set-url",
                "origin",
                "ssh://fixture.invalid/repository",
            ],
        );
        git(
            &clone,
            &[
                "config",
                "core.sshCommand",
                &format!("sh {}", shell_quote(script.to_str().unwrap())),
            ],
        );
        git(&clone, &["config", "ssh.variant", "ssh"]);
        let (store, events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
        Self::open(&store, &events, &clone);
        let repo_id = store.snapshot().active_repo.unwrap();
        let (send, ready) = mpsc::channel();
        let started_reader = started.try_clone().unwrap();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(started_reader)
                .read_line(&mut line)
                .map(|_| line);
            let _ = send.send(result);
        });
        Self {
            store,
            events,
            extra_stores: Vec::new(),
            root,
            clone,
            latest,
            first,
            repo_id,
            started,
            release,
            ready,
            reader: Some(reader),
            group: None,
            _session,
        }
    }

    fn open(store: &AppStore, events: &smol::channel::Receiver<StoreEvent>, path: &Path) {
        store.disable_repo_monitors_for_test();
        store.dispatch(Msg::SetGitLogSettings {
            show_history_tags: false,
            tag_fetch_mode: GitLogTagFetchMode::Disabled,
            verify_commit_signatures: false,
        });
        store.dispatch(Msg::OpenRepo(path.to_path_buf()));
        wait_for_store(store, events, |state| {
            state.repos.first().is_some_and(|repo| {
                matches!(repo.open, Loadable::Ready(())) && !repo.loads_in_flight.any_in_flight()
            })
        });
    }

    fn start(&mut self) -> CancellationToken {
        let mut settings = RemoteSettings::default();
        settings.auto_fetch.enabled = true;
        self.store.dispatch(Msg::SetRemoteSettings(settings));
        assert_eq!(
            self.ready.recv_timeout(HANG_GUARD).unwrap().unwrap(),
            "ready\n"
        );
        self.reader.take().unwrap().join().unwrap();
        let group = std::fs::read_to_string(self.root.path().join("group"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(group > 0);
        self.group = Some(group);
        let processes = process_states();
        assert!(
            processes
                .iter()
                .filter(|(_, pgid, _)| *pgid == group)
                .count()
                >= 3
        );
        self.store
            .msg_tx
            .auto_fetch
            .inner
            .state
            .lock()
            .unwrap()
            .running
            .as_ref()
            .unwrap()
            .cancellation
            .clone()
    }

    fn barrier(&self) {
        self.store
            .barrier_for_test()
            .recv_timeout(HANG_GUARD)
            .unwrap();
    }

    fn resist_termination(&self) {
        let script = self.root.path().join("transport.sh");
        let contents = std::fs::read_to_string(&script).unwrap();
        std::fs::write(script, format!("trap '' TERM\n{contents}")).unwrap();
    }

    fn stop_coordinator(&self) {
        let coordinator = Arc::clone(&self.store.msg_tx.auto_fetch);
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            coordinator.shutdown();
            let _ = send.send(());
        });
        receive.recv_timeout(HANG_GUARD).unwrap();
        worker.join().unwrap();
    }

    fn assert_reaped(&self) {
        let group = self.group.unwrap();
        let processes: Vec<_> = process_states()
            .into_iter()
            .filter(|(pid, pgid, _)| *pid == group || *pgid == group)
            .collect();
        assert!(
            processes.iter().all(|(pid, pgid, state)| {
                *pid != group && (*pgid != group || matches!(state, 'Z' | 'X'))
            }),
            "fetch process group must be reaped: {processes:?}"
        );
        assert!(!self.root.path().join("overlap").exists());
    }
}

impl Drop for GatedStore {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{group}")])
                .output();
        }
        let _ = writeln!(self.release, "release");
        let _ = writeln!(self.started, "stopped");
        self.store.msg_tx.auto_fetch.shutdown();
        for store in self.extra_stores.iter().chain([&self.store]) {
            let _ = store.shutdown_for_test().recv_timeout(HANG_GUARD);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn foreground_fetch_reaps_automatic_transport_before_starting_its_own() {
    let mut fixture = GatedStore::new();
    let cancellation = fixture.start();
    fixture.store.dispatch(Msg::Fetch(FetchMsg::All {
        repo_id: fixture.repo_id,
    }));
    fixture.barrier();
    assert!(cancellation.is_cancelled());
    wait_for_store(&fixture.store, &fixture.events, |state| {
        state.repos[0].feedback.command_log.iter().any(|entry| {
            entry.source == CommandSource::User && entry.ok && entry.command.contains("fetch")
        })
    });
    fixture.assert_reaped();
    assert_eq!(
        git(&fixture.clone, &["rev-parse", "origin/main"]),
        fixture.latest
    );
    assert_eq!(git(&fixture.clone, &["rev-parse", "HEAD"]), fixture.first);
}

#[test]
fn disabling_automatic_fetch_cancels_and_reaps_active_transport() {
    let mut fixture = GatedStore::new();
    let cancellation = fixture.start();
    fixture
        .store
        .dispatch(Msg::SetRemoteSettings(RemoteSettings::default()));
    fixture.barrier();
    assert!(cancellation.is_cancelled());
    fixture.stop_coordinator();
    fixture.assert_reaped();
    assert_eq!(
        git(&fixture.clone, &["rev-parse", "origin/main"]),
        fixture.first
    );
}

#[test]
fn closing_the_last_window_subscription_cancels_and_reaps_active_transport() {
    let mut fixture = GatedStore::new();
    let cancellation = fixture.start();
    let (other, events) = AppStore::new_test_sharing_preferences(
        Arc::new(gitcomet_git_gix::GixBackend),
        &fixture.store,
    );
    fixture.extra_stores.push(other.clone());
    let mut settings = RemoteSettings::default();
    settings.auto_fetch.enabled = true;
    other.dispatch(Msg::SetRemoteSettings(settings));
    GatedStore::open(&other, &events, &fixture.clone);
    let other_id = other.snapshot().active_repo.unwrap();
    fixture.store.dispatch(Msg::CloseRepo {
        repo_id: fixture.repo_id,
    });
    fixture.barrier();
    assert!(!cancellation.is_cancelled());
    other.dispatch(Msg::CloseRepo { repo_id: other_id });
    other.barrier_for_test().recv_timeout(HANG_GUARD).unwrap();
    assert!(cancellation.is_cancelled());
    fixture.stop_coordinator();
    fixture.assert_reaped();
    assert_eq!(
        git(&fixture.clone, &["rev-parse", "origin/main"]),
        fixture.first
    );
}

#[test]
fn explicit_shutdown_waits_for_automatic_git_process_reaping() {
    let mut fixture = GatedStore::new();
    fixture.resist_termination();
    let cancellation = fixture.start();
    fixture.stop_coordinator();
    assert!(cancellation.is_cancelled());
    fixture.assert_reaped();
    assert_eq!(
        git(&fixture.clone, &["rev-parse", "origin/main"]),
        fixture.first
    );
}
