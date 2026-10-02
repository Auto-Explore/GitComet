use super::*;
use gitcomet_core::services::CancellationToken;
use std::path::Path;

fn git(path: &Path, args: &[&str]) -> Vec<u8> {
    let output = crate::util::git_workdir_cmd_for(path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // Empty = no global config everywhere; Git for Windows dies on "NUL".
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_DATE", "1800000000 +0530")
        .env("GIT_TEST_REFTABLE_AUTOCOMPACTION", "0")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git_reftable_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let available = crate::util::git_workdir_cmd_for(dir.path())
            .args(["init", "--ref-format=reftable"])
            .output()
            .unwrap()
            .status
            .success();
        if !available {
            eprintln!("skipping Git cross-check: reftable is unavailable");
        }
        available
    })
}

fn fixture(hash: &str, block: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(
        dir.path(),
        &[
            "init",
            &format!("--object-format={hash}"),
            "--ref-format=reftable",
            "--initial-branch=main",
        ],
    );
    git(dir.path(), &["config", "user.name", "Reader Test"]);
    git(
        dir.path(),
        &["config", "user.email", "reader@example.invalid"],
    );
    git(dir.path(), &["config", "reftable.blockSize", block]);
    git(dir.path(), &["config", "reftable.restartInterval", "3"]);
    git(dir.path(), &["commit", "--allow-empty", "-m", "root"]);
    dir
}

#[test]
fn tables_and_logs_match_git_across_hashes_blocks_and_compaction() {
    if !git_reftable_available() {
        return;
    }
    for (format, hash) in [
        ("sha1", gix::hash::Kind::Sha1),
        ("sha256", gix::hash::Kind::Sha256),
    ] {
        for block in ["256", "512", "1000", "4096", "65536"] {
            let dir = fixture(format, block);
            let mut cmd = crate::util::git_workdir_cmd_for(dir.path());
            cmd.env("GIT_TEST_REFTABLE_AUTOCOMPACTION", "0")
                .args(["update-ref", "--stdin"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null());
            let mut child = cmd.spawn().unwrap();
            {
                use std::io::Write as _;
                let mut stdin = child.stdin.take().unwrap();
                for n in 0..3000 {
                    writeln!(stdin, "create refs/heads/branch-{n:04} HEAD").unwrap();
                }
            }
            assert!(child.wait().unwrap().success());
            git(
                dir.path(),
                &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
            );
            git(dir.path(), &["tag", "-a", "inner", "-m", "inner"]);
            git(dir.path(), &["tag", "-a", "outer", "inner", "-m", "outer"]);
            let stack_dir = dir.path().join(".git/reftable");
            for compact in [false, true] {
                if compact {
                    git(dir.path(), &["pack-refs", "--all"]);
                }
                let stack =
                    Stack::load(&stack_dir, hash, false, &CancellationToken::new()).unwrap();
                let expected = git(
                    dir.path(),
                    &[
                        "for-each-ref",
                        "--include-root-refs",
                        "--format=%(refname)%00%(objectname)%00%(symref)",
                    ],
                );
                for line in expected.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                    let fields: Vec<_> = line.split(|b| *b == 0).collect();
                    let record = &stack.refs[fields[0]];
                    match &record.target {
                        gix::refs::Target::Object(id) => {
                            assert_eq!(id.to_string().as_bytes(), fields[1])
                        }
                        gix::refs::Target::Symbolic(name) => assert_eq!(name.as_bstr(), fields[2]),
                    }
                }
                assert_eq!(
                    stack.refs.len(),
                    expected
                        .split(|b| *b == b'\n')
                        .filter(|l| !l.is_empty())
                        .count()
                );
                let log = stack
                    .reflog(b"HEAD", Some(10), &CancellationToken::new())
                    .unwrap();
                assert_eq!(log.len(), 1);
                assert_eq!(log[0].signature.time.seconds, 1800000000);
                assert_eq!(log[0].signature.time.offset, 19800);
                assert_eq!(log[0].message, "commit (initial): root");
                let cached =
                    Stack::load(&stack_dir, hash, false, &CancellationToken::new()).unwrap();
                assert!(std::sync::Arc::ptr_eq(&stack, &cached));
            }
            git(dir.path(), &["update-ref", "-d", "refs/heads/branch-0001"]);
            let stack = Stack::load(&stack_dir, hash, false, &CancellationToken::new()).unwrap();
            assert!(
                !stack
                    .refs
                    .contains_key(b"refs/heads/branch-0001".as_slice())
            );
        }
    }
}

