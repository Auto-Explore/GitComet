use super::tests::bare_fixture;
use super::*;
use crate::test_support::{git_stdout, git_success, init_test_repo, open_repo};
use gitcomet_core::services::{GitBackend, GitRepository};
use std::fs;

#[test]
fn configuration_repair_uses_main_worktree_config_when_enabled() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    init_test_repo(root);
    git_success(root, &["config", "extensions.worktreeConfig", "true"]);
    git_success(root, &["config", "--worktree", "core.bare", "true"]);
    let shared_before = fs::read(root.join(".git/config")).unwrap();
    let head_before = fs::read(root.join(".git/HEAD")).unwrap();
    let repo = open_repo(root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(fs::read(root.join(".git/config")).unwrap(), shared_before);
    assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), head_before);
    assert_eq!(
        git_stdout(root, &["config", "--worktree", "--get", "core.bare"]),
        "false"
    );
    assert_eq!(native_mode(root).unwrap(), RepositoryMode::WorkingTree);
}

#[test]
fn configuration_repair_keeps_shared_bare_assignment_owned_by_shared_config() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    init_test_repo(root);
    git_success(root, &["config", "extensions.worktreeConfig", "true"]);
    git_success(root, &["config", "core.bare", "true"]);
    let repo = open_repo(root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert!(!root.join(".git/config.worktree").exists());
    assert_eq!(
        git_stdout(root, &["config", "--get-all", "core.bare"]),
        "false"
    );
    git_success(root, &["config", "core.bare", "true"]);
    assert_eq!(native_mode(root).unwrap(), RepositoryMode::Bare);
}

