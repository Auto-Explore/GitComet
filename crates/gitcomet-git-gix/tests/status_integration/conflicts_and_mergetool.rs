use super::*;

#[test]
fn status_and_conflict_stages_cover_all_conflict_kinds() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let base_blob = hash_blob(repo, b"base\n");
    let ours_blob = hash_blob(repo, b"ours\n");
    let theirs_blob = hash_blob(repo, b"theirs\n");

    let fixtures = [
        ConflictStageFixture {
            path: "dd.txt",
            kind: FileConflictKind::BothDeleted,
            has_base: true,
            has_ours: false,
            has_theirs: false,
        },
        ConflictStageFixture {
            path: "au.txt",
            kind: FileConflictKind::AddedByUs,
            has_base: false,
            has_ours: true,
            has_theirs: false,
        },
        ConflictStageFixture {
            path: "ud.txt",
            kind: FileConflictKind::DeletedByThem,
            has_base: true,
            has_ours: true,
            has_theirs: false,
        },
        ConflictStageFixture {
            path: "ua.txt",
            kind: FileConflictKind::AddedByThem,
            has_base: false,
            has_ours: false,
            has_theirs: true,
        },
        ConflictStageFixture {
            path: "du.txt",
            kind: FileConflictKind::DeletedByUs,
            has_base: true,
            has_ours: false,
            has_theirs: true,
        },
        ConflictStageFixture {
            path: "aa.txt",
            kind: FileConflictKind::BothAdded,
            has_base: false,
            has_ours: true,
            has_theirs: true,
        },
        ConflictStageFixture {
            path: "uu.txt",
            kind: FileConflictKind::BothModified,
            has_base: true,
            has_ours: true,
            has_theirs: true,
        },
    ];

    for fixture in &fixtures {
        set_unmerged_stages(
            repo,
            fixture.path,
            fixture.has_base.then_some(base_blob.as_str()),
            fixture.has_ours.then_some(ours_blob.as_str()),
            fixture.has_theirs.then_some(theirs_blob.as_str()),
        );
    }

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();

    for fixture in &fixtures {
        let path = Path::new(fixture.path);
        let status_entry = status
            .unstaged
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("missing status entry for {}", fixture.path));
        assert_eq!(
            status_entry.kind,
            FileStatusKind::Conflicted,
            "expected conflicted kind for {}",
            fixture.path
        );
        assert_eq!(
            status_entry.conflict,
            Some(fixture.kind),
            "wrong conflict kind for {}",
            fixture.path
        );

        assert!(
            !status.staged.iter().any(|e| e.path == path),
            "conflicted path {} should not appear in staged status",
            fixture.path
        );

        let stages = opened
            .conflict_file_stages(path)
            .unwrap()
            .expect("conflict stages");
        assert_eq!(
            stages.base.is_some(),
            fixture.has_base,
            "base stage mismatch for {}",
            fixture.path
        );
        if stages.base.is_some() {
            assert!(
                stages.base_bytes.is_none(),
                "utf-8 base stage should not retain duplicate bytes for {}",
                fixture.path
            );
        }
        assert_eq!(
            stages.ours.is_some(),
            fixture.has_ours,
            "ours stage mismatch for {}",
            fixture.path
        );
        if stages.ours.is_some() {
            assert!(
                stages.ours_bytes.is_none(),
                "utf-8 ours stage should not retain duplicate bytes for {}",
                fixture.path
            );
        }
        assert_eq!(
            stages.theirs.is_some(),
            fixture.has_theirs,
            "theirs stage mismatch for {}",
            fixture.path
        );
        if stages.theirs.is_some() {
            assert!(
                stages.theirs_bytes.is_none(),
                "utf-8 theirs stage should not retain duplicate bytes for {}",
                fixture.path
            );
        }

        let session = opened
            .conflict_session(path)
            .unwrap()
            .expect("conflict session");
        assert_eq!(session.path, PathBuf::from(fixture.path));
        assert_eq!(session.conflict_kind, fixture.kind);
        assert_eq!(
            session.strategy,
            ConflictResolverStrategy::for_conflict(fixture.kind, false)
        );
        assert_eq!(session.base.is_absent(), !fixture.has_base);
        assert_eq!(session.ours.is_absent(), !fixture.has_ours);
        assert_eq!(session.theirs.is_absent(), !fixture.has_theirs);
    }
}