#[test]
fn table_truncation_and_mutation_never_panic() {
    let path = std::path::PathBuf::from("sha256.ref");
    let bytes = include_bytes!("../../../tests/fixtures/reftable/sha256.ref").to_vec();
    let cancel = CancellationToken::new();
    for end in 0..bytes.len() {
        assert!(
            table::Table::parse(
                path.clone(),
                bytes[..end].into(),
                gix::hash::Kind::Sha256,
                &cancel
            )
            .is_err(),
            "truncation at {end}"
        );
    }
    for offset in 0..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[offset] ^= 0x80;
        if let Ok(table) = table::Table::parse(
            path.clone(),
            damaged.into(),
            gix::hash::Kind::Sha256,
            &cancel,
        ) {
            let _ = table.logs(b"HEAD", &cancel);
        }
    }
    assert!(table::Table::parse(path, bytes.into(), gix::hash::Kind::Sha1, &cancel).is_err());
}

#[test]
fn git_binary_fixtures_decode_without_a_git_executable() {
    for (hash, bytes, expected) in [
        (
            gix::hash::Kind::Sha1,
            include_bytes!("../../../tests/fixtures/reftable/sha1.ref").as_slice(),
            include_str!("../../../tests/fixtures/reftable/sha1.expected"),
        ),
        (
            gix::hash::Kind::Sha256,
            include_bytes!("../../../tests/fixtures/reftable/sha256.ref").as_slice(),
            include_str!("../../../tests/fixtures/reftable/sha256.expected"),
        ),
    ] {
        let table = table::Table::parse(
            "fixture.ref".into(),
            bytes.into(),
            hash,
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(table.refs.len(), 2);
        for line in expected.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let entry = table.refs[fields[0].as_bytes()].as_ref().unwrap();
            match &entry.target {
                gix::refs::Target::Object(id) => assert_eq!(id.to_string(), fields[1]),
                gix::refs::Target::Symbolic(name) => assert_eq!(name.as_bstr(), fields[2]),
            }
        }
        let logs = table.logs(b"HEAD", &CancellationToken::new()).unwrap();
        assert_eq!(logs.len(), 1);
        let log = logs[0].1.as_ref().unwrap();
        assert_eq!(log.signature.time.seconds, 1800000000);
        assert_eq!(log.signature.time.offset, 19800);
        assert_eq!(log.message, "commit (initial): root");
    }
}

#[test]
fn missing_stack_is_only_empty_for_optional_worktree_storage() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    assert!(Stack::load(dir.path(), gix::hash::Kind::Sha1, false, &cancel).is_err());
    assert!(
        Stack::load(dir.path(), gix::hash::Kind::Sha1, true, &cancel)
            .unwrap()
            .refs
            .is_empty()
    );
    std::fs::write(dir.path().join("tables.list"), b"missing.ref\n").unwrap();
    assert!(Stack::load(dir.path(), gix::hash::Kind::Sha1, true, &cancel).is_err());
    std::fs::write(dir.path().join("tables.list"), b"../escape.ref\n").unwrap();
    assert!(Stack::load(dir.path(), gix::hash::Kind::Sha1, true, &cancel).is_err());
}