#[test]
fn omitted_linked_registration_refuses_conversion_and_head_rewrite() {
    let (_directory, root) = bare_fixture();
    let linked = root.parent().unwrap().join("linked");
    let branch = git_stdout(&root, &["symbolic-ref", "--short", "HEAD"]);
    git_success(
        &root,
        &["worktree", "add", linked.to_str().unwrap(), &branch],
    );
    let registration = native_path(&linked, "--absolute-git-dir").unwrap();
    fs::remove_file(registration.join("gitdir")).unwrap();
    let before = storage::tree_digest(&root).unwrap();
    let repo = open_repo(&root);
    for result in [
        repo.preview_disable_bare_mode().map(|_| ()),
        repo.reset_with_output("HEAD", gitcomet_core::services::ResetMode::Soft)
            .map(|_| ()),
    ] {
        assert!(matches!(
            result.unwrap_err().kind(),
            ErrorKind::BareConversionRefused(BareConversionRefusal::InaccessibleWorktree { .. })
        ));
        assert_eq!(storage::tree_digest(&root).unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn case_collisions_and_running_annex_assistant_refuse_without_mutation() {
    for kind in ["file-prefix", "directory-prefix", "assistant"] {
        let (directory, root) = bare_fixture();
        let checkout = directory.path().join("checkout");
        if kind == "assistant" {
            fs::create_dir(root.join("annex")).unwrap();
            fs::write(
                root.join("annex/daemon.pid"),
                std::process::id().to_string(),
            )
            .unwrap();
        } else {
            let (first, second) = if kind == "file-prefix" {
                ("A", "a/child")
            } else {
                ("Dir/first", "dir/second")
            };
            let blob = git_stdout(&checkout, &["rev-parse", "HEAD:tracked.txt"]);
            for name in [first, second] {
                git_success(
                    &checkout,
                    &[
                        "-c",
                        "core.ignorecase=false",
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{blob},{name}"),
                    ],
                );
            }
            git_success(&checkout, &["commit", "-m", "case collision"]);
            git_success(
                &root,
                &["fetch", checkout.to_str().unwrap(), "+HEAD:refs/heads/main"],
            );
            git_success(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        }
        let before = storage::tree_digest(&root).unwrap();
        let error = open_repo(&root).preview_disable_bare_mode().unwrap_err();
        if kind == "assistant" {
            assert!(matches!(
                error.kind(),
                ErrorKind::BareConversionRefused(BareConversionRefusal::ActiveOperation { .. })
            ));
        } else {
            assert!(matches!(
                error.kind(),
                ErrorKind::BareConversionRefused(BareConversionRefusal::PathCollision { .. })
            ));
        }
        assert_eq!(storage::tree_digest(&root).unwrap(), before);
    }
}

#[test]
fn config_repair_preserves_initialized_submodule_and_dirty_index() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    let root = directory.path().join("main");
    fs::create_dir(&child).unwrap();
    fs::create_dir(&root).unwrap();
    init_test_repo(&child);
    init_test_repo(&root);
    fs::write(child.join("content.txt"), "child\n").unwrap();
    git_success(&child, &["add", "."]);
    git_success(&child, &["commit", "-m", "child"]);
    git_success(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            child.to_str().unwrap(),
            "nested",
        ],
    );
    git_success(&root, &["commit", "-m", "submodule"]);
    fs::write(root.join("nested/content.txt"), "dirty child\n").unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    let submodule = storage::tree_digest(&root.join(".git/modules")).unwrap();
    let pointer = fs::read(root.join("nested/.git")).unwrap();
    git_success(&root, &["config", "core.bare", "true"]);
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(
        storage::tree_digest(&root.join(".git/modules")).unwrap(),
        submodule
    );
    assert_eq!(fs::read(root.join("nested/.git")).unwrap(), pointer);
    assert_eq!(
        fs::read_to_string(root.join("nested/content.txt")).unwrap(),
        "dirty child\n"
    );
}

#[test]
fn config_repair_allows_existing_path_dependent_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    init_test_repo(root);
    fs::write(
        root.join(".git/local-config"),
        "[custom]\nvalue = retained\n",
    )
    .unwrap();
    git_success(root, &["config", "include.path", "local-config"]);
    git_success(root, &["config", "lfs.storage", "retained-store"]);
    git_success(root, &["config", "core.bare", "true"]);
    let repo = open_repo(root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(git_stdout(root, &["config", "custom.value"]), "retained");
    assert_eq!(
        git_stdout(root, &["config", "lfs.storage"]),
        "retained-store"
    );
}

#[test]
fn relocation_refusals_leave_every_source_byte_unchanged() {
    for kind in [
        "unknown",
        "index",
        "lock",
        "operation",
        "include",
        "conditional",
        "hooks",
        "lfs",
        "annex",
        "alternates",
        "nested",
    ] {
        let (_directory, root) = bare_fixture();
        match kind {
            "unknown" => fs::write(root.join("unexpected.txt"), "retain").unwrap(),
            "index" => git_success(&root, &["read-tree", "HEAD"]),
            "lock" => fs::write(root.join("refs/heads/main.lock"), "lock").unwrap(),
            "operation" => fs::create_dir(root.join("rebase-merge")).unwrap(),
            "include" => {
                fs::write(
                    root.join("info/included.config"),
                    "[custom]\nvalue = retained\n",
                )
                .unwrap();
                git_success(
                    &root,
                    &[
                        "config",
                        "include.path",
                        root.join("info/included.config").to_str().unwrap(),
                    ],
                );
            }
            "conditional" => git_success(
                &root,
                &[
                    "config",
                    "includeIf.gitdir:/unmatched/.path",
                    "/unmatched/config",
                ],
            ),
            "hooks" => git_success(
                &root,
                &[
                    "config",
                    "core.hooksPath",
                    root.join("hooks").to_str().unwrap(),
                ],
            ),
            "lfs" => git_success(&root, &["config", "lfs.storage", "store"]),
            "annex" => git_success(&root, &["config", "annex.objects", "store"]),
            "alternates" => {
                fs::write(root.join("objects/info/alternates"), "/nonexistent\n").unwrap()
            }
            "nested" => git_success(
                &root,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "nested",
                    root.join("nested").to_str().unwrap(),
                ],
            ),
            _ => unreachable!(),
        }
        let before = storage::tree_digest(&root).unwrap();
        let result = crate::GixBackend
            .open(&root)
            .and_then(|repo| repo.preview_disable_bare_mode());
        assert!(
            matches!(
                result.unwrap_err().kind(),
                ErrorKind::BareConversionRefused(_)
            ),
            "{kind}"
        );
        assert_eq!(storage::tree_digest(&root).unwrap(), before, "{kind}");
        assert!(!root.join(".git").exists(), "{kind}");
    }
}

#[test]
fn inaccessible_linked_checkout_refuses_conversion_without_mutation() {
    let (_directory, root) = bare_fixture();
    let linked = root.parent().unwrap().join("linked");
    git_success(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    fs::rename(&linked, root.parent().unwrap().join("moved-linked")).unwrap();
    let before = storage::tree_digest(&root).unwrap();
    let error = open_repo(&root).preview_disable_bare_mode().unwrap_err();
    assert!(matches!(
        error.kind(),
        ErrorKind::BareConversionRefused(BareConversionRefusal::InaccessibleWorktree { .. })
    ));
    assert_eq!(storage::tree_digest(&root).unwrap(), before);
}

#[test]
fn linked_relocation_config_refuses_metadata_paths_and_preserves_external_paths() {
    for kind in ["relative-worktree", "metadata-hook", "external-path"] {
        let (directory, root) = bare_fixture();
        let linked = directory.path().join("linked");
        git_success(
            &root,
            &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
        );
        git_success(&root, &["config", "extensions.worktreeConfig", "true"]);
        git_success(&root, &["config", "--worktree", "core.bare", "true"]);
        git_success(&linked, &["config", "--worktree", "core.bare", "false"]);
        match kind {
            "relative-worktree" => git_success(
                &linked,
                &["config", "--worktree", "core.worktree", "../../../linked"],
            ),
            "metadata-hook" => git_success(
                &linked,
                &[
                    "config",
                    "--worktree",
                    "core.hooksPath",
                    root.join("hooks").to_str().unwrap(),
                ],
            ),
            "external-path" => {
                let external = directory.path().join("external.config");
                fs::write(&external, "[custom]\n\tretained = external\n").unwrap();
                git_success(
                    &linked,
                    &[
                        "config",
                        "--worktree",
                        "include.path",
                        external.to_str().unwrap(),
                    ],
                );
                git_success(
                    &linked,
                    &[
                        "config",
                        "--worktree",
                        "core.worktree",
                        linked.to_str().unwrap(),
                    ],
                );
            }
            _ => unreachable!(),
        }
        fs::write(linked.join("tracked.txt"), "linked dirty\n").unwrap();
        let before = storage::tree_digest(&root).unwrap();
        let pointer = fs::read(linked.join(".git")).unwrap();
        let repo = open_repo(&root);
        if kind == "external-path" {
            let preview = repo.preview_disable_bare_mode().unwrap();
            repo.disable_bare_mode(&preview).unwrap();
            assert_eq!(native_path(&linked, "--show-toplevel").unwrap(), linked);
            assert_eq!(native_mode(&linked).unwrap(), RepositoryMode::WorkingTree);
            assert_eq!(
                git_stdout(&linked, &["config", "custom.retained"]),
                "external"
            );
            assert_ne!(fs::read(linked.join(".git")).unwrap(), pointer);
        } else {
            let error = repo.preview_disable_bare_mode().unwrap_err();
            assert!(matches!(
                error.kind(),
                ErrorKind::BareConversionRefused(
                    BareConversionRefusal::RelocationDependentConfig { .. }
                )
            ));
            assert_eq!(storage::tree_digest(&root).unwrap(), before);
            assert_eq!(fs::read(linked.join(".git")).unwrap(), pointer);
        }
        assert_eq!(
            fs::read_to_string(linked.join("tracked.txt")).unwrap(),
            "linked dirty\n"
        );
    }
}

#[test]
fn linked_external_config_change_invalidates_conversion_preview() {
    let (directory, root) = bare_fixture();
    let linked = directory.path().join("linked");
    git_success(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    git_success(&root, &["config", "extensions.worktreeConfig", "true"]);
    git_success(&root, &["config", "--worktree", "core.bare", "true"]);
    git_success(&linked, &["config", "--worktree", "core.bare", "false"]);
    let external = directory.path().join("external.config");
    fs::write(&external, "[custom]\n\tretained = before\n").unwrap();
    git_success(
        &linked,
        &[
            "config",
            "--worktree",
            "include.path",
            external.to_str().unwrap(),
        ],
    );
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    fs::write(&external, "[custom]\n\tretained = after\n").unwrap();
    let before = storage::tree_digest(&root).unwrap();
    let pointer = fs::read(linked.join(".git")).unwrap();
    let error = repo.disable_bare_mode(&preview).unwrap_err();
    assert!(matches!(
        error.kind(),
        ErrorKind::BareConversionRefused(BareConversionRefusal::StalePreview)
    ));
    assert_eq!(storage::tree_digest(&root).unwrap(), before);
    assert_eq!(fs::read(linked.join(".git")).unwrap(), pointer);
}

#[cfg(unix)]
#[test]
fn symlinked_config_is_never_replaced_or_changed() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main");
    fs::create_dir(&root).unwrap();
    init_test_repo(&root);
    git_success(&root, &["config", "core.bare", "true"]);
    let external = directory.path().join("shared-config");
    fs::rename(root.join(".git/config"), &external).unwrap();
    symlink(&external, root.join(".git/config")).unwrap();
    let before = fs::read(&external).unwrap();
    let error = open_repo(&root).preview_disable_bare_mode().unwrap_err();
    assert!(matches!(
        error.kind(),
        ErrorKind::BareConversionRefused(BareConversionRefusal::UnsupportedLayout { .. })
    ));
    assert!(root.join(".git/config").is_symlink());
    assert_eq!(fs::read(external).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn relocation_uses_native_required_smudge_and_crlf_checkout() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, root) = bare_fixture();
    let hook = root.join("hooks/post-checkout");
    fs::write(&hook, "#!/bin/sh\nexit 93\n").unwrap();
    fs::set_permissions(hook, fs::Permissions::from_mode(0o755)).unwrap();
    let checkout = directory.path().join("checkout");
    fs::write(
        checkout.join(".gitattributes"),
        "tracked.txt filter=rewrite text eol=crlf\n",
    )
    .unwrap();
    git_success(&checkout, &["add", ".gitattributes"]);
    git_success(&checkout, &["commit", "-m", "attributes"]);
    git_success(
        &root,
        &["fetch", checkout.to_str().unwrap(), "+HEAD:refs/heads/main"],
    );
    git_success(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_success(
        &root,
        &[
            "config",
            "filter.rewrite.smudge",
            "sed s/original/materialized/g",
        ],
    );
    git_success(
        &root,
        &[
            "config",
            "filter.rewrite.clean",
            "sed s/materialized/original/g",
        ],
    );
    git_success(&root, &["config", "filter.rewrite.required", "true"]);
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(
        fs::read(root.join("tracked.txt")).unwrap(),
        b"materialized\r\n"
    );
    assert_eq!(git_stdout(&root, &["status", "--porcelain=v1"]), "");
}

#[cfg(unix)]
#[test]
fn required_smudge_failure_aborts_before_install_and_preserves_source() {
    let (directory, root) = bare_fixture();
    let checkout = directory.path().join("checkout");
    fs::write(
        checkout.join(".gitattributes"),
        "tracked.txt filter=failing\n",
    )
    .unwrap();
    git_success(&checkout, &["add", ".gitattributes"]);
    git_success(&checkout, &["commit", "-m", "attributes"]);
    git_success(
        &root,
        &["fetch", checkout.to_str().unwrap(), "+HEAD:refs/heads/main"],
    );
    git_success(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_success(&root, &["config", "filter.failing.smudge", "exit 7"]);
    git_success(&root, &["config", "filter.failing.required", "true"]);
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    let before = storage::tree_digest(&root).unwrap();
    assert!(repo.disable_bare_mode(&preview).is_err());
    assert_eq!(storage::tree_digest(&root).unwrap(), before);
    assert!(!root.join(".git").exists());
    assert!(!storage::journal_path(&root).unwrap().exists());
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".bare-checkout-")
    }));
}

