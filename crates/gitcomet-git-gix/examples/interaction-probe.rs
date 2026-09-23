//! Shared before/after driver. Invoke only against disposable benchmark repos.
//! Usage: interaction-probe REPO OP PATH SAMPLES WARMUPS [context]
use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::git_operation::{self, GitOperationContext, GitOperationEvent};
use gitcomet_core::services::GitBackend;
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
    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: DiffArea::Unstaged,
    };
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
        let witness = match operation.as_str() {
            "diff" => {
                let diff = repo.diff_file_text(&target).unwrap().expect("text diff");
                json!({"old": diff.old_source.as_ref().map(|source| &source.path),
                    "new": diff.new_source.as_ref().map(|source| &source.path)})
            }
            "status" => {
                let status = repo.status().unwrap();
                json!({"staged":status.staged.len(), "unstaged":status.unstaged.len()})
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
            _ => panic!("unsupported operation"),
        };
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        if ix >= warmups {
            observations.push(json!({"milliseconds":elapsed, "witness":witness, "progress":*events.lock().unwrap()}));
        }
    }
    println!("{}", json!({"operation":operation, "samples":observations}));
}