#[test]
fn native_reads_stay_in_process_and_ref_writes_keep_raw_expectations() {
    if !git_reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        let dir = fixture(hash, "512");
        let repo = crate::open::open_worktree_repo(dir.path()).unwrap();
        let root = super::super::head_oid(&repo).unwrap().unwrap();
        git(dir.path(), &["commit", "--allow-empty", "-m", "child"]);
        let head = super::super::head_oid(&repo).unwrap().unwrap();
        git(
            dir.path(),
            &["update-ref", "refs/heads/topic", &root.to_string()],
        );
        let observed = super::super::find_required(&repo, "refs/heads/topic").unwrap();
        git(
            dir.path(),
            &["update-ref", "refs/heads/topic", &head.to_string()],
        );
        assert!(super::super::delete(observed).is_err());
        assert_eq!(
            super::super::find_required(&repo, "topic")
                .unwrap()
                .id()
                .detach(),
            head
        );
        git(
            dir.path(),
            &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
        );
        let observed = super::super::find_required(&repo, "refs/heads/alias").unwrap();
        git(
            dir.path(),
            &["symbolic-ref", "refs/heads/alias", "refs/heads/topic"],
        );
        assert!(super::super::delete(observed).is_err());
        let observed = super::super::find_required(&repo, "refs/heads/alias").unwrap();
        super::super::delete(observed).unwrap();
        assert!(
            super::super::find(&repo, "refs/heads/alias")
                .unwrap()
                .is_none()
        );
        assert!(
            super::super::find(&repo, "refs/heads/topic")
                .unwrap()
                .is_some()
        );
        git(
            dir.path(),
            &["symbolic-ref", "refs/heads/dangling", "refs/heads/missing"],
        );
        assert!(super::super::delete_named(&repo, "refs/heads/dangling").unwrap());
        assert!(!super::super::delete_named(&repo, "refs/heads/dangling").unwrap());
        super::super::create(&repo, "refs/heads/batch-a", root, "test").unwrap();
        super::super::create(&repo, "refs/heads/batch-b", root, "test").unwrap();
        let observed = super::super::view(&repo).unwrap();
        let mut commands = Vec::new();
        for name in ["refs/heads/batch-a", "refs/heads/batch-b"] {
            let entry = observed.find_exact(name).unwrap().unwrap();
            super::super::write::encode_delete(
                &mut commands,
                name.as_bytes(),
                entry.target.to_ref(),
            );
        }
        git(
            dir.path(),
            &["update-ref", "refs/heads/batch-b", &head.to_string()],
        );
        assert!(super::super::write::transaction(&repo, commands, None).is_err());
        assert_eq!(
            super::super::find_required(&repo, "batch-a")
                .unwrap()
                .id()
                .detach(),
            root
        );
        assert_eq!(
            super::super::find_required(&repo, "batch-b")
                .unwrap()
                .id()
                .detach(),
            head
        );
        super::super::delete_batch(
            &repo,
            &[
                "refs/heads/batch-a",
                "refs/heads/batch-b",
                "refs/heads/batch-a",
            ],
        )
        .unwrap();
        assert!(super::super::find(&repo, "batch-a").unwrap().is_none());
        assert!(super::super::find(&repo, "batch-b").unwrap().is_none());
        git(dir.path(), &["config", "core.filesRefLockTimeout", "0"]);
        let lock = repo.git_dir().join("reftable/tables.list.lock");
        std::fs::write(&lock, []).unwrap();
        let error = super::super::create(&repo, "refs/heads/locked", root, "test").unwrap_err();
        assert!(error.to_string().contains("lock"), "{error}");
        std::fs::remove_file(lock).unwrap();
        assert!(super::super::find(&repo, "locked").unwrap().is_none());
        let (_, commands) = crate::command_trace::capture(|| {
            let refs = super::super::view(&repo).unwrap();
            assert_eq!(refs.head_oid().unwrap(), Some(head));
            assert_eq!(refs.resolve("HEAD~1").unwrap(), Some(root));
            assert_eq!(refs.resolve(&head.to_string()[..12]).unwrap(), Some(head));
            assert_eq!(refs.resolve("main@{1}").unwrap(), Some(root));
            refs.local_branches()
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            refs.reflog("HEAD", Some(2)).unwrap();
        });
        assert!(commands.is_empty(), "native read spawned: {commands:?}");
    }
}

