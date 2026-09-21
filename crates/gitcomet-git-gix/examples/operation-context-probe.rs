//! Local review probe: compare identical GitComet operations with activity tracing.
use gitcomet_core::git_operation::{self, GitOperationContext};
use gitcomet_core::services::{GitBackend, RemoteUrlKind};
use gitcomet_git_gix::GixBackend;
use std::{process::Command, time::Instant};

fn main() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["remote", "add", "origin", "https://example.invalid/repo"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "")
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let repo = GixBackend.open(directory.path()).unwrap();
    let operation = GitOperationContext::new("probe", |_, _| {});
    let mut plain = Vec::new();
    let mut traced = Vec::new();
    for round in 0..40 {
        for attach in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let _scope = attach.then(|| git_operation::attach(&operation));
            let started = Instant::now();
            repo.set_remote_url_with_output(
                "origin",
                "https://example.invalid/repo",
                RemoteUrlKind::Fetch,
            )
            .unwrap();
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if round >= 5 {
                if attach {
                    traced.push(elapsed);
                } else {
                    plain.push(elapsed);
                }
            }
        }
    }
    for (label, mut times) in [("plain", plain), ("operation-context", traced)] {
        times.sort_by(f64::total_cmp);
        println!(
            "{label}: n={} median_ms={:.3} min_ms={:.3} max_ms={:.3}",
            times.len(),
            times[times.len() / 2],
            times[0],
            times[times.len() - 1]
        );
    }
}