#[test]
fn checkout_conflict_side_resolves_all_conflict_stage_shapes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    #[derive(Clone, Copy)]
    struct ConflictCheckoutFixture {
        kind: FileConflictKind,
        has_base: bool,
        has_ours: bool,
        has_theirs: bool,
    }

    let fixtures = [
        ConflictCheckoutFixture {
            kind: FileConflictKind::BothDeleted,
            has_base: true,
            has_ours: false,
            has_theirs: false,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::AddedByUs,
            has_base: false,
            has_ours: true,
            has_theirs: false,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::DeletedByThem,
            has_base: true,
            has_ours: true,
            has_theirs: false,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::AddedByThem,
            has_base: false,
            has_ours: false,
            has_theirs: true,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::DeletedByUs,
            has_base: true,
            has_ours: false,
            has_theirs: true,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::BothAdded,
            has_base: false,
            has_ours: true,
            has_theirs: true,
        },
        ConflictCheckoutFixture {
            kind: FileConflictKind::BothModified,
            has_base: true,
            has_ours: true,
            has_theirs: true,
        },
    ];

    for fixture in fixtures {
        for side in [ConflictSide::Ours, ConflictSide::Theirs] {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path();

            run_git(repo, &["init"]);
            run_git(repo, &["config", "user.email", "you@example.com"]);
            run_git(repo, &["config", "user.name", "You"]);
            run_git(repo, &["config", "commit.gpgsign", "false"]);

            write(repo, "seed.txt", "seed\n");
            run_git(repo, &["add", "seed.txt"]);
            run_git(
                repo,
                &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
            );

            let base_blob = hash_blob(repo, b"base\n");
            let ours_blob = hash_blob(repo, b"ours\n");
            let theirs_blob = hash_blob(repo, b"theirs\n");

            set_unmerged_stages(
                repo,
                "a.txt",
                fixture.has_base.then_some(base_blob.as_str()),
                fixture.has_ours.then_some(ours_blob.as_str()),
                fixture.has_theirs.then_some(theirs_blob.as_str()),
            );

            let backend = GixBackend;
            let opened = backend.open(repo).unwrap();

            let before = opened.status().unwrap();
            let conflict_entry = before
                .unstaged
                .iter()
                .find(|e| e.path == Path::new("a.txt"))
                .expect("expected staged-shape fixture to appear as conflict");
            assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
            assert_eq!(conflict_entry.conflict, Some(fixture.kind));

            opened
                .checkout_conflict_side(Path::new("a.txt"), side)
                .unwrap();

            let after = opened.status().unwrap();
            let selected_stage_exists = match side {
                ConflictSide::Ours => fixture.has_ours,
                ConflictSide::Theirs => fixture.has_theirs,
            };

            if selected_stage_exists {
                let expected_bytes: &[u8] = match side {
                    ConflictSide::Ours => b"ours\n",
                    ConflictSide::Theirs => b"theirs\n",
                };
                assert_eq!(fs::read(repo.join("a.txt")).unwrap(), expected_bytes);
                assert!(
                    after
                        .staged
                        .iter()
                        .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Added),
                    "expected selected side to stage added file for {:?} with {:?}; status={after:?}",
                    fixture.kind,
                    side
                );
                assert!(
                    after.unstaged.iter().all(|e| e.path != Path::new("a.txt")),
                    "expected conflict path to disappear from unstaged after resolving {:?} with {:?}; status={after:?}",
                    fixture.kind,
                    side
                );
            } else {
                assert!(
                    !repo.join("a.txt").exists(),
                    "expected path to be removed when chosen stage is missing for {:?} with {:?}",
                    fixture.kind,
                    side
                );
                assert!(
                    after
                        .staged
                        .iter()
                        .chain(after.unstaged.iter())
                        .all(|e| e.path != Path::new("a.txt")),
                    "expected no status entry for removed path after resolving {:?} with {:?}; status={after:?}",
                    fixture.kind,
                    side
                );
            }
        }
    }
}