#[test]
fn private_worktree_refs_do_not_leak_and_explicit_aliases_route_correctly() {
    if !git_reftable_available() {
        return;
    }
    let dir = fixture("sha256", "512");
    let parent = tempfile::tempdir().unwrap();
    let linked = parent.path().join("linked");
    git(
        dir.path(),
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    git(&linked, &["commit", "--allow-empty", "-m", "linked"]);
    let main = crate::open::open_worktree_repo(dir.path()).unwrap();
    let other = crate::open::open_worktree_repo(&linked).unwrap();
    let main_id = super::super::head_oid(&main).unwrap().unwrap();
    let other_id = super::super::head_oid(&other).unwrap().unwrap();
    for name in [
        "refs/worktree/y",
        "refs/bisect/x",
        "refs/rewritten/z",
        "ORIG_HEAD",
    ] {
        git(dir.path(), &["update-ref", name, &main_id.to_string()]);
        git(&linked, &["update-ref", name, &other_id.to_string()]);
        let refs = super::super::view(&other).unwrap();
        assert_eq!(refs.resolve(name).unwrap(), Some(other_id));
        assert_eq!(
            refs.resolve(&format!("main-worktree/{name}")).unwrap(),
            Some(main_id)
        );
        let id = other.git_dir().file_name().unwrap().to_str().unwrap();
        assert_eq!(
            super::super::view(&main)
                .unwrap()
                .resolve(&format!("worktrees/{id}/{name}"))
                .unwrap(),
            Some(other_id)
        );
    }
    git(
        &linked,
        &["symbolic-ref", "refs/worktree/alias", "refs/worktree/y"],
    );
    let worktree_id = other.git_dir().file_name().unwrap().to_str().unwrap();
    let spec = format!("worktrees/{worktree_id}/refs/worktree/alias");
    let expected = git(dir.path(), &["rev-parse", "--verify", &spec]);
    assert_eq!(
        super::super::resolve(&main, &spec)
            .unwrap()
            .unwrap()
            .to_string()
            .as_bytes(),
        expected.trim_ascii()
    );
    for (repo, path) in [(&main, dir.path()), (&other, linked.as_path())] {
        let refs = super::super::view(repo).unwrap();
        let actual: String = refs
            .all()
            .unwrap()
            .map(|reference| {
                let reference = reference.unwrap();
                format!("{} {}\n", reference.name().as_bstr(), reference.id())
            })
            .collect();
        let expected = git(path, &["for-each-ref", "--format=%(refname) %(objectname)"]);
        assert_eq!(
            actual.as_bytes(),
            expected,
            "merged worktree refs must retain Git's order"
        );
    }
    for name in ["FETCH_HEAD", "MERGE_HEAD"] {
        std::fs::write(
            main.git_dir().join(name),
            format!("{main_id}\n{other_id}\n"),
        )
        .unwrap();
        std::fs::write(
            other.git_dir().join(name),
            format!("{other_id}\n{main_id}\n"),
        )
        .unwrap();
        let id = other.git_dir().file_name().unwrap().to_str().unwrap();
        for (repo, workdir, spec) in [
            (&main, dir.path(), format!("worktrees/{id}/{name}")),
            (&other, linked.as_path(), format!("main-worktree/{name}")),
        ] {
            // File pseudorefs have no cross-worktree alias in Git's ref store.
            let output = crate::util::git_workdir_cmd_for(workdir)
                .args(["rev-parse", "--verify", &spec])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(super::super::resolve(repo, &spec).unwrap().is_none());
            let expected = git(workdir, &["rev-parse", "--verify", name]);
            assert_eq!(
                super::super::resolve(repo, name)
                    .unwrap()
                    .unwrap()
                    .to_string()
                    .as_bytes(),
                expected.trim_ascii()
            );
        }
    }
    git(
        dir.path(),
        &["symbolic-ref", "refs/heads/cycle-a", "refs/heads/cycle-b"],
    );
    git(
        dir.path(),
        &["symbolic-ref", "refs/heads/cycle-b", "refs/heads/cycle-a"],
    );
    assert!(super::super::find(&main, "cycle-a").is_err());
    git(
        dir.path(),
        &["symbolic-ref", "refs/heads/dangling", "refs/heads/missing"],
    );
    assert!(super::super::find(&main, "dangling").unwrap().is_none());
    assert!(
        super::super::view(&main)
            .unwrap()
            .find_exact(b"refs/heads/dangling")
            .unwrap()
            .is_some()
    );
}

#[test]
fn concurrent_compaction_produces_complete_snapshots_or_a_retry_error() {
    if !git_reftable_available() {
        return;
    }
    let dir = fixture("sha1", "256");
    let repo = crate::open::open_worktree_repo(dir.path()).unwrap();
    let id = super::super::head_oid(&repo).unwrap().unwrap();
    let root = dir.path().to_path_buf();
    let writer = std::thread::spawn(move || {
        for n in 0..30 {
            git(
                &root,
                &[
                    "update-ref",
                    &format!("refs/heads/race-{n}"),
                    &id.to_string(),
                ],
            );
            git(&root, &["pack-refs", "--all"]);
        }
    });
    for _ in 0..150 {
        match super::super::view(&repo) {
            Ok(refs) => {
                assert_eq!(refs.head_oid().unwrap(), Some(id));
                for reference in refs.local_branches().unwrap() {
                    assert_eq!(reference.unwrap().id().detach(), id);
                }
            }
            Err(error) => assert!(error.to_string().contains("stack kept changing"), "{error}"),
        }
    }
    writer.join().unwrap();
}

#[test]
fn revision_expressions_match_git_without_rewriting_unsupported_grammar() {
    if !git_reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        let dir = fixture(hash, "512");
        std::fs::write(dir.path().join("tracked.txt"), "base\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "tracked"]);
        git(dir.path(), &["checkout", "-b", "topic"]);
        git(
            dir.path(),
            &["commit", "--allow-empty", "-m", "topic commit"],
        );
        git(dir.path(), &["checkout", "main"]);
        git(
            dir.path(),
            &["commit", "--allow-empty", "-m", "main commit"],
        );
        git(dir.path(), &["merge", "--no-ff", "topic", "-m", "merge"]);
        git(dir.path(), &["tag", "-a", "tag", "-m", "annotated"]);
        git(dir.path(), &["tag", "collision", "HEAD~1"]);
        git(dir.path(), &["branch", "collision"]);
        git(dir.path(), &["branch", "branch@"]);
        git(dir.path(), &["branch", "--set-upstream-to=topic", "main"]);
        for n in 0..2 {
            std::fs::write(dir.path().join("tracked.txt"), format!("stash {n}\n")).unwrap();
            git(dir.path(), &["stash", "push", "-m", &format!("stash {n}")]);
        }
        let repo = crate::open::open_worktree_repo(dir.path()).unwrap();
        let refs = super::super::view(&repo).unwrap();
        for spec in [
            "HEAD",
            "@",
            "HEAD~2",
            "HEAD^2",
            "tag^{commit}",
            "tag^{tree}",
            "tag^{}",
            "stash@{1}",
            "@{0}",
            "@{1}",
            "HEAD@{1}",
            "main@{1}",
            "HEAD:tracked.txt",
            ":0:tracked.txt",
            "HEAD@{2027-01-15 08:00:00 +0000}",
            "@{u}",
            "@{-1}",
            ":/topic commit",
            "HEAD^{/topic}",
            "collision",
            "branch@",
        ] {
            let expected = git(
                dir.path(),
                &["rev-parse", "--verify", "--end-of-options", spec],
            );
            assert_eq!(
                refs.resolve(spec).unwrap().unwrap().to_string().as_bytes(),
                expected.trim_ascii(),
                "{hash} {spec}"
            );
        }
        // A hexadecimal short ref name takes precedence over an object prefix.
        let head = refs.head_oid().unwrap().unwrap().to_string();
        git(dir.path(), &["tag", &head[..12], "HEAD~1"]);
        let expected = git(dir.path(), &["rev-parse", "--verify", &head[..12]]);
        assert_eq!(
            super::super::resolve(&repo, &head[..12])
                .unwrap()
                .unwrap()
                .to_string()
                .as_bytes(),
            expected.trim_ascii()
        );
        #[cfg(unix)]
        {
            std::fs::write(dir.path().join("with:colon"), "colon\n").unwrap();
            git(dir.path(), &["add", "with:colon"]);
            git(dir.path(), &["commit", "-m", "colon path"]);
            let expected = git(dir.path(), &["rev-parse", "HEAD:with:colon"]);
            assert_eq!(
                super::super::resolve(&repo, "HEAD:with:colon")
                    .unwrap()
                    .unwrap()
                    .to_string()
                    .as_bytes(),
                expected.trim_ascii()
            );
        }
    }
}

#[test]
fn storage_detection_uses_only_top_level_repository_configuration() {
    if !git_reftable_available() {
        return;
    }
    let dir = fixture("sha1", "512");
    std::fs::write(
        dir.path().join(".git/included"),
        "[extensions]\nrefStorage = files\n",
    )
    .unwrap();
    git(dir.path(), &["config", "include.path", "included"]);
    let repo = gix::open_opts(
        dir.path(),
        gix::open::Options::default().config_overrides(["extensions.refStorage=files"]),
    )
    .unwrap();
    assert_eq!(
        super::super::backend(&repo).unwrap(),
        super::super::RefBackend::Reftable
    );
    let config = dir.path().join(".git/config");
    let contents = std::fs::read_to_string(&config)
        .unwrap()
        .replace("refstorage = reftable", "refstorage = future");
    std::fs::write(config, contents).unwrap();
    let repo = crate::open::open_worktree_repo(dir.path()).unwrap();
    assert!(
        super::super::backend(&repo)
            .unwrap_err()
            .to_string()
            .contains("future")
    );
}

#[cfg(unix)]
#[test]
fn reference_names_preserve_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt as _;
    if !git_reftable_available() {
        return;
    }
    let dir = fixture("sha256", "512");
    let name = b"refs/heads/non-utf8-\xff";
    let output = crate::util::git_workdir_cmd_for(dir.path())
        .arg("update-ref")
        .arg(std::ffi::OsStr::from_bytes(name))
        .arg("HEAD")
        .output()
        .unwrap();
    assert!(output.status.success());
    let repo = crate::open::open_worktree_repo(dir.path()).unwrap();
    let refs = super::super::view(&repo).unwrap();
    assert_eq!(
        refs.find(name).unwrap().unwrap().name().as_bstr(),
        name.as_slice()
    );
    assert!(
        refs.local_branches()
            .unwrap()
            .any(|r| r.unwrap().name().as_bstr() == name.as_slice())
    );
}