#[cfg(unix)]
#[test]
fn cancellation_during_required_smudge_preserves_source_and_removes_staging() {
    let (directory, root) = bare_fixture();
    let checkout = directory.path().join("checkout");
    fs::write(
        checkout.join(".gitattributes"),
        "tracked.txt filter=waiting\n",
    )
    .unwrap();
    git_success(&checkout, &["add", ".gitattributes"]);
    git_success(&checkout, &["commit", "-m", "waiting filter"]);
    git_success(
        &root,
        &["fetch", checkout.to_str().unwrap(), "+HEAD:refs/heads/main"],
    );
    git_success(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_success(
        &root,
        &[
            "config",
            "filter.waiting.smudge",
            "printf started > ../filter-started; sleep 30; cat",
        ],
    );
    git_success(&root, &["config", "filter.waiting.required", "true"]);
    let before = storage::tree_digest(&root).unwrap();
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        tx.send(repo.disable_bare_mode_cancellable(&preview, &worker_cancellation))
            .unwrap();
    });
    let started = directory.path().join("filter-started");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !started.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    cancellation.cancel();
    let result = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    worker.join().unwrap();
    assert!(started.exists(), "required filter did not start");
    assert!(matches!(result.unwrap_err().kind(), ErrorKind::Cancelled));
    assert_eq!(storage::tree_digest(&root).unwrap(), before);
    assert!(!storage::journal_path(&root).unwrap().exists());
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".bare-checkout-")
    }));
}