#[test]
fn accept_conflict_deletion_resolves_delete_outcome_conflicts() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    #[derive(Clone, Copy)]
    struct ConflictDeleteFixture {
        kind: FileConflictKind,
        has_base: bool,
        has_ours: bool,
        has_theirs: bool,
    }

    let fixtures = [
        ConflictDeleteFixture {
            kind: FileConflictKind::BothDeleted,
            has_base: true,
            has_ours: false,
            has_theirs: false,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::AddedByUs,
            has_base: false,
            has_ours: true,
            has_theirs: false,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::AddedByThem,
            has_base: false,
            has_ours: false,
            has_theirs: true,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::DeletedByUs,
            has_base: true,
            has_ours: false,
            has_theirs: true,
        },
        ConflictDeleteFixture {
            kind: FileConflictKind::DeletedByThem,
            has_base: true,
            has_ours: true,
            has_theirs: false,
        },
    ];

    for fixture in fixtures {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();

        run_git(repo, &["init"]);
        run_git(repo, &["config", "user.email", "you@example.com"]);
        run_git(repo, &["config", "user.name", "You"]);
        run_git(repo, &["config", "commit.gpgsign", "false"]);

        write(repo, "seed.txt", "seed\n");
        run_git(repo, &["add", "seed.txt"]);
        run_git(
            repo,
            &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
        );

        let base_blob = hash_blob(repo, b"base\n");
        let ours_blob = hash_blob(repo, b"ours\n");
        let theirs_blob = hash_blob(repo, b"theirs\n");

        set_unmerged_stages(
            repo,
            "a.txt",
            fixture.has_base.then_some(base_blob.as_str()),
            fixture.has_ours.then_some(ours_blob.as_str()),
            fixture.has_theirs.then_some(theirs_blob.as_str()),
        );

        let backend = GixBackend;
        let opened = backend.open(repo).unwrap();

        let before = opened.status().unwrap();
        let conflict_entry = before
            .unstaged
            .iter()
            .find(|e| e.path == Path::new("a.txt"))
            .expect("expected fixture path to appear as conflict");
        assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
        assert_eq!(conflict_entry.conflict, Some(fixture.kind));

        opened.accept_conflict_deletion(Path::new("a.txt")).unwrap();

        let after = opened.status().unwrap();
        assert!(
            !repo.join("a.txt").exists(),
            "expected path to be removed after accepting deletion for {:?}",
            fixture.kind
        );
        assert!(
            after
                .staged
                .iter()
                .chain(after.unstaged.iter())
                .all(|e| e.path != Path::new("a.txt")),
            "expected no status entry for deleted path after resolving {:?}; status={after:?}",
            fixture.kind
        );
    }
}

#[test]
fn status_reports_single_conflict_for_modify_delete() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();

    let entries = status
        .unstaged
        .iter()
        .filter(|e| e.path == Path::new("a.txt"))
        .collect::<Vec<_>>();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one status entry for a.txt, got {:#?}",
        status.unstaged
    );
    assert_eq!(entries[0].kind, FileStatusKind::Conflicted);
    assert_eq!(entries[0].conflict, Some(FileConflictKind::DeletedByUs));
}

#[test]
fn status_reports_conflict_kind_for_add_add() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "base.txt", "base\n");
    run_git(repo, &["add", "base.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs_add"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "ours\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_add"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let status = opened.status().unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Conflicted);
    assert_eq!(
        status.unstaged[0].conflict,
        Some(FileConflictKind::BothAdded)
    );
}

#[test]
fn conflict_file_stages_preserve_non_utf8_bytes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let base_bytes = b"\x00base\xff\n".to_vec();
    let ours_bytes = b"\x00ours\xff\n".to_vec();
    let theirs_bytes = b"\x00theirs\xff\n".to_vec();

    write(repo, "bin.dat", &base_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "bin.dat", &theirs_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "bin.dat", &ours_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let stages = opened
        .conflict_file_stages(Path::new("bin.dat"))
        .unwrap()
        .expect("conflict stage data");

    assert_eq!(stages.path, PathBuf::from("bin.dat"));
    assert_eq!(stages.base_bytes.as_deref(), Some(base_bytes.as_slice()));
    assert_eq!(stages.ours_bytes.as_deref(), Some(ours_bytes.as_slice()));
    assert_eq!(
        stages.theirs_bytes.as_deref(),
        Some(theirs_bytes.as_slice())
    );
    assert_eq!(stages.base, None);
    assert_eq!(stages.ours, None);
    assert_eq!(stages.theirs, None);

    let session = opened
        .conflict_session(Path::new("bin.dat"))
        .unwrap()
        .expect("conflict session");
    assert_eq!(session.path, PathBuf::from("bin.dat"));
    assert_eq!(session.strategy, ConflictResolverStrategy::BinarySidePick);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.unsolved_count(), 1);
    assert!(!session.is_fully_resolved());
    assert!(matches!(session.base, ConflictPayload::Binary(_)));
    assert!(matches!(session.ours, ConflictPayload::Binary(_)));
    assert!(matches!(session.theirs, ConflictPayload::Binary(_)));
}

#[test]
fn checkout_conflict_side_resolves_non_utf8_binary_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let base_bytes = b"\x00base\xff\n".to_vec();
    let ours_bytes = b"\x00ours\xff\n".to_vec();
    let theirs_bytes = b"\x00theirs\xff\n".to_vec();

    write(repo, "bin.dat", &base_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "bin.dat", &theirs_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "bin.dat", &ours_bytes);
    run_git(repo, &["add", "bin.dat"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let session = opened
        .conflict_session(Path::new("bin.dat"))
        .unwrap()
        .expect("binary conflict session");
    assert_eq!(session.strategy, ConflictResolverStrategy::BinarySidePick);

    opened
        .checkout_conflict_side(Path::new("bin.dat"), ConflictSide::Theirs)
        .unwrap();

    assert_eq!(fs::read(repo.join("bin.dat")).unwrap(), theirs_bytes);

    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("bin.dat") && e.kind == FileStatusKind::Conflicted),
        "binary conflict should be cleared after choosing theirs"
    );
    assert!(
        status_after
            .staged
            .iter()
            .any(|e| e.path == Path::new("bin.dat")),
        "chosen binary side should be staged"
    );
}

