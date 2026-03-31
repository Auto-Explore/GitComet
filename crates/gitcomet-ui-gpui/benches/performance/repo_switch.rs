use super::common::*;

pub(crate) fn bench_repo_switch(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);

    let refocus_same_repo =
        RepoSwitchFixture::refocus_same_repo(commits, local_branches, remote_branches, remotes);
    let two_hot_repos =
        RepoSwitchFixture::two_hot_repos(commits, local_branches, remote_branches, remotes);
    let selected_commit_and_details = RepoSwitchFixture::selected_commit_and_details(
        commits,
        local_branches,
        remote_branches,
        remotes,
    );
    let twenty_tabs =
        RepoSwitchFixture::twenty_tabs(commits, local_branches, remote_branches, remotes);
    let twenty_repos_all_hot =
        RepoSwitchFixture::twenty_repos_all_hot(commits, local_branches, remote_branches, remotes);
    let selected_diff_file =
        RepoSwitchFixture::selected_diff_file(commits, local_branches, remote_branches, remotes);
    let selected_conflict_target = RepoSwitchFixture::selected_conflict_target(
        commits,
        local_branches,
        remote_branches,
        remotes,
    );
    let merge_active_with_draft_restore = RepoSwitchFixture::merge_active_with_draft_restore(
        commits,
        local_branches,
        remote_branches,
        remotes,
    );

    let mut group = c.benchmark_group("repo_switch");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("refocus_same_repo"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = refocus_same_repo.fresh_state();
                let started_at = Instant::now();
                let _ = refocus_same_repo.run_with_state_hash_only(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = refocus_same_repo.fresh_state();
            let (_, metrics) = measure_sidecar_allocations(|| {
                refocus_same_repo.run_with_state(&mut sidecar_state)
            });
            emit_repo_switch_sidecar("refocus_same_repo", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("two_hot_repos"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = two_hot_repos.fresh_state();
                let started_at = Instant::now();
                let _ = two_hot_repos.run_with_state_hash_only(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = two_hot_repos.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| two_hot_repos.run_with_state(&mut sidecar_state));
            emit_repo_switch_sidecar("two_hot_repos", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("selected_commit_and_details"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = selected_commit_and_details.fresh_state();
                    let started_at = Instant::now();
                    let _ = selected_commit_and_details.run_with_state_hash_only(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = selected_commit_and_details.fresh_state();
                let (_, metrics) = measure_sidecar_allocations(|| {
                    selected_commit_and_details.run_with_state(&mut sidecar_state)
                });
                emit_repo_switch_sidecar("selected_commit_and_details", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(BenchmarkId::from_parameter("twenty_tabs"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = twenty_tabs.fresh_state();
                let started_at = Instant::now();
                let _ = twenty_tabs.run_with_state_hash_only(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = twenty_tabs.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| twenty_tabs.run_with_state(&mut sidecar_state));
            emit_repo_switch_sidecar("twenty_tabs", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("20_repos_all_hot"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = twenty_repos_all_hot.fresh_state();
                let started_at = Instant::now();
                let _ = twenty_repos_all_hot.run_with_state_hash_only(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = twenty_repos_all_hot.fresh_state();
            let (_, metrics) = measure_sidecar_allocations(|| {
                twenty_repos_all_hot.run_with_state(&mut sidecar_state)
            });
            emit_repo_switch_sidecar("20_repos_all_hot", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("selected_diff_file"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = selected_diff_file.fresh_state();
                let started_at = Instant::now();
                let _ = selected_diff_file.run_with_state_hash_only(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = selected_diff_file.fresh_state();
            let (_, metrics) = measure_sidecar_allocations(|| {
                selected_diff_file.run_with_state(&mut sidecar_state)
            });
            emit_repo_switch_sidecar("selected_diff_file", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("selected_conflict_target"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = selected_conflict_target.fresh_state();
                    let started_at = Instant::now();
                    let _ = selected_conflict_target.run_with_state_hash_only(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = selected_conflict_target.fresh_state();
                let (_, metrics) = measure_sidecar_allocations(|| {
                    selected_conflict_target.run_with_state(&mut sidecar_state)
                });
                emit_repo_switch_sidecar("selected_conflict_target", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("merge_active_with_draft_restore"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = merge_active_with_draft_restore.fresh_state();
                    let started_at = Instant::now();
                    let _ = merge_active_with_draft_restore.run_with_state_hash_only(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = merge_active_with_draft_restore.fresh_state();
                let (_, metrics) = measure_sidecar_allocations(|| {
                    merge_active_with_draft_restore.run_with_state(&mut sidecar_state)
                });
                emit_repo_switch_sidecar("merge_active_with_draft_restore", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}
