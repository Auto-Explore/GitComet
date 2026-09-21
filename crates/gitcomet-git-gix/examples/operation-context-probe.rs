//! Compare raw Git, the backend, and attached operation context on disposable
//! fixtures. `--diagnostics` captures command stages and backend operation counts;
//! leave it off for latency acceptance. Fixture creation is outside all samples.
use gitcomet_core::git_operation::{self, GitOperationContext};
use gitcomet_core::git_ops_trace;
use gitcomet_core::process::git_command;
use gitcomet_core::services::{GitBackend, RemoteUrlKind};
use gitcomet_git_gix::{GixBackend, command_trace};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

const URL: &str = "https://example.invalid/repo";
const STATUS_ARGS: &[&str] = &["status", "--porcelain=v1", "-z", "--untracked-files=all"];

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn command(&self, repo: &Path) -> Command {
        let mut command = git_command();
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.config)
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env("GNUPGHOME", self.root.join("gnupg"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .args([
                "--no-optional-locks",
                "-c",
                "protocol.file.allow=always",
                "-C",
            ])
            .arg(repo)
            .stdin(Stdio::null());
        command
    }

    fn git(&self, repo: &Path, args: &[&str]) -> Vec<u8> {
        let output = self.command(repo).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn init(&self, repo: &Path) {
        fs::create_dir_all(repo).unwrap();
        self.git(repo, &["init", "-q", "-b", "main"]);
        let config = repo.join(".git/config");
        let mut text = fs::read_to_string(&config).unwrap();
        text.push_str("\n[user]\n name = Probe\n email = probe@example.invalid\n[core]\n autocrlf = false\n[commit]\n gpgsign = false\n");
        fs::write(config, text).unwrap();
    }

    fn new(kind: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let config = root.join("empty.gitconfig");
        fs::write(&config, "").unwrap();
        for name in ["home", "xdg", "gnupg"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        // Match the raw command's environment in every backend subprocess.
        gitcomet_git_gix::install_test_git_command_environment(
            config.clone(),
            root.join("home"),
            root.join("xdg"),
            root.join("gnupg"),
        );
        let fixture = Self {
            _directory: directory,
            root,
            config,
        };
        let repo = fixture.root.join("repo");
        fixture.init(&repo);
        fs::write(repo.join("file.txt"), "base\n").unwrap();
        if kind == "lfs" {
            fixture.git(
                &repo,
                &["config", "filter.lfs.process", "git-lfs filter-process"],
            );
            fixture.git(&repo, &["config", "filter.lfs.required", "true"]);
            fs::write(
                repo.join(".gitattributes"),
                "*.bin filter=lfs diff=lfs merge=lfs -text\n",
            )
            .unwrap();
            for index in 0..60 {
                fs::write(
                    repo.join(format!("asset-{index:03}.bin")),
                    vec![index as u8; 1024],
                )
                .unwrap();
            }
        }
        fixture.git(&repo, &["add", "."]);
        fixture.git(&repo, &["commit", "-qm", "base"]);
        if kind == "submodule" {
            let seed = fixture.root.join("seed");
            fixture.init(&seed);
            fs::write(seed.join("child.txt"), "child\n").unwrap();
            fixture.git(&seed, &["add", "."]);
            fixture.git(&seed, &["commit", "-qm", "seed"]);
            fixture.git(
                &repo,
                &["submodule", "add", "-q", seed.to_str().unwrap(), "child"],
            );
            fixture.git(&repo, &["commit", "-qam", "submodule"]);
            fs::write(repo.join("child/child.txt"), "dirty child\n").unwrap();
        }
        fixture.git(&repo, &["remote", "add", "origin", URL]);
        fs::write(repo.join("file.txt"), "changed\n").unwrap();
        if kind == "lfs" {
            fs::write(repo.join("asset-000.bin"), "dirty LFS asset\n").unwrap();
        }
        fixture
    }
}

fn command_json(command: command_trace::CommandTiming) -> Value {
    json!({"label": command.label, "milliseconds": command.elapsed.as_secs_f64() * 1000.0,
           "stages": command.stages.into_iter().map(|(stage, elapsed)|
               json!({"stage": stage, "milliseconds": elapsed.as_secs_f64() * 1000.0})).collect::<Vec<_>>()})
}

fn main() {
    // gix reads configuration in-process, while raw Git and the backend's
    // subprocesses have per-command environments. Start the probe itself with
    // an empty home/config too, without mutating a multithreaded process's env.
    const CHILD: &str = "--isolated-probe-child";
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.first().is_none_or(|argument| argument != CHILD) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let config = root.join("empty.gitconfig");
        fs::write(&config, "").unwrap();
        for name in ["home", "xdg", "gnupg"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        let status = Command::new(std::env::current_exe().unwrap())
            .arg(CHILD)
            .args(arguments)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", config)
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("xdg"))
            .env("GNUPGHOME", root.join("gnupg"))
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap();
        drop(directory);
        std::process::exit(status.code().unwrap_or(1));
    }
    let mut samples = 35usize;
    let mut warmups = 5usize;
    let mut diagnostics = false;
    let mut fixture_kind = String::from("plain");
    let mut profile = String::from("unspecified");
    let mut output = None;
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => samples = args.next().expect("sample count").parse().unwrap(),
            "--warmups" => warmups = args.next().expect("warmup count").parse().unwrap(),
            "--diagnostics" => diagnostics = true,
            "--fixture" => fixture_kind = args.next().expect("plain|lfs|submodule"),
            "--profile-label" => profile = args.next().expect("build profile label"),
            "--output" => output = Some(PathBuf::from(args.next().expect("JSON output path"))),
            _ => panic!("unknown argument: {arg}"),
        }
    }
    assert!(samples > 0 && samples <= 10_000 && warmups <= 10_000);
    assert!(matches!(
        fixture_kind.as_str(),
        "plain" | "lfs" | "submodule"
    ));
    let fixture = Fixture::new(&fixture_kind);
    let path = fixture.root.join("repo");
    let repo = GixBackend.open(&path).unwrap();
    // Verify equivalent work outside the samples. In particular, the plain
    // file alone must not let a missing LFS/submodule status go unnoticed.
    let mut expected = vec![PathBuf::from("file.txt")];
    match fixture_kind.as_str() {
        "lfs" => expected.push(PathBuf::from("asset-000.bin")),
        "submodule" => expected.push(PathBuf::from("child")),
        _ => {}
    }
    expected.sort();
    let raw_status = fixture.git(&path, STATUS_ARGS);
    let mut raw_paths: Vec<_> = raw_status
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            assert_eq!(entry[0], b' ', "fixture must have no staged changes");
            PathBuf::from(std::str::from_utf8(&entry[3..]).unwrap())
        })
        .collect();
    raw_paths.sort();
    assert_eq!(raw_paths, expected, "raw Git fixture status");
    let status = repo.status().unwrap();
    assert!(status.staged.is_empty());
    let mut backend_paths: Vec<_> = status
        .unstaged
        .iter()
        .map(|file| file.path.clone())
        .collect();
    backend_paths.sort();
    assert_eq!(backend_paths, expected, "backend fixture status");
    let operation = GitOperationContext::new("probe", |_, _| {});
    let modes = ["raw-git", "backend", "operation-context"];
    let mut results = Vec::new();
    for action in ["remote-set-url", "status"] {
        let mut measurements: [Vec<Value>; 3] = std::array::from_fn(|_| Vec::new());
        for round in 0..warmups + samples {
            // Rotate and reverse order to distribute cache/order effects.
            for position in 0..3 {
                let mode = (round
                    + if round % 2 == 0 {
                        position
                    } else {
                        2 - position
                    })
                    % 3;
                let _scope = (mode == 2).then(|| git_operation::attach(&operation));
                let invoke = || {
                    if mode == 0 {
                        let args: &[&str] = if action == "remote-set-url" {
                            &["remote", "set-url", "--", "origin", URL]
                        } else {
                            STATUS_ARGS
                        };
                        let _ = fixture.git(&path, args);
                    } else if action == "remote-set-url" {
                        repo.set_remote_url_with_output("origin", URL, RemoteUrlKind::Fetch)
                            .unwrap();
                    } else {
                        let status = repo.status().unwrap();
                        assert!(status.staged.is_empty());
                        assert!(!status.unstaged.is_empty());
                    }
                };
                let started = Instant::now();
                let (commands, backend_status_calls) = if diagnostics {
                    let _capture = git_ops_trace::capture();
                    let (_, commands) = command_trace::capture(invoke);
                    (commands, Some(git_ops_trace::snapshot().status.calls))
                } else {
                    invoke();
                    (Vec::new(), None)
                };
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                if round >= warmups {
                    measurements[mode].push(json!({"milliseconds": elapsed,
                        "backend_status_calls": backend_status_calls,
                        "raw_git_commands": (mode == 0).then_some(1),
                        "wrapped_commands": (diagnostics && mode != 0).then_some(commands.len()),
                        "command_timings": commands.into_iter().map(command_json).collect::<Vec<_>>() }));
                }
            }
        }
        for (mode, raw) in modes.iter().zip(measurements) {
            let mut times: Vec<_> = raw
                .iter()
                .map(|sample| sample["milliseconds"].as_f64().unwrap())
                .collect();
            times.sort_by(f64::total_cmp);
            let median = (times[(samples - 1) / 2] + times[samples / 2]) / 2.0;
            results.push(
                json!({"operation": action, "mode": mode, "median_ms": median,
                "p95_ms": times[samples - samples / 20 - 1], "samples": raw}),
            );
        }
    }
    let report = json!({"schema_version": 1, "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH, "profile": profile, "fixture": fixture_kind,
        "verified_status_paths": expected,
        "diagnostics": diagnostics, "warmups": warmups,
        "git": String::from_utf8(fixture.git(&path, &["--version"])).unwrap().trim(),
        "results": results});
    let json = serde_json::to_string_pretty(&report).unwrap() + "\n";
    if let Some(output) = output {
        fs::write(output, &json).unwrap();
    }
    print!("{json}");
}