#[test]
fn conflict_session_both_deleted_binary_prefers_decision_strategy() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let base_blob = hash_blob(repo, b"\x00base\xff\n");
    set_unmerged_stages(repo, "gone.bin", Some(base_blob.as_str()), None, None);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("gone.bin"))
        .expect("expected conflict status entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::BothDeleted));

    let session = opened
        .conflict_session(Path::new("gone.bin"))
        .unwrap()
        .expect("conflict session");
    assert_eq!(session.conflict_kind, FileConflictKind::BothDeleted);
    assert_eq!(session.strategy, ConflictResolverStrategy::DecisionOnly);
    assert!(matches!(session.base, ConflictPayload::Binary(_)));
    assert!(session.ours.is_absent());
    assert!(session.theirs.is_absent());
}

#[test]
fn diff_file_text_handles_modify_delete_conflicts() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let diff = opened
        .diff_file_text(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap()
        .expect("file diff for conflicted changes");
    assert_file_diff_text_sources(&diff, None, Some("theirs\n"));
}

#[test]
fn checkout_conflict_side_resolves_modify_delete_using_ours() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Ours)
        .unwrap();

    assert!(
        !repo.join("a.txt").exists(),
        "expected ours resolution to remove file from worktree"
    );
    let status = opened.status().unwrap();
    assert!(
        !status
            .staged
            .iter()
            .chain(status.unstaged.iter())
            .any(|e| e.path == Path::new("a.txt")),
        "expected ours resolution to clear status entries for a.txt, got {status:?}"
    );
}

#[test]
fn checkout_conflict_side_resolves_modify_delete_using_theirs() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours_delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Theirs)
        .unwrap();

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "theirs\n",
        "expected theirs resolution to restore file contents"
    );
    let status = opened.status().unwrap();
    assert!(
        status.unstaged.is_empty(),
        "expected theirs resolution to clear unstaged entries"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Added),
        "expected theirs resolution to stage file as added, got {status:?}"
    );
}

#[test]
fn checkout_conflict_side_stages_resolution() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "theirs\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "ours\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Theirs)
        .unwrap();

    let status = opened.status().unwrap();
    assert!(status.unstaged.iter().all(|s| s.path != Path::new("a.txt")));
    assert!(
        status
            .staged
            .iter()
            .any(|s| s.path == Path::new("a.txt") && s.kind == FileStatusKind::Modified)
    );

    let on_disk = fs::read_to_string(repo.join("a.txt")).unwrap();
    assert_eq!(on_disk, "theirs\n");
}

#[cfg(unix)]
#[test]
fn launch_mergetool_reports_a_symlink_conflict_as_such() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_symlink_conflict(repo, "link", "ours.txt", "theirs.txt");
    assert!(
        fs::symlink_metadata(repo.join("link"))
            .unwrap()
            .is_symlink()
    );

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", "true");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    // Git launches no tool here either, so the refusal must say what to do.
    let err = opened
        .launch_mergetool(Path::new("link"))
        .expect_err("a symlink conflict has no mergetool");
    let ErrorKind::Backend(message) = err.kind() else {
        panic!("expected a backend refusal, got {err:?}");
    };
    assert!(message.contains("symbolic-link conflict"), "{message}");
    assert!(message.contains("local or remote"), "{message}");
}

#[cfg(unix)]
#[test]
fn launch_mergetool_keeps_tool_output_when_the_result_is_a_symlink() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");
    write(repo, "victim.txt", "keep\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        "echo tool-ran; rm -f \"$MERGED\"; ln -s victim.txt \"$MERGED\"",
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened
        .launch_mergetool(Path::new("a.txt"))
        .expect("the tool ran; its output must survive an unsafe readback");
    assert!(!result.success, "{result:?}");
    assert!(result.output.stdout.contains("tool-ran"), "{result:?}");
    assert_eq!(result.output.exit_code, Some(0));
    assert!(result.merged_contents.is_none(), "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.join("victim.txt")).unwrap(),
        "keep\n"
    );
}