#[test]
fn relocation_uses_native_working_tree_encoding() {
    let (directory, root) = bare_fixture();
    let checkout = directory.path().join("checkout");
    fs::write(
        checkout.join(".gitattributes"),
        "encoded.txt working-tree-encoding=UTF-16LE\n",
    )
    .unwrap();
    let encoded = "hello\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    fs::write(checkout.join("encoded.txt"), &encoded).unwrap();
    git_success(&checkout, &["add", "."]);
    git_success(&checkout, &["commit", "-m", "encoding"]);
    git_success(
        &root,
        &["fetch", checkout.to_str().unwrap(), "+HEAD:refs/heads/main"],
    );
    git_success(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(fs::read(root.join("encoded.txt")).unwrap(), encoded);
    assert_eq!(git_stdout(&root, &["status", "--porcelain=v1"]), "");
}

#[cfg(unix)]
#[test]
fn relocation_preserves_dangling_and_outward_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;
    let (directory, root) = bare_fixture();
    let checkout = directory.path().join("checkout");
    let outside = directory.path().join("outside.txt");
    fs::write(&outside, "external bytes\n").unwrap();
    let dangling_target = Path::new("missing-target");
    let outside_target = Path::new("../outside.txt");
    symlink(dangling_target, checkout.join("dangling")).unwrap();
    symlink(outside_target, checkout.join("outward")).unwrap();
    git_success(&checkout, &["add", "."]);
    git_success(&checkout, &["commit", "-m", "symbolic links"]);
    git_success(
        &root,
        &["fetch", checkout.to_str().unwrap(), "+HEAD:refs/heads/main"],
    );
    git_success(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    let outside_before = fs::metadata(&outside).unwrap();
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(
        fs::read_link(root.join("dangling")).unwrap(),
        dangling_target
    );
    assert_eq!(fs::read_link(root.join("outward")).unwrap(), outside_target);
    assert!(!root.join("dangling").exists());
    assert_eq!(fs::read(&outside).unwrap(), b"external bytes\n");
    assert_eq!(
        fs::metadata(&outside).unwrap().modified().unwrap(),
        outside_before.modified().unwrap()
    );
    assert_eq!(git_stdout(&root, &["status", "--porcelain=v1"]), "");
}

#[cfg(windows)]
#[test]
fn windows_conversion_preserves_readonly_source_objects_while_syncing_private_copies() {
    let (directory, root) = bare_fixture();
    let oid = git_stdout(&root, &["rev-parse", "HEAD"]);
    let relative = PathBuf::from("objects").join(&oid[..2]).join(&oid[2..]);
    let source_object = root.join(&relative);
    let mut permissions = fs::metadata(&source_object).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source_object, permissions).unwrap();
    let original = fs::read(&source_object).unwrap();
    let retained_object = directory.path().join("checkout/.git").join(&relative);
    let retained_readonly = fs::metadata(&retained_object)
        .unwrap()
        .permissions()
        .readonly();
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(fs::read(root.join("tracked.txt")).unwrap(), b"original\n");
    assert_eq!(
        fs::read(root.join(".git").join(&relative)).unwrap(),
        original
    );
    assert!(
        fs::metadata(root.join(".git").join(&relative))
            .unwrap()
            .permissions()
            .readonly()
    );
    assert_eq!(fs::read(&retained_object).unwrap(), original);
    assert!(!same_file::is_same_file(root.join(".git").join(&relative), &retained_object).unwrap());
    assert_eq!(
        fs::metadata(retained_object)
            .unwrap()
            .permissions()
            .readonly(),
        retained_readonly
    );
}