#[test]
fn reference_iteration_filters_prefixes_and_observes_cancellation_after_creation() {
    if !git_reftable_available() {
        return;
    }
    let dir = fixture("sha256", "512");
    git(
        dir.path(),
        &["update-ref", "refs/heads-neighbor/other", "HEAD"],
    );
    git(
        dir.path(),
        &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
    );
    git(
        dir.path(),
        &["symbolic-ref", "refs/heads/dangling", "refs/heads/missing"],
    );
    let repo = crate::open::open_worktree_repo(dir.path()).unwrap();
    let cancel = CancellationToken::new();
    let refs = super::super::view_cancellable(&repo, &cancel).unwrap();
    let names = refs
        .local_branches()
        .unwrap()
        .map(|reference| reference.unwrap().name().as_bstr().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [b"refs/heads/alias".to_vec(), b"refs/heads/main".to_vec()]
    );
    let mut iter = refs.local_branches().unwrap();
    assert!(iter.next().unwrap().is_ok());
    cancel.cancel();
    let error = iter
        .next()
        .unwrap()
        .err()
        .expect("iteration must observe cancellation");
    assert!(matches!(
        error.kind(),
        gitcomet_core::error::ErrorKind::Cancelled
    ));
}

// A minimal independent encoder for structures Git normally compacts away.
// It deliberately has no production use and doesn't share decoder helpers.
fn varint(mut value: u64, out: &mut Vec<u8>) {
    let mut bytes = vec![(value & 127) as u8];
    while value >= 128 {
        value = (value >> 7) - 1;
        bytes.push(128 | (value & 127) as u8);
    }
    out.extend(bytes.into_iter().rev());
}
fn u24(value: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&(value as u32).to_be_bytes()[1..]);
}
fn header(version: u8, hash: gix::hash::Kind, alignment: usize) -> Vec<u8> {
    let mut h = b"REFT".to_vec();
    h.push(version);
    u24(alignment, &mut h);
    h.extend_from_slice(&1u64.to_be_bytes());
    h.extend_from_slice(&1u64.to_be_bytes());
    if version == 2 {
        h.extend_from_slice(if hash == gix::hash::Kind::Sha1 {
            b"sha1"
        } else {
            b"s256"
        });
    }
    h
}
fn footer(h: &[u8], out: &mut Vec<u8>) {
    let mut f = h.to_vec();
    f.extend_from_slice(&[0; 40]);
    f.extend_from_slice(&crc32fast::hash(&f).to_be_bytes());
    out.extend(f);
}