#[test]
fn launch_mergetool_trust_exit_false_detects_same_size_content_change() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    // Normalize pre-tool mtime to a fixed timestamp so metadata-only checks
    // cannot detect the edit when the command restores mtime.
    set_fixed_mtime(&repo.join("a.txt"));

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_same_size_content_change_and_exit_failure(),
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(1));

    let on_disk = fs::read(repo.join("a.txt")).unwrap();
    assert!(!on_disk.is_empty());
    assert_eq!(on_disk[0], b'R');
    assert_eq!(result.merged_contents.as_deref(), Some(on_disk.as_slice()));

    let status = opened.status().unwrap();
    assert!(status.unstaged.iter().all(|e| e.path != Path::new("a.txt")));
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Modified),
        "expected staged resolution after content-changing mergetool run, got {status:?}"
    );
}

#[test]
fn launch_mergetool_reflects_config_written_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_copy_remote_to_merged_and_exit_success(),
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "theirs\n");

    let status = opened.status().unwrap();
    assert!(
        status
            .unstaged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt"))
    );
    assert!(
        status
            .staged
            .iter()
            .any(|entry| entry.path == Path::new("a.txt") && entry.kind == FileStatusKind::Modified),
        "expected mergetool resolution after config refresh, got {status:?}"
    );
}

#[test]
fn launch_mergetool_trust_exit_false_requires_content_change() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_exit_success());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(!result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert!(result.merged_contents.is_none());

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt")),
        "unexpected staged resolution when mergetool did not change output: {status:?}"
    );
    let conflict_entry = status
        .unstaged
        .iter()
        .find(|entry| entry.path == Path::new("a.txt"))
        .expect("conflict should remain unresolved");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(
        conflict_entry.conflict,
        Some(FileConflictKind::BothModified)
    );
}

#[test]
fn launch_mergetool_trust_exit_false_detects_deleted_output_change() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_delete_merged_and_exit_failure());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(1));
    assert!(
        result.merged_contents.is_none(),
        "deleted-output resolution should not return merged file bytes"
    );
    assert!(
        !repo.join("a.txt").exists(),
        "mergetool delete output should remove the worktree file"
    );

    let status = opened.status().unwrap();
    assert!(
        status.unstaged.iter().all(|e| e.path != Path::new("a.txt")),
        "expected conflict to clear from unstaged after delete-output mergetool run, got {status:?}"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Deleted),
        "expected delete-output mergetool run to stage file deletion, got {status:?}"
    );
}

#[test]
fn launch_mergetool_rejects_unresolved_marker_output() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_write_unresolved_markers_and_exit_success(),
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .launch_mergetool(Path::new("a.txt"))
        .expect_err("mergetool should fail when merged output still has markers");

    match err.kind() {
        ErrorKind::Backend(msg) => {
            assert!(
                msg.contains("left unresolved conflict markers"),
                "unexpected backend error: {msg}"
            );
            assert!(
                msg.contains("a.txt"),
                "backend error should include conflicted path: {msg}"
            );
        }
        other => panic!("expected backend error, got {other:?}"),
    }

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt")),
        "unexpected staged resolution when mergetool left markers: {status:?}"
    );
    let conflict_entry = status
        .unstaged
        .iter()
        .find(|entry| entry.path == Path::new("a.txt"))
        .expect("conflict should remain unresolved");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(
        conflict_entry.conflict,
        Some(FileConflictKind::BothModified)
    );
}

#[cfg(not(windows))]
#[test]
fn launch_mergetool_custom_cmd_supports_braced_env_variables() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/a space.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        "cat \"${REMOTE}\" > \"${MERGED}\"; exit 0",
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let path = Path::new(conflicted_path);
    let result = opened.launch_mergetool(path).unwrap();
    assert!(
        result.success,
        "expected braced variable expansion to succeed, got {result:?}"
    );
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));

    let on_disk = fs::read_to_string(repo.join(conflicted_path)).unwrap();
    assert_eq!(on_disk, "theirs\n");
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );

    let status = opened.status().unwrap();
    assert!(
        status.unstaged.iter().all(|e| e.path != path),
        "expected conflict to clear after mergetool resolution: {status:?}"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == path && e.kind == FileStatusKind::Modified),
        "expected resolved file to be staged after mergetool run: {status:?}"
    );
}

#[test]
#[cfg(windows)]
fn launch_mergetool_custom_cmd_supports_cmd_percent_env_variables() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/a space.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        "copy /Y \"%REMOTE%\" \"%MERGED%\" > NUL && exit /b 0",
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let path = Path::new(conflicted_path);
    let result = opened.launch_mergetool(path).unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert_eq!(
        fs::read_to_string(repo.join(conflicted_path)).unwrap(),
        "theirs\n"
    );
}

