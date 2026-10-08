use super::common::*;

pub(crate) fn bench_watcher_startup(c: &mut Criterion) {
    let mut group = c.benchmark_group("native_watcher_startup");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
    for (case, directories, ignored, cancel) in [
        ("small", 16, false, false),
        ("near_limit", 4090, false, false),
        ("ignored_churn", 4090, true, false),
        ("cancel_during_setup", 4090, false, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let parent = if ignored {
            std::fs::write(root.join(".gitignore"), "generated/\n").unwrap();
            root.join("generated")
        } else {
            root.clone()
        };
        for index in 0..directories {
            std::fs::create_dir_all(parent.join(format!("dir-{index}"))).unwrap();
        }
        let backend = gitcomet_git_gix::GixBackend;
        group.bench_function(case, |b| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let (duration, _, _) =
                        gitcomet_state::benchmarks::watcher_startup(&root, &backend, cancel);
                    elapsed += duration;
                }
                elapsed
            })
        });
        let (duration, visited, ready) = measure_sidecar_allocations(|| {
            gitcomet_state::benchmarks::watcher_startup(&root, &backend, cancel)
        });
        let mut payload = serde_json::Map::new();
        payload.insert("startup_ns".into(), serde_json::json!(duration.as_nanos()));
        payload.insert("directories".into(), serde_json::json!(visited));
        payload.insert("ready".into(), serde_json::json!(ready));
        emit_sidecar_metrics(&format!("native_watcher_startup/{case}"), payload);
    }
    group.finish();
}