#[test]
fn conversion_waits_for_owned_operation_lease_before_mutating() {
    let (_directory, root) = bare_fixture();
    let repo = std::sync::Arc::new(open_repo(&root));
    let preview = repo.preview_disable_bare_mode().unwrap();
    let lease = repo.acquire_operation_lease().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = repo.clone();
    let thread = std::thread::spawn(move || tx.send(worker.disable_bare_mode(&preview)).unwrap());
    wait_until_converting(&repo);
    assert!(
        matches!(repo.acquire_operation_lease(), Err(error) if matches!(error.kind(), ErrorKind::RepositoryChanged))
    );
    assert!(!root.join(".git").exists());
    assert!(matches!(
        rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    drop(lease);
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    thread.join().unwrap();
    assert_eq!(native_mode(&root).unwrap(), RepositoryMode::WorkingTree);
}

#[test]
fn cancellation_while_waiting_for_operations_releases_conversion_gate() {
    let (_directory, root) = bare_fixture();
    let repo = std::sync::Arc::new(open_repo(&root));
    let preview = repo.preview_disable_bare_mode().unwrap();
    let lease = repo.acquire_operation_lease().unwrap();
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let worker = repo.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        tx.send(worker.disable_bare_mode_cancellable(&preview, &worker_token))
            .unwrap()
    });
    wait_until_converting(&repo);
    token.cancel();
    let outcome = rx.recv_timeout(std::time::Duration::from_secs(5));
    drop(lease);
    thread.join().unwrap();
    assert!(matches!(
        outcome.unwrap().unwrap_err().kind(),
        ErrorKind::Cancelled
    ));
    assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::Bare);
    assert!(!root.join(".git").exists());
}

fn wait_until_converting(repo: &GixRepo) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if matches!(repo.ensure_current(), Err(error) if matches!(error.kind(), ErrorKind::RepositoryChanged))
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "conversion did not enter exclusive wait"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