#[test]
fn launch_mergetool_custom_cmd_supports_unicode_conflicted_path() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let conflicted_path = "docs/spaced 日本語 file.txt";
    setup_both_modified_text_conflict(repo, conflicted_path, "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_copy_remote_to_merged_and_exit_success(),
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let path = Path::new(conflicted_path);
    let result = opened.launch_mergetool(path).unwrap();
    assert!(
        result.success,
        "expected unicode conflicted path to resolve, got {result:?}"
    );
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));

    let on_disk = fs::read_to_string(repo.join(conflicted_path)).unwrap();
    assert_eq!(on_disk, "theirs\n");
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );

    let status = opened.status().unwrap();
    assert!(
        status.unstaged.iter().all(|entry| entry.path != path),
        "expected unicode conflict to clear after mergetool resolution: {status:?}"
    );
    assert!(
        status
            .staged
            .iter()
            .any(|entry| entry.path == path && entry.kind == FileStatusKind::Modified),
        "expected resolved unicode path to be staged after mergetool run: {status:?}"
    );
}

#[test]
fn launch_mergetool_prefers_merge_guitool_when_gui_default_true() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "cli"]);
    run_git(repo, &["config", "merge.guitool", "gui"]);
    run_git(repo, &["config", "mergetool.guiDefault", "true"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "cli", cmd_write_cli_to_merged());
    set_repo_local_mergetool_cmd_with_consent(repo, "gui", cmd_write_gui_to_merged());
    run_git(repo, &["config", "mergetool.cli.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.gui.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "gui");
    assert_eq!(result.merged_contents.as_deref(), Some("gui\n".as_bytes()));
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "gui\n");
}

#[cfg(unix)]
#[test]
fn launch_mergetool_uses_tool_path_override_without_custom_cmd() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    let script_path = repo.join("fake-merge-tool.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\n# args: local base remote merged\ncat \"$3\" > \"$4\"\n",
    )
    .unwrap();
    make_executable(&script_path);

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.path",
            git_path_arg(&script_path).as_str(),
        ],
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    // Both the tool name and its path come from `.git/config`, so the launch
    // needs the same consent a repository-local `.cmd` does.
    allow_repo_local_mergetool_cmd(repo, "fake");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes())
    );
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "theirs\n");
}

#[cfg(unix)]
#[test]
fn launch_mergetool_refuses_repo_local_tool_path_without_consent() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    // A hostile `.git/config` can point `mergetool.<tool>.path` at any file in
    // the checkout. The script drops a marker so the test proves it never ran,
    // not merely that the launch reported an error. `codecompare` is a git
    // built-in, so only the path is repository-controlled here, and its real
    // program (`CodeMerge`) exists only on Windows: the backend reads the
    // developer's own global git config, so a common tool such as `kdiff3`
    // could fall back to a trusted path and launch a real GUI mid-test.
    let script_path = repo.join("repo-controlled-tool.sh");
    let marker_path = repo.join("tool-ran");
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\n: > \"{}\"\ncat \"$3\" > \"$4\"\n",
            marker_path.display()
        ),
    )
    .unwrap();
    make_executable(&script_path);

    run_git(repo, &["config", "merge.tool", "codecompare"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.codecompare.path",
            git_path_arg(&script_path).as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let message = opened
        .launch_mergetool(Path::new("a.txt"))
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("Refusing to use repository-local mergetool.codecompare.path"),
        "{message}"
    );
    assert!(
        !marker_path.exists(),
        "repository-local mergetool.<tool>.path executed without consent"
    );
    let conflicted = fs::read_to_string(repo.join("a.txt")).unwrap();
    assert!(
        conflicted.contains("<<<<<<<"),
        "conflict must stay unresolved: {conflicted:?}"
    );
}