#[test]
fn encoder_covers_v2_sha1_empty_unaligned_and_non_power_of_two_tables() {
    let cancel = CancellationToken::new();
    for hash in [gix::hash::Kind::Sha1, gix::hash::Kind::Sha256] {
        for alignment in [0, 256, 1000] {
            let h = header(2, hash, alignment);
            let mut empty = h.clone();
            footer(&h, &mut empty);
            assert!(
                table::Table::parse("empty".into(), empty.into(), hash, &cancel)
                    .unwrap()
                    .refs
                    .is_empty()
            );
            let mut bytes = h.clone();
            for n in 0..4 {
                let block_start = if n == 0 { 0 } else { bytes.len() };
                let first = bytes.len() - block_start + 4;
                let mut block = vec![b'r', 0, 0, 0];
                let name = format!("refs/heads/{n}");
                varint(0, &mut block);
                varint(((name.len() as u64) << 3) | 1, &mut block);
                block.extend_from_slice(name.as_bytes());
                varint(0, &mut block);
                block.extend(vec![1; hash.len_in_bytes()]);
                u24(first, &mut block);
                block.extend_from_slice(&1u16.to_be_bytes());
                let len = bytes.len() - block_start + block.len();
                block[1..4].copy_from_slice(&(len as u32).to_be_bytes()[1..]);
                bytes.extend(block);
                if n != 3 && alignment != 0 {
                    bytes.resize(bytes.len().div_ceil(alignment) * alignment, 0);
                }
            }
            footer(&h, &mut bytes);
            let parsed =
                table::Table::parse("unaligned".into(), bytes.into(), hash, &cancel).unwrap();
            assert_eq!(parsed.refs.len(), 4);
        }
    }
}

