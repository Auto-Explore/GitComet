//! Shared before/after driver. Invoke only against disposable benchmark repos.
//! Usage: interaction-probe REPO OP PATH SAMPLES WARMUPS [context]
use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget};
use gitcomet_core::git_operation::{self, GitOperationContext, GitOperationEvent};
use gitcomet_core::large_files::LargeFileCommand;
use gitcomet_core::services::GitBackend;
use gitcomet_core::services::PullMode;
use gitcomet_git_gix::GixBackend;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(args.len() >= 6, "REPO OP PATH SAMPLES WARMUPS [context]");
    let root = PathBuf::from(&args[1]);
    let operation = &args[2];
    let path = PathBuf::from(&args[3]);
    let samples: usize = args[4].parse().unwrap();
    let warmups: usize = args[5].parse().unwrap();
    assert!(samples > 0 && samples <= 10000 && warmups <= 10000);
    let repo = GixBackend.open(&root).unwrap();
    let target = DiffTarget::working_tree(path.clone(), DiffArea::Unstaged);
    let mut observations = Vec::new();
    for ix in 0..warmups + samples {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let started = Instant::now();
        let context = GitOperationContext::new("interaction-probe", move |_, event| {
            if let GitOperationEvent::Output { chunks } = event {
                sink.lock()
                    .unwrap()
                    .push(json!({"at_ms": started.elapsed().as_secs_f64() * 1000.0,
                    "bytes": chunks.iter().map(|chunk| chunk.text.len()).sum::<usize>()}));
            }
        });
        let _scope = (args.get(6).map(String::as_str) == Some("context"))
            .then(|| git_operation::attach(&context));
        let (witness, commands) = gitcomet_git_gix::command_trace::capture(|| {
            match operation.as_str() {
                "diff" => {
                    let diff = repo.diff_file_text(&target).unwrap().expect("text diff");
                    json!({"old": diff.old_source.as_ref().map(|source| &source.path),
                    "new": diff.new_source.as_ref().map(|source| &source.path)})
                }
                "status" => {
                    let status = repo.status().unwrap();
                    json!({"staged":status.staged.len(), "unstaged":status.unstaged.len()})
                }
                "large-file-support" => {
                    let support = repo
                        .large_file_support_cancellable(&Default::default())
                        .unwrap();
                    json!({"lfs": support.lfs.in_use(), "annex": support.annex.in_use(),
                        "lfs_patterns": support.lfs.tracked_patterns.len(),
                        "annex_repositories": support.annex.repositories.len()})
                }
                "large-file-status" => {
                    let status = repo.status().unwrap();
                    let files = repo
                        .uncommitted_large_files_for_status_cancellable(
                            &status,
                            &Default::default(),
                        )
                        .unwrap();
                    json!({"staged": files.staged.len(), "unstaged": files.unstaged.len()})
                }
                "commit-details" => {
                    let details = repo
                        .commit_details(&CommitId(args[3].clone().into()))
                        .unwrap();
                    json!({"id": details.id.as_ref(), "files": details.files.len(),
                        "large_files": details.files.iter().filter(|file| file.large_file.is_some()).count()})
                }
                "stage" => {
                    repo.stage(&[&path]).unwrap();
                    json!("staged")
                }
                "checkout" => {
                    repo.checkout_branch(&args[3]).unwrap();
                    json!("checked-out")
                }
                "push" => {
                    repo.push_with_output().unwrap();
                    json!("pushed")
                }
                "fetch" => {
                    repo.fetch_all_with_output().unwrap();
                    json!("fetched")
                }
                "pull" | "pull-merge" => {
                    repo.pull_with_output(PullMode::Merge).unwrap();
                    json!("pulled")
                }
                "pull-rebase" => {
                    repo.pull_with_output(PullMode::Rebase).unwrap();
                    json!("pulled")
                }
                "lfs-fetch" => {
                    repo.run_large_file_command(&LargeFileCommand::LfsFetchAll)
                        .unwrap();
                    json!("fetched")
                }
                "lfs-pull" => {
                    repo.run_large_file_command(&LargeFileCommand::LfsPull { paths: vec![] })
                        .unwrap();
                    json!("pulled")
                }
                "lfs-push" => {
                    repo.run_large_file_command(&LargeFileCommand::LfsPushAll {
                        remote: "origin".into(),
                    })
                    .unwrap();
                    json!("pushed")
                }
                "annex-get" => {
                    repo.run_large_file_command(&LargeFileCommand::AnnexGet {
                        paths: vec![".".into()],
                        from: Some("backup".into()),
                    })
                    .unwrap();
                    json!("got")
                }
                "annex-copy" => {
                    repo.run_large_file_command(&LargeFileCommand::AnnexCopy {
                        paths: vec![".".into()],
                        to: "backup".into(),
                    })
                    .unwrap();
                    json!("copied")
                }
                "annex-pull" => {
                    repo.run_large_file_command(&LargeFileCommand::AnnexPull { content: true })
                        .unwrap();
                    json!("pulled")
                }
                "annex-push" => {
                    repo.run_large_file_command(&LargeFileCommand::AnnexPush { content: true })
                        .unwrap();
                    json!("pushed")
                }
                "annex-sync" => {
                    repo.run_large_file_command(&LargeFileCommand::AnnexSync { content: true })
                        .unwrap();
                    json!("synced")
                }
                _ => panic!("unsupported operation"),
            }
        });
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        if ix >= warmups {
            let timings: Vec<_> = commands.iter().map(|command| json!({"label": command.label,
                "milliseconds": command.elapsed.as_secs_f64() * 1000.0,
                "stages": command.stages.iter().map(|(stage, duration)| json!({"stage": stage, "milliseconds": duration.as_secs_f64() * 1000.0})).collect::<Vec<_>>() })).collect();
            observations.push(json!({"milliseconds":elapsed, "witness":witness, "progress":*events.lock().unwrap(), "commands": timings}));
        }
    }
    println!("{}", json!({"operation":operation, "samples":observations}));
}