#[cfg(unix)]
#[test]
fn launch_mergetool_builtin_tool_gets_merge_mode_arguments() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    // Stand-in for kdiff3: like the real tool it only merges when an output
    // file is named with `-o`, and otherwise just shows a read-only 3-way diff.
    let script_path = repo.join("fake-kdiff3.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\n\
         : > \"$PWD/kdiff3-args\"\n\
         output=\n\
         prev=\n\
         for arg in \"$@\"; do\n\
         \tprintf '%s\\n' \"$arg\" >> \"$PWD/kdiff3-args\"\n\
         \tif [ \"$prev\" = \"-o\" ]; then output=$arg; fi\n\
         \tprev=$arg\n\
         done\n\
         [ -n \"$output\" ] || exit 1\n\
         printf 'merged\\n' > \"$output\"\n",
    )
    .unwrap();
    make_executable(&script_path);

    run_git(repo, &["config", "merge.tool", "kdiff3"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.kdiff3.path",
            git_path_arg(&script_path).as_str(),
        ],
    );
    run_git(repo, &["config", "mergetool.kdiff3.trustExitCode", "true"]);
    // `mergetool.kdiff3.path` in `.git/config` is repository-controlled.
    allow_repo_local_mergetool_cmd(repo, "kdiff3");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();

    assert!(
        result.success,
        "kdiff3 should be launched in merge mode: {:?}",
        result.output
    );
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("merged\n".as_bytes())
    );
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "merged\n");

    let args: Vec<String> = fs::read_to_string(repo.join("kdiff3-args"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(args.iter().any(|arg| arg == "--auto"), "{args:?}");

    let output_index = args
        .iter()
        .position(|arg| arg == "-o")
        .expect("merge output flag should be passed");
    let output_path = Path::new(&args[output_index + 1]);
    assert!(output_path.is_absolute(), "{args:?}");
    assert_eq!(output_path.file_name().unwrap(), "a.txt");

    // git's kdiff3 recipe ends with BASE, LOCAL, REMOTE in that order.
    let tail = &args[args.len() - 3..];
    assert!(tail[0].contains("_BASE_"), "{args:?}");
    assert!(tail[1].contains("_LOCAL_"), "{args:?}");
    assert!(tail[2].contains("_REMOTE_"), "{args:?}");

    let label_index = args
        .iter()
        .position(|arg| arg == "--L1")
        .expect("window labels should be passed");
    assert_eq!(args[label_index + 1], "a.txt (Base)", "{args:?}");
}

#[test]
fn launch_mergetool_rejects_builtin_tool_that_cannot_merge() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "kompare"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened.launch_mergetool(Path::new("a.txt")).unwrap_err();

    assert!(
        format!("{err}").contains("cannot merge"),
        "expected a clear diff-only tool error, got {err}"
    );
    assert!(
        fs::read_to_string(repo.join("a.txt"))
            .unwrap()
            .contains("<<<<<<<"),
        "the conflicted file should be left untouched"
    );
}

#[cfg(unix)]
#[test]
fn launch_mergetool_prefers_custom_cmd_over_tool_path_override() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    let script_path = repo.join("fake-merge-tool.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\nprintf 'path\\n' > \"$4\"\ntouch \"$PWD/path_invoked\"\n",
    )
    .unwrap();
    make_executable(&script_path);

    run_git(repo, &["config", "merge.tool", "fake"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.fake.path",
            git_path_arg(&script_path).as_str(),
        ],
    );
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_write_cmd_to_merged());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);
    assert_eq!(result.tool_name, "fake");
    assert_eq!(result.output.exit_code, Some(0));
    assert_eq!(result.merged_contents.as_deref(), Some("cmd\n".as_bytes()));
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "cmd\n");
    assert!(
        !repo.join("path_invoked").exists(),
        "tool path executable should not run when mergetool.<tool>.cmd is configured"
    );
}

