use super::*;
use gitcomet_core::error::ErrorKind;
use gitcomet_core::services::{CommandOutput, Result};
use std::fs::{File, OpenOptions};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

const FIXTURE_WAIT_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
struct FetchProcesses {
    group: u32,
    transport: u32,
    descendant: u32,
}

#[derive(Debug)]
struct ProcessState {
    pid: u32,
    group: u32,
    state: char,
}

fn process_states() -> Vec<ProcessState> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,pgid=,stat="])
        .output()
        .expect("inspect fixture process group");
    assert!(output.status.success(), "fixture process inspection failed");
    String::from_utf8(output.stdout)
        .expect("fixture process state output")
        .lines()
        .map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields
                .next()
                .expect("fixture process id")
                .parse()
                .expect("numeric fixture process id");
            let group = fields
                .next()
                .expect("fixture process group")
                .parse()
                .expect("numeric fixture process group");
            let state = fields
                .next()
                .and_then(|value| value.chars().next())
                .expect("fixture process state");
            assert!(fields.next().is_none(), "unexpected fixture process fields");
            ProcessState { pid, group, state }
        })
        .collect()
}

struct PausedFetch {
    cancellation: CancellationToken,
    started: File,
    release: File,
    ready: Receiver<std::io::Result<String>>,
    completed: Receiver<Result<CommandOutput>>,
    reader: Option<JoinHandle<()>>,
    worker: Option<JoinHandle<()>>,
    processes: Option<FetchProcesses>,
}

impl PausedFetch {
    fn start(root: &Path, local: &Path, remote: &Path, resumed: &Path) -> Self {
        let started_path = root.join("started");
        let release_path = root.join("release");
        let output = Command::new("mkfifo")
            .arg(&started_path)
            .arg(&release_path)
            .output()
            .expect("create fixture transport gates");
        assert!(
            output.status.success(),
            "fixture transport gate creation failed"
        );
        let started = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&started_path)
            .expect("open fixture start gate");
        let release = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&release_path)
            .expect("open fixture release gate");
        let descendant = "set -eu\n\
            group=$(ps -o pgid= -p \"$$\")\n\
            printf '%s %s %s\\n' \"$group\" \"$PPID\" \"$$\" > \"$1\"\n\
            IFS= read -r release < \"$2\"\n\
            touch \"$3\"\n\
            exec git-upload-pack \"$4\"\n";
        install_remote(
            local,
            &root.join("transport.sh"),
            &format!(
                "set -eu\nexec 3<&0\nsh -c {} fixture {} {} {} {} <&3 &\nwait \"$!\"\n",
                shell_quote(descendant),
                shell_path(&started_path),
                shell_path(&release_path),
                shell_path(resumed),
                shell_path(remote),
            ),
        );
        let opened = GixBackend.open(local).expect("open repository");
        let started_reader = started.try_clone().expect("clone fixture start gate");
        let (ready_sender, ready) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(started_reader)
                .read_line(&mut line)
                .map(|_| line);
            let _ = ready_sender.send(result);
        });
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (completed_sender, completed) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result = opened.fetch_all_background(false, &worker_cancellation);
            let _ = completed_sender.send(result);
        });
        Self {
            cancellation,
            started,
            release,
            ready,
            completed,
            reader: Some(reader),
            worker: Some(worker),
            processes: None,
        }
    }

    fn wait_for_start(&mut self) -> FetchProcesses {
        let line = self
            .ready
            .recv_timeout(FIXTURE_WAIT_LIMIT)
            .expect("background fetch descendant must reach the closed gate")
            .expect("read fixture start handshake");
        let ids: Vec<u32> = line
            .split_whitespace()
            .map(|value| value.parse().expect("numeric fixture handshake id"))
            .collect();
        let [group, transport, descendant] = ids.as_slice() else {
            panic!("expected process group, transport and descendant ids: {line:?}");
        };
        assert!(
            *group > 0 && *transport > 0 && *descendant > 0,
            "fixture process ids must be positive"
        );
        let processes = FetchProcesses {
            group: *group,
            transport: *transport,
            descendant: *descendant,
        };
        self.processes = Some(processes);
        self.reader
            .take()
            .expect("fixture start reader")
            .join()
            .expect("fixture start reader stopped");
        processes
    }

    fn wait_for_completion(&mut self) -> Result<CommandOutput> {
        let result = self
            .completed
            .recv_timeout(FIXTURE_WAIT_LIMIT)
            .expect("cancellation must finish while the transport gate remains closed");
        self.worker
            .take()
            .expect("fixture fetch worker")
            .join()
            .expect("fixture fetch worker stopped");
        result
    }

    fn release(&mut self) {
        writeln!(self.release, "release").expect("release fixture transport gate");
    }
}

impl Drop for PausedFetch {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(processes) = self.processes {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", processes.group)])
                .output();
        }
        let _ = writeln!(self.release, "release");
        let _ = writeln!(self.started, "stopped");
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn background_fetch_cancellation_reaps_the_process_tree_before_returning() {
    let root = tempfile::tempdir().expect("fixture directory");
    let remote = repository(root.path(), "remote");
    let first = commit(&remote, "first");
    let local = repository(root.path(), "local");
    git(
        &local,
        &[
            "fetch",
            &remote_url(&remote),
            "refs/heads/main:refs/remotes/origin/main",
        ],
    );
    let latest = commit(&remote, "second");
    let resumed = root.path().join("resumed");
    let mut fetch = PausedFetch::start(root.path(), &local, &remote, &resumed);
    let processes = fetch.wait_for_start();
    let running = process_states();
    for pid in [processes.group, processes.transport, processes.descendant] {
        assert!(
            running.iter().any(|process| {
                process.pid == pid && process.group == processes.group && process.state != 'Z'
            }),
            "fixture process {pid} must be running in the fetch group: {running:?}"
        );
    }
    assert_ne!(processes.group, processes.transport);
    assert_ne!(processes.transport, processes.descendant);
    assert!(!resumed.exists());

    fetch.cancellation.cancel();
    let error = fetch
        .wait_for_completion()
        .expect_err("running background fetch must return cancellation");

    assert!(matches!(error.kind(), ErrorKind::Cancelled));
    let remaining = process_states();
    assert!(
        remaining
            .iter()
            .all(|process| process.pid != processes.group),
        "the Git process must be reaped before cancellation returns: {remaining:?}"
    );
    assert!(
        remaining.iter().all(|process| {
            process.group != processes.group || matches!(process.state, 'Z' | 'X')
        }),
        "the cancelled fetch must leave no live descendants: {remaining:?}"
    );
    assert_eq!(git(&local, &["rev-parse", "origin/main"]), first);

    fetch.release();

    assert!(!resumed.exists());
    assert_eq!(git(&local, &["rev-parse", "origin/main"]), first);

    GixBackend
        .open(&local)
        .expect("reopen repository")
        .fetch_all_background(false, &CancellationToken::new())
        .expect("the released transport must permit a later fetch");
    assert!(resumed.exists());
    assert_eq!(git(&local, &["rev-parse", "origin/main"]), latest);
}
