use super::*;

#[test]
fn pull_emits_its_safe_command_before_output_and_keeps_it_on_failure() {
    use gitcomet_core::git_operation::{self, GitOperationContext, GitOperationEvent};
    use std::sync::Arc;

    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let (remote, work) = init_work_repo_with_remote(dir.path(), "origin");
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let context =
        GitOperationContext::new("Pull", move |_, event| sink.lock().unwrap().push(event));
    let _scope = git_operation::attach(&context);
    let opened = GixBackend.open(&work).unwrap();

    let output = opened.pull(PullMode::Rebase).unwrap();
    {
        let events = events.lock().unwrap();
        assert!(
            matches!(events.first(), Some(GitOperationEvent::CommandInvocation { command }) if command == &output.command)
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, GitOperationEvent::CommandStarted))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, GitOperationEvent::Output { .. }))
        );
    }
    events.lock().unwrap().clear();
    run_git(&remote, &["update-ref", "-d", "refs/heads/main"]);
    let error = opened.pull(PullMode::Rebase).unwrap_err();
    let events = events.lock().unwrap();
    let Some(GitOperationEvent::CommandInvocation { command }) = events.first() else {
        panic!("failed Pull must expose its command before output");
    };
    assert!(command.ends_with("pull --progress --rebase --no-prune -- origin refs/heads/main"));
    assert!(error.to_string().contains(command));
}

#[test]
fn pull_fetches_only_the_configured_upstream_without_pruning() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let (remote, work) = init_work_repo_with_remote(dir.path(), "origin");
    let writer = dir.path().join("writer");
    run_git(
        dir.path(),
        &["clone", "-q", &git_remote_url(&remote), "writer"],
    );
    configure_repo_with_user(&writer);
    run_git(&writer, &["checkout", "-q", "-b", "unrelated"]);
    fs::write(writer.join("unrelated.txt"), "unrelated object\n").unwrap();
    run_git(&writer, &["add", "unrelated.txt"]);
    run_git(&writer, &["commit", "-q", "-m", "unrelated"]);
    run_git(&writer, &["push", "-q", "origin", "unrelated"]);
    let unrelated = run_git_capture(&writer, &["rev-parse", "HEAD"]);
    run_git(&writer, &["checkout", "-q", "main"]);
    fs::write(writer.join("upstream.txt"), "upstream update\n").unwrap();
    run_git(&writer, &["add", "upstream.txt"]);
    run_git(&writer, &["commit", "-q", "-m", "upstream"]);
    run_git(&writer, &["push", "-q", "origin", "main"]);
    let upstream = run_git_capture(&writer, &["rev-parse", "HEAD"]);
    run_git(&work, &["config", "remote.origin.tagOpt", "--no-tags"]);
    run_git(&work, &["config", "fetch.prune", "true"]);
    run_git(&work, &["config", "fetch.all", "true"]);
    run_git(
        &work,
        &["remote", "add", "unavailable", "/missing-pull-remote.git"],
    );
    run_git(&work, &["update-ref", "refs/remotes/origin/stale", "HEAD"]);

    let opened = GixBackend.open(&work).unwrap();
    opened.pull(PullMode::Rebase).unwrap();

    assert_eq!(run_git_capture(&work, &["rev-parse", "HEAD"]), upstream);
    assert!(!ref_exists(&work, "refs/remotes/origin/unrelated"));
    assert!(!run_git_status(&work, &["cat-file", "-e", unrelated.trim()]).success());
    assert!(ref_exists(&work, "refs/remotes/origin/stale"));
    let fetched = fs::read_to_string(work.join(".git/FETCH_HEAD")).unwrap();
    assert_eq!(fetched.lines().count(), 1, "{fetched}");
}

fn advance_main(root: &Path, remote: &Path) -> String {
    let writer = root.join("writer");
    run_git(root, &["clone", "-q", &git_remote_url(remote), "writer"]);
    configure_repo_with_user(&writer);
    fs::write(writer.join("upstream.txt"), "upstream update\n").unwrap();
    run_git(&writer, &["add", "upstream.txt"]);
    run_git(&writer, &["commit", "-q", "-m", "upstream"]);
    run_git(&writer, &["push", "-q", "origin", "main"]);
    run_git_capture(&writer, &["rev-parse", "HEAD"])
}