#[test]
fn launch_mergetool_write_to_temp_true_uses_temp_stage_paths() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success);

    let vars = read_stage_env_vars(&repo.join("a.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let var_path = Path::new(&var);
        let normalized_var = normalize_stage_var(&var);
        assert!(
            var_path.is_absolute(),
            "writeToTemp=true should pass absolute temp paths, got {var}"
        );
        assert!(
            normalized_var.contains("gitcomet-mergetool-"),
            "expected temporary mergetool prefix in path, got {var}"
        );
        assert!(
            !normalized_var.starts_with("./"),
            "writeToTemp=true should not use workdir-prefixed paths: {var}"
        );
        assert!(
            !var_path.exists(),
            "writeToTemp=true with default keepTemporaries=false should cleanup stage files: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_false_uses_workdir_prefixed_stage_paths() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("docs/note.txt")).unwrap();
    assert!(result.success, "{result:?}");
    // On Windows the filesystem form uses backslashes, but gix index keys
    // retain slashes. The REMOTE bytes prove those two path forms stayed apart.
    assert_eq!(
        result.merged_contents.as_deref(),
        Some("theirs\n".as_bytes()),
        "the nested conflict's REMOTE stage must reach the mergetool"
    );
    assert_eq!(
        fs::read(repo.join("docs/note.txt")).unwrap(),
        b"theirs\n",
        "the mergetool must not stage an empty resolution for a nested path"
    );

    let vars = read_stage_env_vars(&repo.join("docs/note.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let normalized_var = normalize_stage_var(&var);
        assert!(
            normalized_var.starts_with("./docs/note_"),
            "writeToTemp=false should use './' prefixed workdir paths, got {var}"
        );
        assert!(
            normalized_var.contains("_BASE_")
                || normalized_var.contains("_LOCAL_")
                || normalized_var.contains("_REMOTE_"),
            "unexpected stage-file naming: {var}"
        );
        let fs_path = stage_var_to_fs_path(repo, &var);
        assert!(
            !fs_path.exists(),
            "writeToTemp=false with default keepTemporaries=false should cleanup stage files: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_false_keep_temporaries_preserves_stage_files() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("docs/note.txt")).unwrap();
    assert!(result.success, "{result:?}");

    let vars = read_stage_env_vars(&repo.join("docs/note.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let normalized_var = normalize_stage_var(&var);
        assert!(
            normalized_var.starts_with("./docs/note_"),
            "writeToTemp=false should use './' prefixed workdir paths, got {var}"
        );
        let fs_path = stage_var_to_fs_path(repo, &var);
        assert!(
            fs_path.exists(),
            "keepTemporaries=true should keep stage file in workdir mode: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_false_keep_temporaries_preserves_stage_files_on_abort() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "docs/note.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_dump_stage_paths_and_exit_failure(),
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("docs/note.txt")).unwrap();
    assert!(
        !result.success,
        "tool exit failure should be reported as unresolved"
    );

    let vars = read_stage_env_vars(&repo.join("docs/note.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");
    for var in vars {
        let normalized_var = normalize_stage_var(&var);
        assert!(
            normalized_var.starts_with("./docs/note_"),
            "writeToTemp=false should use './' prefixed workdir paths, got {var}"
        );
        let fs_path = stage_var_to_fs_path(repo, &var);
        assert!(
            fs_path.exists(),
            "keepTemporaries=true should keep stage file on abort in workdir mode: {var}"
        );
    }
}

#[test]
fn launch_mergetool_write_to_temp_true_keep_temporaries_preserves_stage_files() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_stage_paths_and_copy_remote());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "true"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(result.success, "{result:?}");

    let vars = read_stage_env_vars(&repo.join("a.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");

    let mut temp_dirs: Vec<PathBuf> = Vec::new();
    for var in vars {
        let var_path = Path::new(&var);
        let normalized_var = normalize_stage_var(&var);
        assert!(
            var_path.is_absolute(),
            "writeToTemp=true should pass absolute temp paths, got {var}"
        );
        assert!(
            normalized_var.contains("gitcomet-mergetool-"),
            "expected temporary mergetool prefix in path, got {var}"
        );
        assert!(
            var_path.exists(),
            "keepTemporaries=true should keep stage file in temp mode: {var}"
        );
        if let Some(parent) = var_path.parent()
            && !temp_dirs.iter().any(|dir| dir == parent)
        {
            temp_dirs.push(parent.to_path_buf());
        }
    }

    // Keep test environment clean even though behavior keeps temp files.
    for dir in temp_dirs {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn launch_mergetool_write_to_temp_true_keep_temporaries_preserves_stage_files_on_abort() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_modified_text_conflict(repo, "a.txt", "ours\n", "theirs\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(
        repo,
        "fake",
        cmd_dump_stage_paths_and_exit_failure(),
    );
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "true"]);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("a.txt")).unwrap();
    assert!(
        !result.success,
        "tool exit failure should be reported as unresolved"
    );

    let vars = read_stage_env_vars(&repo.join("a.txt.env"));
    assert_eq!(vars.len(), 3, "expected BASE/LOCAL/REMOTE dump");

    let mut temp_dirs: Vec<PathBuf> = Vec::new();
    for var in vars {
        let var_path = Path::new(&var);
        let normalized_var = normalize_stage_var(&var);
        assert!(
            var_path.is_absolute(),
            "writeToTemp=true should pass absolute temp paths, got {var}"
        );
        assert!(
            normalized_var.contains("gitcomet-mergetool-"),
            "expected temporary mergetool prefix in path, got {var}"
        );
        assert!(
            var_path.exists(),
            "keepTemporaries=true should keep stage file on abort in temp mode: {var}"
        );
        if let Some(parent) = var_path.parent()
            && !temp_dirs.iter().any(|dir| dir == parent)
        {
            temp_dirs.push(parent.to_path_buf());
        }
    }

    // Keep test environment clean even though behavior keeps temp files.
    for dir in temp_dirs {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn launch_mergetool_no_base_conflict_passes_empty_base_file() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_added_text_conflict(repo, "new.txt", "ours added\n", "theirs added\n");

    run_git(repo, &["config", "merge.tool", "fake"]);
    set_repo_local_mergetool_cmd_with_consent(repo, "fake", cmd_dump_base_size_and_copy_remote());
    run_git(repo, &["config", "mergetool.fake.trustExitCode", "true"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let result = opened.launch_mergetool(Path::new("new.txt")).unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(
        fs::read_to_string(repo.join("new.txt.base-size")).unwrap(),
        "0",
        "BASE should be an empty file for both-added/no-base conflicts"
    );
    assert_eq!(
        fs::read_to_string(repo.join("new.txt")).unwrap(),
        "theirs added\n"
    );
}
