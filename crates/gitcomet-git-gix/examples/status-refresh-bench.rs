//! Disposable-fixture status benchmark. Build with `--features benchmarks`.
//! `init <new-directory> plain|small-lfs|dirty-lfs|mixed|large-lfs|many-lfs`
//! `run <fixture> [refresh-count]`; worker override: auto/1/4/8/production.
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant, SystemTime},
};

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init(path: &Path, kind: &str) {
    let (ordinary, assets, bytes) = match kind {
        "plain" => (4000, 0, 0),
        "small-lfs" | "dirty-lfs" => (0, 60, 1024),
        "mixed" => (4000, 60, 1024),
        "large-lfs" => (0, 60, 2 * 1024 * 1024),
        "many-lfs" => (0, 512, 1024),
        _ => panic!("unknown fixture"),
    };
    assert!(!path.exists(), "init requires a new directory");
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q"]);
    for (key, value) in [
        ("user.name", "Benchmark"),
        ("user.email", "bench@example.invalid"),
        ("core.autocrlf", "false"),
        ("commit.gpgsign", "false"),
        ("filter.lfs.process", "git-lfs filter-process"),
        ("filter.lfs.clean", "git-lfs clean -- %f"),
        ("filter.lfs.smudge", "git-lfs smudge -- %f"),
        ("filter.lfs.required", "true"),
    ] {
        git(path, &["config", key, value]);
    }
    if assets > 0 {
        fs::write(
            path.join(".gitattributes"),
            "*.bin filter=lfs diff=lfs merge=lfs -text\n",
        )
        .unwrap();
    }
    for n in 0..ordinary {
        fs::write(
            path.join(format!("plain-{n:05}.txt")),
            format!("file {n}\n").repeat(16),
        )
        .unwrap();
    }
    for n in 0..assets {
        let mut content = vec![b'x'; bytes];
        content[..8].copy_from_slice(&(n as u64).to_le_bytes());
        fs::write(path.join(format!("asset-{n:05}.bin")), content).unwrap();
    }
    git(path, &["add", "."]);
    git(path, &["commit", "-qm", "fixture"]);
    if kind == "dirty-lfs" {
        fs::write(path.join("asset-00000.bin"), b"modified\n").unwrap();
    }
    fs::write(path.join(".git/gitcomet-benchmark-fixture"), kind).unwrap();
}

fn run(path: &Path, repeats: usize) {
    let kind = fs::read_to_string(path.join(".git/gitcomet-benchmark-fixture"))
        .expect("only marked disposable fixtures may be benchmarked");
    let index_path = path.join(".git/index");
    let original = fs::read(&index_path).unwrap();
    let original_mtime = fs::metadata(&index_path).unwrap().modified().unwrap();
    let repo = GixBackend.open(path).unwrap();
    for iteration in 0..repeats {
        // Force clean-content comparison without touching index bytes/stat data.
        let index_repo = gix::open(path).unwrap();
        let index = index_repo.index_or_empty().unwrap();
        for entry in index.entries() {
            if !matches!(
                entry.mode,
                gix::index::entry::Mode::FILE | gix::index::entry::Mode::FILE_EXECUTABLE
            ) {
                continue;
            }
            let relative = gix::path::try_from_bstr(entry.path(&index)).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(path.join(relative))
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(60 + iteration as u64))
                .unwrap();
        }
        let token = CancellationToken::new();
        let start = Instant::now();
        let status = repo.status_cancellable(&token).unwrap();
        let status_elapsed = start.elapsed();
        let stats_start = Instant::now();
        let stats = repo
            .uncommitted_line_stats_for_status_cancellable(&status, &token)
            .unwrap();
        let stats_elapsed = stats_start.elapsed();
        let elapsed = start.elapsed();
        assert!(status.staged.is_empty());
        assert_eq!(status.unstaged.len(), usize::from(kind == "dirty-lfs"));
        if kind == "dirty-lfs" {
            assert_eq!(status.unstaged[0].path, Path::new("asset-00000.bin"));
            assert_eq!(
                status.unstaged[0].kind,
                gitcomet_core::domain::FileStatusKind::Modified
            );
            assert!(status.unstaged[0].conflict.is_none());
        }
        assert_eq!(
            fs::read(&index_path).unwrap(),
            original,
            "status modified the index"
        );
        assert_eq!(
            fs::metadata(&index_path).unwrap().modified().unwrap(),
            original_mtime
        );
        println!(
            "iteration={iteration} elapsed_ms={:.3} status_ms={:.3} line_stats_ms={:.3} unstaged={} stats={stats:?}",
            elapsed.as_secs_f64() * 1000.0,
            status_elapsed.as_secs_f64() * 1000.0,
            stats_elapsed.as_secs_f64() * 1000.0,
            status.unstaged.len()
        );
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let path = Path::new(args.get(2).expect("init/run <fixture>"));
    match args[1].as_str() {
        "init" => init(path, args.get(3).expect("fixture kind")),
        "run" => run(path, args.get(3).map_or(1, |n| n.parse().unwrap())),
        _ => panic!("expected init or run"),
    }
}
