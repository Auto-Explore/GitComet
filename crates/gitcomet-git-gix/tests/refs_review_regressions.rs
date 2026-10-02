use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{GitBackend, GitRepository, SequencerState};
use gitcomet_git_gix::GixBackend;
use std::{fs, path::Path};

#[path = "support/extension_formats.rs"]
#[allow(dead_code)]
mod formats;
use formats::{fixture, git};

fn commit_file(path: &Path) -> String {
    fs::write(path.join("file.txt"), "root\n").unwrap();
    git(path, &["add", "file.txt"]);
    git(path, &["commit", "-m", "root"]);
    git(path, &["rev-parse", "HEAD"])
}

#[test]
fn sha256_files_follow_symbolic_revisions_and_create_branches() {
    let dir = fixture("sha256", "files");
    let tip = commit_file(dir.path());
    git(
        dir.path(),
        &["update-ref", "refs/remotes/origin/main", &tip],
    );
    git(
        dir.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    git(
        dir.path(),
        &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
    );
    git(
        dir.path(),
        &["symbolic-ref", "refs/heads/chain", "refs/heads/alias"],
    );
    for packed in [false, true] {
        if packed {
            git(dir.path(), &["gc", "-q"]);
        }
        let repo = GixBackend.open(dir.path()).unwrap();
        for (index, spec) in ["origin/HEAD", "alias", "chain"].into_iter().enumerate() {
            assert_eq!(git(dir.path(), &["rev-parse", spec]), tip);
            let revision = CommitId(spec.into());
            assert_eq!(
                repo.resolve_commit(&revision).unwrap().id.as_ref(),
                tip,
                "packed={packed} {spec}"
            );
            let branch = format!("from-symbolic-{packed}-{index}");
            repo.create_branch(&branch, &revision).unwrap();
            assert_eq!(git(dir.path(), &["rev-parse", &branch]), tip);
        }
    }
}

fn assert_tab_width(repo: &dyn GitRepository, path: &Path, columns: u8) {
    assert_eq!(
        git(path, &["config", "get", "core.whitespace"]),
        format!("tabwidth={columns}")
    );
    assert_eq!(
        repo.text_attributes(Path::new("file.txt"))
            .unwrap()
            .tab_width
            .map(|width| width.columns),
        Some(columns),
        "{}",
        path.display()
    );
}

fn onbranch_configuration(hash: &str) {
    if !formats::reftable_available() {
        return;
    }
    let dir = fixture(hash, "reftable");
    let config_path = dir.path().join(".git/config");
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str(
        "\n[core]\nwhitespace = tabwidth=4\n\
         [includeIf \"onbranch:main\"]\npath = main-config\n\
         [includeIf \"onbranch:topic\"]\npath = topic-config\n",
    );
    fs::write(config_path, config).unwrap();
    let main_config = dir.path().join(".git/main-config");
    fs::write(&main_config, "[core]\nwhitespace = tabwidth=8\n").unwrap();
    fs::write(
        dir.path().join(".git/topic-config"),
        "[core]\nwhitespace = tabwidth=3\n",
    )
    .unwrap();

    // Native HEAD matters even before the branch has its first commit.
    let repo = GixBackend.open(dir.path()).unwrap();
    assert_tab_width(repo.as_ref(), dir.path(), 8);
    let tip = commit_file(dir.path());
    assert_tab_width(repo.as_ref(), dir.path(), 8);
    git(dir.path(), &["checkout", "-b", "topic"]);
    assert_tab_width(repo.as_ref(), dir.path(), 3);
    git(dir.path(), &["checkout", "--detach", &tip]);
    assert_tab_width(repo.as_ref(), dir.path(), 4);
    git(dir.path(), &["checkout", "main"]);
    assert_tab_width(repo.as_ref(), dir.path(), 8);
    fs::write(&main_config, "[core]\nwhitespace = tabwidth=6\n").unwrap();
    assert_tab_width(repo.as_ref(), dir.path(), 6);

    // Linked worktrees must evaluate includes against their private HEAD.
    let linked_parent = tempfile::tempdir().unwrap();
    let linked = linked_parent.path().join("linked");
    git(
        dir.path(),
        &["worktree", "add", linked.to_str().unwrap(), "topic"],
    );
    let linked_repo = GixBackend.open(&linked).unwrap();
    assert_tab_width(linked_repo.as_ref(), &linked, 3);
    git(&linked, &["checkout", "--detach", &tip]);
    assert_tab_width(linked_repo.as_ref(), &linked, 4);
    assert_tab_width(repo.as_ref(), dir.path(), 6);
}

#[test]
fn reftable_sha1_onbranch_configuration_uses_native_head() {
    onbranch_configuration("sha1");
}

#[test]
fn reftable_sha256_onbranch_configuration_uses_native_head() {
    onbranch_configuration("sha256");
}

#[test]
fn packed_files_branch_listing_does_not_read_direct_target_objects() {
    for hash in ["sha1", "sha256"] {
        let dir = fixture(hash, "files");
        let tip = commit_file(dir.path());
        git(dir.path(), &["branch", "first"]);
        git(dir.path(), &["branch", "second"]);
        git(dir.path(), &["gc", "-q"]);
        assert!(dir.path().join(".git/packed-refs").is_file());

        // Removing the packs makes unnecessary object reads fail deterministically;
        // branch names and direct target IDs are all in the reference store.
        fs::remove_dir_all(dir.path().join(".git/objects/pack")).unwrap();
        let repo = GixBackend.open(dir.path()).unwrap();
        let branches = repo.list_branches().unwrap();
        assert_eq!(
            branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "main", "second"]
        );
        assert!(branches.iter().all(|branch| branch.target.as_ref() == tip));
    }
}