#[test]
fn configured_pull_uses_fresh_config_without_requiring_a_tracking_ref() {
    let _guard = remote_management_test_lock();
    for mapping in [
        "+refs/heads/main:refs/remotes/upstream/review",
        "+refs/heads/other:refs/remotes/upstream/other",
        "+refs/heads/*:refs/remotes/upstream/*",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (remote, work) = init_work_repo_with_remote(dir.path(), "upstream");
        let expected = advance_main(dir.path(), &remote);
        run_git(&work, &["checkout", "-q", "-b", "local"]);
        let opened = GixBackend.open(&work).unwrap();
        run_git(&work, &["config", "branch.local.remote", "upstream"]);
        run_git(&work, &["config", "branch.local.merge", "refs/heads/main"]);
        run_git(&work, &["config", "remote.upstream.fetch", mapping]);
        if mapping.contains('*') {
            run_git(
                &work,
                &[
                    "config",
                    "--add",
                    "remote.upstream.fetch",
                    "^refs/heads/main",
                ],
            );
        }
        run_git(&work, &["update-ref", "-d", "refs/remotes/upstream/main"]);

        let output = opened.pull(PullMode::FastForwardOnly).unwrap();

        assert_eq!(run_git_capture(&work, &["rev-parse", "HEAD"]), expected);
        assert!(output.command.ends_with("-- upstream refs/heads/main"));
        assert_eq!(
            run_git_capture(&work, &["config", "branch.local.merge"]).trim(),
            "refs/heads/main"
        );
        if mapping.ends_with("/review") {
            assert_eq!(
                run_git_capture(&work, &["rev-parse", "refs/remotes/upstream/review"]),
                expected
            );
        } else if mapping.contains('*') {
            assert_eq!(
                run_git_capture(&work, &["rev-parse", "refs/remotes/upstream/main"]),
                expected
            );
        } else {
            assert!(!ref_exists(&work, "refs/remotes/upstream/main"));
        }
    }
}

#[test]
fn pull_accepts_a_repository_url_as_the_configured_remote() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let (remote, work) = init_work_repo_with_remote(dir.path(), "origin");
    let expected = advance_main(dir.path(), &remote);
    run_git(
        &work,
        &["config", "branch.main.remote", &git_remote_url(&remote)],
    );
    run_git(
        &work,
        &["remote", "set-url", "origin", "/missing-pull-remote.git"],
    );

    GixBackend
        .open(&work)
        .unwrap()
        .pull(PullMode::FastForwardOnly)
        .unwrap();

    assert_eq!(run_git_capture(&work, &["rev-parse", "HEAD"]), expected);
}

#[test]
fn pull_rejects_incomplete_or_multiple_upstreams_before_fetching() {
    let _guard = remote_management_test_lock();
    for (repository, sources) in [
        (Some("origin"), vec![]),
        (None, vec!["refs/heads/main"]),
        (Some(""), vec!["refs/heads/main"]),
        (Some("origin"), vec!["main"]),
        (Some("origin"), vec!["refs/heads/*"]),
        (Some("origin"), vec!["refs/heads/main", "refs/heads/other"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (_, work) = init_work_repo_with_remote(dir.path(), "origin");
        run_git(&work, &["config", "--remove-section", "branch.main"]);
        if let Some(repository) = repository {
            run_git(&work, &["config", "branch.main.remote", repository]);
        }
        for source in sources {
            run_git(&work, &["config", "--add", "branch.main.merge", source]);
        }
        let config = fs::read(work.join(".git/config")).unwrap();
        let head = run_git_capture(&work, &["rev-parse", "HEAD"]);
        fs::write(work.join(".git/FETCH_HEAD"), "untouched\n").unwrap();

        let error = GixBackend
            .open(&work)
            .unwrap()
            .pull(PullMode::Rebase)
            .unwrap_err();

        assert!(matches!(error.kind(), ErrorKind::Unsupported(_)), "{error}");
        assert_eq!(
            fs::read_to_string(work.join(".git/FETCH_HEAD")).unwrap(),
            "untouched\n"
        );
        assert_eq!(fs::read(work.join(".git/config")).unwrap(), config);
        assert_eq!(run_git_capture(&work, &["rev-parse", "HEAD"]), head);
    }
}

#[test]
fn automatic_upstream_uses_its_unique_mapping_and_git_rebase_configuration() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let (remote, work) = init_work_repo_with_remote(dir.path(), "upstream");
    let expected = advance_main(dir.path(), &remote);
    run_git(&work, &["branch", "--unset-upstream"]);
    run_git(&work, &["config", "branch.autoSetupRebase", "always"]);
    run_git(
        &work,
        &[
            "config",
            "remote.upstream.fetch",
            "+refs/heads/main:refs/remotes/review/main",
        ],
    );

    GixBackend
        .open(&work)
        .unwrap()
        .pull(PullMode::FastForwardOnly)
        .unwrap();

    assert_eq!(run_git_capture(&work, &["rev-parse", "HEAD"]), expected);
    assert_eq!(
        run_git_capture(&work, &["config", "branch.main.remote"]).trim(),
        "upstream"
    );
    assert_eq!(
        run_git_capture(&work, &["config", "branch.main.merge"]).trim(),
        "refs/heads/main"
    );
    assert_eq!(
        run_git_capture(&work, &["config", "branch.main.rebase"]).trim(),
        "true"
    );
}

#[test]
fn automatic_upstream_rejects_missing_or_ambiguous_tracking_mappings_before_fetch() {
    let _guard = remote_management_test_lock();
    for mappings in [
        vec!["+refs/heads/other:refs/remotes/origin/other"],
        vec![
            "+refs/heads/main:refs/remotes/origin/main",
            "+refs/heads/main:refs/remotes/review/main",
        ],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (_, work) = init_work_repo_with_remote(dir.path(), "origin");
        run_git(&work, &["branch", "--unset-upstream"]);
        run_git(&work, &["config", "--unset-all", "remote.origin.fetch"]);
        for mapping in mappings {
            run_git(&work, &["config", "--add", "remote.origin.fetch", mapping]);
        }
        fs::write(work.join(".git/FETCH_HEAD"), "untouched\n").unwrap();
        let error = GixBackend
            .open(&work)
            .unwrap()
            .pull(PullMode::Merge)
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Unsupported(_)), "{error}");
        assert_eq!(
            fs::read_to_string(work.join(".git/FETCH_HEAD")).unwrap(),
            "untouched\n"
        );
        assert!(!run_git_status(&work, &["config", "--get", "branch.main.remote"]).success());
    }
}