#[test]
fn log_only_empty_reflog_marker_and_signed_timezone() {
    let hash = gix::hash::Kind::Sha256;
    let h = header(2, hash, 0);
    for (old, new, timezone, message) in [
        (0, 0, 0i16, b"".as_slice()),
        (0, 1, -130, b"message\n"),
        (1, 1, 530, b"no-newline"),
    ] {
        let mut payload = Vec::new();
        let key = [b"HEAD\0".as_slice(), &(u64::MAX - 1).to_be_bytes()].concat();
        varint(0, &mut payload);
        varint(((key.len() as u64) << 3) | 1, &mut payload);
        payload.extend(key);
        payload.extend(vec![old; 32]);
        payload.extend(vec![new; 32]);
        for text in [b"Test".as_slice(), b"test@example.invalid"] {
            varint(text.len() as u64, &mut payload);
            payload.extend(text);
        }
        varint(1800000000, &mut payload);
        payload.extend(timezone.to_be_bytes());
        varint(message.len() as u64, &mut payload);
        payload.extend(message);
        u24(h.len() + 4, &mut payload);
        payload.extend(1u16.to_be_bytes());
        use std::io::Write as _;
        let mut compressor =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        compressor.write_all(&payload).unwrap();
        let mut bytes = h.clone();
        bytes.push(b'g');
        u24(h.len() + 4 + payload.len(), &mut bytes);
        bytes.extend(compressor.finish().unwrap());
        footer(&h, &mut bytes);
        let table = table::Table::parse(
            "logs-only".into(),
            bytes.clone().into(),
            hash,
            &CancellationToken::new(),
        )
        .unwrap();
        let logs = table.logs(b"HEAD", &CancellationToken::new()).unwrap();
        let line = logs[0].1.as_ref().unwrap();
        assert_eq!(
            line.signature.time.offset,
            match timezone {
                -130 => -5400,
                530 => 19800,
                _ => 0,
            }
        );
        assert_eq!(
            line.message.as_slice(),
            message.strip_suffix(b"\n").unwrap_or(message)
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("logs.ref"), &bytes).unwrap();
        std::fs::write(dir.path().join("tables.list"), b"logs.ref\n").unwrap();
        let stack = Stack::load(dir.path(), hash, false, &CancellationToken::new()).unwrap();
        assert_eq!(
            stack
                .reflog(b"HEAD", Some(1), &CancellationToken::new())
                .unwrap()
                .len(),
            usize::from(old != 0 || new != 0)
        );
    }
}