fn ambiguous_prefix_type_hints(hash: &str, refs: &str) {
    if refs == "reftable" && !formats::reftable_available() {
        return;
    }
    let dir = fixture(hash, refs);
    let tip = commit_file(dir.path());
    let commit_id = gix::ObjectId::from_hex(tip.as_bytes()).unwrap();
    let prefix = &tip[..4];
    // Find a real blob sharing a commit's prefix, storing only the matching blob.
    let (contents, blob_id) = (0..2_000_000)
        .find_map(|n| {
            let contents = format!("prefix collision {n}\n");
            let id = gix::objs::compute_hash(
                commit_id.kind(),
                gix::objs::Kind::Blob,
                contents.as_bytes(),
            )
            .unwrap();
            (id.as_bytes()[..2] == commit_id.as_bytes()[..2]).then_some((contents, id))
        })
        .expect("find a four-digit commit/blob collision");
    let blob_file = tempfile::NamedTempFile::new().unwrap();
    fs::write(blob_file.path(), contents).unwrap();
    assert_eq!(
        git(
            dir.path(),
            &["hash-object", "-w", blob_file.path().to_str().unwrap()]
        ),
        blob_id.to_string()
    );
    let candidates = git(
        dir.path(),
        &["rev-parse", &format!("--disambiguate={prefix}")],
    );
    assert_eq!(candidates.lines().count(), 2, "{candidates}");

    for packed in [false, true] {
        if packed {
            git(dir.path(), &["repack", "-a", "-d", "--keep-unreachable"]);
        }
        let repo = GixBackend.open(dir.path()).unwrap();
        let error = repo.resolve_commit(&CommitId(prefix.into())).unwrap_err();
        assert!(
            error.to_string().to_lowercase().contains("ambiguous"),
            "{error}"
        );
        for suffix in ["^{commit}", "~0", "^0"] {
            let spec = format!("{prefix}{suffix}");
            assert_eq!(git(dir.path(), &["rev-parse", "--verify", &spec]), tip);
            assert_eq!(
                repo.resolve_commit(&CommitId(spec.clone().into()))
                    .unwrap()
                    .id
                    .as_ref(),
                tip,
                "{hash}/{refs} packed={packed} {spec}"
            );
        }
        let branch = format!("from-prefix-{packed}");
        repo.create_branch(&branch, &CommitId(format!("{prefix}^{{commit}}").into()))
            .unwrap();
        assert_eq!(git(dir.path(), &["rev-parse", &branch]), tip);
    }
}

#[test]
fn sha256_files_disambiguate_prefixes_using_revision_type_hints() {
    ambiguous_prefix_type_hints("sha256", "files");
}

#[test]
fn reftable_sha1_disambiguates_prefixes_using_revision_type_hints() {
    ambiguous_prefix_type_hints("sha1", "reftable");
}

#[test]
fn reftable_sha256_disambiguates_prefixes_using_revision_type_hints() {
    ambiguous_prefix_type_hints("sha256", "reftable");
}

#[test]
fn files_operation_state_does_not_dwim_root_refs_to_branches() {
    for hash in ["sha1", "sha256"] {
        for name in ["REVERT_HEAD", "CHERRY_PICK_HEAD"] {
            let dir = fixture(hash, "files");
            commit_file(dir.path());
            git(dir.path(), &["branch", name]);
            git(dir.path(), &["bisect", "start"]);
            assert!(!dir.path().join(".git").join(name).exists());
            let repo = GixBackend.open(dir.path()).unwrap();
            assert_eq!(
                repo.sequencer_state().unwrap(),
                SequencerState::None,
                "{hash} refs/heads/{name} is an ordinary branch during bisect"
            );
        }
    }
}