#[test]
fn pull_honors_explicit_tag_options_without_pruning_tags() {
    let _guard = remote_management_test_lock();
    for tag_option in ["--no-tags", "--tags"] {
        let dir = tempfile::tempdir().unwrap();
        let (remote, work) = init_work_repo_with_remote(dir.path(), "origin");
        let expected = advance_main(dir.path(), &remote);
        run_git(&remote, &["tag", "remote-tag", expected.trim()]);
        run_git(&work, &["tag", "local-tag"]);
        run_git(&work, &["config", "remote.origin.tagOpt", tag_option]);
        run_git(&work, &["config", "remote.origin.pruneTags", "true"]);
        run_git(&work, &["config", "fetch.prune", "true"]);
        GixBackend
            .open(&work)
            .unwrap()
            .pull(PullMode::Rebase)
            .unwrap();
        assert_eq!(run_git_capture(&work, &["rev-parse", "HEAD"]), expected);
        assert_eq!(
            ref_exists(&work, "refs/tags/remote-tag"),
            tag_option == "--tags"
        );
        assert!(ref_exists(&work, "refs/tags/local-tag"));
    }
}

#[test]
fn automatic_upstream_rejects_a_destination_shared_by_other_sources_before_fetch() {
    let _guard = remote_management_test_lock();
    for (remote_name, excluded) in [
        ("origin", false),
        ("origin", true),
        ("mirror", false),
        ("mirror", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (remote, work) = init_work_repo_with_remote(dir.path(), "origin");
        advance_main(dir.path(), &remote);
        run_git(&work, &["branch", "--unset-upstream"]);
        if remote_name == "mirror" {
            run_git(
                &work,
                &["remote", "add", "mirror", &git_remote_url(&remote)],
            );
        }
        run_git(
            &work,
            &[
                "config",
                "--unset-all",
                &format!("remote.{remote_name}.fetch"),
            ],
        );
        run_git(
            &work,
            &[
                "config",
                "--add",
                &format!("remote.{remote_name}.fetch"),
                "+refs/heads/other:refs/remotes/origin/main",
            ],
        );
        run_git(
            &work,
            &[
                "config",
                "--add",
                &format!("remote.{remote_name}.fetch"),
                "+refs/heads/main:refs/remotes/origin/main",
            ],
        );
        if excluded {
            run_git(
                &work,
                &[
                    "config",
                    "--add",
                    &format!("remote.{remote_name}.fetch"),
                    "^refs/heads/other",
                ],
            );
        }
        let head = run_git_capture(&work, &["rev-parse", "HEAD"]);
        let config = fs::read(work.join(".git/config")).unwrap();
        fs::write(work.join(".git/FETCH_HEAD"), "untouched\n").unwrap();

        let error = GixBackend
            .open(&work)
            .unwrap()
            .pull(PullMode::FastForwardOnly)
            .unwrap_err();

        assert!(matches!(error.kind(), ErrorKind::Unsupported(_)), "{error}");
        assert_eq!(
            fs::read_to_string(work.join(".git/FETCH_HEAD")).unwrap(),
            "untouched\n"
        );
        assert_eq!(run_git_capture(&work, &["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(work.join(".git/config")).unwrap(), config);
    }
}
